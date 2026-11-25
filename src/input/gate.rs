//! Which controller input reaches Lodestar, decided once per frame from the window's focus,
//! whether a game runs and the settings (pure; the regression tests below pin the rules).
//!
//! THE SAFETY RULE (not configurable): while a game runs and Lodestar isn't the focused window,
//! the only pad input that gets through is the menu button (`Nav::Guide`: the Home / PS button
//! and/or the chosen button combination, held for a moment — input/pad.rs), and all it may do
//! is bring Lodestar's in-game menu up. The combination's own buttons (L3, R3, Select, Start …)
//! never get through by themselves, and a quick press of the combination doesn't count.
//! Presses meant for the game never act on Lodestar. Once Lodestar is focused while a game
//! runs, only the Playing overlay's buttons (Accept / Back / Menu / Guide) get through, unless
//! the user opened Lodestar's own in-game menu or library (then Lodestar is what they are
//! using).
//!
//! Without a game, `background_input` (default on) lets the controller drive Lodestar even when
//! another window is active; off ("Only use the controller when Lodestar is the active window")
//! keeps it to the focused window, the Guide button still summoning Lodestar. Background input
//! only acts while a recent look for running games (every couple of seconds while unfocused,
//! running_app.rs) found nothing that looks like a game Lodestar doesn't follow — a game started
//! from Steam, a desktop shortcut or another launcher while Lodestar sat in the background. No
//! look yet, a stale one, or one that saw a game: only Guide (fail safe). A window hidden to the
//! tray or minimised never acts on background input either (nothing visible would react).
//!
//! Focus nobody reported yet (egui's `focused == None`): while a game runs (or one was seen
//! running), unfocused (never guess in the game's favour); otherwise focused until the first
//! focus event arrived, and unfocused after one (a report that went missing again).
//!
//! DESKTOP MODE ([`desktop_pad`]) is stricter still, because many games read the pad even while
//! they aren't focused: a press that moved around the library would act in the game too. The pad
//! drives desktop mode only while a Lodestar window is reported focused (an unknown focus never
//! counts), the window is visible, no game is up or on its way or just ended, a look for running
//! games has finished since the last game ended and saw nothing, and the in-game menu isn't up.
//! `background_input` doesn't apply there. It narrows what [`filter`] lets through, never widens
//! it.
//!
//! Scripted screenshot runs never act on a real pad.

use super::Nav;

/// What the gamepad thread lets through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    /// Nothing (held buttons are forgotten).
    Off,
    /// Only Guide presses.
    GuideOnly,
    /// Everything (the frame's filter may still narrow it, see [`filter`]).
    All,
}

impl Gate {
    pub(crate) fn to_u8(self) -> u8 {
        match self {
            Gate::Off => 0,
            Gate::GuideOnly => 1,
            Gate::All => 2,
        }
    }
    pub(crate) fn from_u8(v: u8) -> Gate {
        match v {
            0 => Gate::Off,
            1 => Gate::GuideOnly,
            _ => Gate::All,
        }
    }
}

/// One frame's situation.
#[derive(Clone, Copy, Debug, Default)]
pub struct Situation {
    /// `--screenshot` run.
    pub scripted: bool,
    /// egui's `viewport().focused` (None: not reported).
    pub focused: Option<bool>,
    /// A focus event was seen this run.
    pub focus_seen: bool,
    /// A game Lodestar launched or follows is actually up.
    pub game_up: bool,
    /// `[controller] background_input`.
    pub background_input: bool,
    /// A recent look for running games (fresh, see running_app.rs) found nothing that looks like
    /// a game Lodestar doesn't follow.
    pub others_clear: bool,
    /// The last look for running games saw something that looks like a game.
    pub others_seen: bool,
    /// The window is hidden to the tray or minimised.
    pub hidden: bool,
    /// `[controller] guide_opens_lodestar`.
    pub guide_summons: bool,
    /// Lodestar's in-game menu / library (or a dialog of Lodestar's own) is open: the user asked
    /// for Lodestar while playing.
    pub lodestar_menu: bool,
}

impl Situation {
    /// The controller may drive Lodestar although it isn't focused: the setting is on, the window
    /// can be seen, and a fresh look found no game running elsewhere.
    pub fn background(&self) -> bool {
        self.background_input && !self.hidden && self.others_clear && !self.game_up
    }

    /// Looking for games started elsewhere is what lets background input act (see
    /// [`Situation::background`]): wanted while the controller would drive an unfocused,
    /// visible Lodestar without a game.
    pub fn wants_background_look(&self) -> bool {
        !self.scripted && self.background_input && !self.hidden && !self.game_up && !self.is_focused()
    }

    /// The window counts as focused (see the module docs for an unknown focus).
    pub fn is_focused(&self) -> bool {
        match self.focused {
            Some(f) => f,
            None => !self.game_up && !self.others_seen && !self.focus_seen,
        }
    }
}

/// The focus to decide with: a window hidden to the tray or minimised is never focused, whatever
/// the last report said (a report can be stale while the window is hidden, and a stale "focused"
/// while a game runs would let the game's presses reach Lodestar).
pub fn effective_focus(reported: Option<bool>, hidden: bool, minimised: Option<bool>) -> Option<bool> {
    if hidden || minimised == Some(true) { Some(false) } else { reported }
}

/// One of Lodestar's other windows (a dialog in its own window, ui/desktop/own_window.rs) as the
/// main window's pass sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OtherWindow {
    /// Its focus as eframe hands it to the main window's pass. eframe only refreshes it while
    /// drawing that window, which happens later in the main window's pass: it can be one pass
    /// old (still "focused" in the pass that the window losing the focus to a game woke up).
    pub reported: Option<bool>,
    pub minimised: Option<bool>,
    /// What the window's own latest pass saw (focused and not minimised), when it has been drawn.
    pub latest: Option<bool>,
}

impl OtherWindow {
    /// Focused only when every report says so: a stale "focused" never counts once the window's
    /// own pass saw otherwise, a minimised window never does.
    pub fn focused(&self) -> bool {
        self.reported == Some(true) && self.minimised != Some(true) && self.latest != Some(false)
    }
}

/// Lodestar's focus over all its windows: the main window's (as [`effective_focus`] decides) or
/// any of its own windows' (Settings, Edit game details … opened in their own window, see
/// [`OtherWindow::focused`]) — focused as soon as any of them is, so a focused Settings window
/// is Lodestar being used, exactly like the main window. Hidden to the tray: never focused
/// (those windows aren't shown then; a stale report must not count). A minimised main window
/// with a focused Settings window beside it is focused.
pub fn lodestar_focus(
    main: Option<bool>,
    hidden: bool,
    minimised: Option<bool>,
    others: impl IntoIterator<Item = OtherWindow>,
) -> Option<bool> {
    if hidden {
        return Some(false);
    }
    if others.into_iter().any(|w| w.focused()) {
        return Some(true);
    }
    effective_focus(main, false, minimised)
}

/// What the gamepad thread may pass on.
pub fn gate(s: &Situation) -> Gate {
    if s.scripted {
        return Gate::Off;
    }
    let focused = s.is_focused();
    let fallback = if s.guide_summons { Gate::GuideOnly } else { Gate::Off };
    if s.game_up {
        if focused { Gate::All } else { fallback }
    } else if focused || s.background() {
        Gate::All
    } else {
        fallback
    }
}

/// The overlay's buttons: what reaches a focused Lodestar while a game runs.
pub fn overlay_button(n: Nav) -> bool {
    matches!(n, Nav::Accept | Nav::Back | Nav::Menu | Nav::Guide)
}

/// Narrow this frame's navigation (pad + keyboard) to what the situation allows.
pub fn filter(s: &Situation, mut navs: Vec<Nav>) -> Vec<Nav> {
    match gate(s) {
        Gate::Off => navs.clear(),
        Gate::GuideOnly => navs.retain(|n| *n == Nav::Guide),
        Gate::All if s.game_up && !s.lodestar_menu => navs.retain(|n| overlay_button(*n)),
        Gate::All => {}
    }
    navs
}

/// Desktop mode's own state next to the [`Situation`] (see [`desktop_pad`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct DesktopPad {
    /// Desktop mode, with Settings → Controller "Use the controller in desktop mode" on.
    pub enabled: bool,
    /// A launch is on its way (not started yet): a Steam launch waiting on an update dialog may
    /// already read the pad.
    pub launching: bool,
    /// A game ended a moment ago.
    pub settling: bool,
    /// A look for running games has finished since the last game ended ([`looked_since`]).
    pub looked: bool,
    /// The in-game menu or the menu button screens are up (they take the pad).
    pub overlay: bool,
}

/// The pad moves around desktop mode: only a reported-focused, visible Lodestar with nothing
/// running and nothing seen running (see the module docs).
pub fn desktop_pad(s: &Situation, d: DesktopPad) -> bool {
    d.enabled
        && !s.scripted
        && s.focused == Some(true)
        && !s.hidden
        && !s.game_up
        && d.looked
        && !s.others_seen
        && !d.launching
        && !d.settling
        && !d.overlay
}

/// The last look for running games (started at `look`) counts for [`DesktopPad::looked`]: it
/// started after the last game this run ended (`ended`), so a look from before that game never
/// stands for the time after it.
pub fn looked_since(look: Option<std::time::Instant>, ended: Option<std::time::Instant>) -> bool {
    look.is_some_and(|at| ended.is_none_or(|e| at >= e))
}

/// Desktop mode wants the look for running games while Lodestar is in the background (so a game
/// left running behind it is known once it comes to the front), until there is one since the last
/// game ended, and while the last one saw a game (so the pad comes back once that game is gone).
pub fn desktop_look(s: &Situation, d: DesktopPad) -> bool {
    d.enabled && !s.scripted && !s.hidden && !s.game_up && (s.focused != Some(true) || !d.looked || s.others_seen)
}

/// Guide presses closer together than this count once (a bouncy button, Steam's virtual pad
/// echoing the real one).
pub const GUIDE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(450);

/// Debounce for Guide presses.
#[derive(Debug, Default)]
pub struct GuideDebounce {
    last: Option<std::time::Instant>,
}

impl GuideDebounce {
    /// True when a press at `now` counts.
    pub fn press(&mut self, now: std::time::Instant) -> bool {
        let ok = self.last.is_none_or(|t| now.saturating_duration_since(t) >= GUIDE_DEBOUNCE);
        if ok {
            self.last = Some(now);
        }
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Nav; 15] = [
        Nav::Up,
        Nav::Down,
        Nav::Left,
        Nav::Right,
        Nav::Accept,
        Nav::Back,
        Nav::Menu,
        Nav::Select,
        Nav::Favorite,
        Nav::Search,
        Nav::PageUp,
        Nav::PageDown,
        Nav::JumpUp,
        Nav::JumpDown,
        Nav::Guide,
    ];

    fn s() -> Situation {
        Situation {
            scripted: false,
            focused: Some(true),
            focus_seen: true,
            game_up: false,
            background_input: true,
            others_clear: true,
            others_seen: false,
            hidden: false,
            guide_summons: true,
            lodestar_menu: false,
        }
    }

    /// REGRESSION (critical, user-requested): background input on + a game running + Lodestar
    /// not focused → nothing but Guide, whatever else is on, and Guide only when it summons.
    #[test]
    fn game_running_unfocused_lets_only_guide_through() {
        for lodestar_menu in [false, true] {
            for focused in [Some(false), None] {
                for focus_seen in [false, true] {
                    let st = Situation { game_up: true, focused, focus_seen, lodestar_menu, ..s() };
                    assert_eq!(gate(&st), Gate::GuideOnly, "{st:?}");
                    assert_eq!(filter(&st, ALL.to_vec()), vec![Nav::Guide], "{st:?}");
                    let off = Situation { guide_summons: false, ..st };
                    assert_eq!(gate(&off), Gate::Off);
                    assert!(filter(&off, ALL.to_vec()).is_empty(), "{off:?}");
                }
            }
        }
    }

    /// REGRESSION (critical): a game running + Lodestar focused → only the Playing overlay's
    /// buttons (Accept / Back / Menu, and Guide for the in-game menu).
    #[test]
    fn game_running_focused_lets_only_overlay_buttons_through() {
        let st = Situation { game_up: true, ..s() };
        assert_eq!(filter(&st, ALL.to_vec()), vec![Nav::Accept, Nav::Back, Nav::Menu, Nav::Guide]);
        let st = Situation { background_input: false, ..st };
        assert_eq!(filter(&st, ALL.to_vec()), vec![Nav::Accept, Nav::Back, Nav::Menu, Nav::Guide]);
        // The in-game menu the user opened (Lodestar focused): Lodestar is what they use.
        let st = Situation { lodestar_menu: true, ..st };
        assert_eq!(filter(&st, ALL.to_vec()), ALL.to_vec());
    }

    #[test]
    fn no_game_background_input() {
        // Default: the controller drives Lodestar in the background too.
        let st = Situation { focused: Some(false), ..s() };
        assert_eq!(gate(&st), Gate::All);
        assert_eq!(filter(&st, ALL.to_vec()), ALL.to_vec());
        // "Only when Lodestar is the active window": Guide still summons it.
        let st = Situation { background_input: false, ..st };
        assert_eq!(filter(&st, ALL.to_vec()), vec![Nav::Guide]);
        let st = Situation { guide_summons: false, ..st };
        assert!(filter(&st, ALL.to_vec()).is_empty());
        // Focused: everything.
        let st = Situation { focused: Some(true), ..st };
        assert_eq!(filter(&st, ALL.to_vec()), ALL.to_vec());
    }

    #[test]
    fn unknown_focus() {
        // Before any focus event, without a game: focused (a window that just opened).
        let st = Situation { focused: None, focus_seen: false, background_input: false, ..s() };
        assert!(st.is_focused());
        assert_eq!(gate(&st), Gate::All);
        // After focus events were seen, an unknown focus is not focus.
        let st = Situation { focus_seen: true, ..st };
        assert!(!st.is_focused());
        assert_eq!(filter(&st, ALL.to_vec()), vec![Nav::Guide]);
        // With a game, unknown is never focus.
        let st = Situation { focus_seen: false, game_up: true, ..st };
        assert!(!st.is_focused());
    }

    /// REGRESSION: hidden to the tray / minimised counts as unfocused even when the last focus
    /// report said "focused", so a game running then only lets the Guide button through.
    #[test]
    fn hidden_or_minimised_is_never_focused() {
        assert_eq!(effective_focus(Some(true), true, None), Some(false));
        assert_eq!(effective_focus(Some(true), false, Some(true)), Some(false));
        assert_eq!(effective_focus(None, true, None), Some(false));
        assert_eq!(effective_focus(Some(true), false, Some(false)), Some(true));
        assert_eq!(effective_focus(None, false, None), None);
        let st = Situation { game_up: true, focused: effective_focus(Some(true), true, None), ..s() };
        assert_eq!(filter(&st, ALL.to_vec()), vec![Nav::Guide]);
    }

    /// REGRESSION (critical): Lodestar counts as focused when ANY of its windows has the focus —
    /// a focused Settings window (its own OS window) is Lodestar being used, not "another window
    /// in front"; Lodestar's windows all unfocused is unfocused; hidden to the tray never is.
    #[test]
    fn settings_window_focused_counts_as_lodestar_focused() {
        let w = |f: Option<bool>| OtherWindow { reported: f, minimised: Some(false), latest: None };
        // Main window unfocused, Settings focused: focused, with or without a game.
        let f = lodestar_focus(Some(false), false, Some(false), [w(Some(true))]);
        assert_eq!(f, Some(true));
        let st = Situation { focused: f, game_up: true, ..s() };
        assert_eq!(filter(&st, ALL.to_vec()), vec![Nav::Accept, Nav::Back, Nav::Menu, Nav::Guide]);
        let st = Situation { focused: f, background_input: false, ..s() };
        assert_eq!(filter(&st, ALL.to_vec()), ALL.to_vec());
        // Main window minimised beside a focused Settings window: still focused.
        assert_eq!(lodestar_focus(Some(false), false, Some(true), [w(Some(true))]), Some(true));
        // Every window of Lodestar unfocused (a game in front): unfocused → only Guide.
        let f = lodestar_focus(Some(false), false, Some(false), [w(Some(false)), w(None)]);
        assert_eq!(f, Some(false));
        let st = Situation { focused: f, game_up: true, ..s() };
        assert_eq!(filter(&st, ALL.to_vec()), vec![Nav::Guide]);
        // ... and the other way round: the main window focused, Settings not.
        assert_eq!(lodestar_focus(Some(true), false, Some(false), [w(Some(false))]), Some(true));
        // No other windows: exactly the main window's rule.
        for (main, min) in [(Some(true), None), (Some(true), Some(true)), (None, None), (Some(false), None)] {
            assert_eq!(lodestar_focus(main, false, min, []), effective_focus(main, false, min));
        }
        // Hidden to the tray: never focused, whatever a (stale) report says.
        assert_eq!(lodestar_focus(Some(true), true, None, [w(Some(true))]), Some(false));
        let st = Situation { focused: lodestar_focus(None, true, None, [w(Some(true))]), game_up: true, ..s() };
        assert_eq!(filter(&st, ALL.to_vec()), vec![Nav::Guide]);
    }

    /// REGRESSION (critical): a dialog window that loses the focus to a game is unfocused as
    /// soon as its own pass saw it, even while eframe still hands the main window's pass the
    /// old "focused" report; a minimised dialog window never counts.
    #[test]
    fn stale_or_minimised_dialog_window_is_not_focused() {
        // The pass the focus change woke up: reported (stale) focused, its own pass saw not.
        let stale = OtherWindow { reported: Some(true), minimised: Some(false), latest: Some(false) };
        assert!(!stale.focused());
        let f = lodestar_focus(Some(false), false, Some(false), [stale]);
        assert_eq!(f, Some(false));
        let st = Situation { focused: f, game_up: true, ..s() };
        assert_eq!(filter(&st, ALL.to_vec()), vec![Nav::Guide]);
        // Minimised (some compositors keep reporting the focus): not focused.
        let min = OtherWindow { reported: Some(true), minimised: Some(true), latest: None };
        assert_eq!(lodestar_focus(Some(false), false, Some(false), [min]), Some(false));
        // Not drawn yet (its own pass hasn't run): the report decides; both agreeing: focused.
        let new = OtherWindow { reported: Some(true), minimised: None, latest: None };
        assert!(new.focused());
        let both = OtherWindow { reported: Some(true), minimised: Some(false), latest: Some(true) };
        assert!(both.focused());
        // Its own pass says focused but the report doesn't (yet): not focused (fail-safe).
        let early = OtherWindow { reported: Some(false), minimised: Some(false), latest: Some(true) };
        assert!(!early.focused());
        assert!(!OtherWindow { reported: None, ..both }.focused());
    }

    /// REGRESSION (critical): a game started outside Lodestar (Steam client, desktop shortcut,
    /// another launcher) while Lodestar sits unfocused with background input on — Lodestar
    /// doesn't follow it (`game_up` false), but the background look saw it: only Guide.
    /// Fail safe: no look yet, or a stale one (`others_clear` false), is treated the same.
    #[test]
    fn game_started_elsewhere_lets_only_guide_through_in_the_background() {
        for (others_clear, others_seen) in [(false, true), (false, false)] {
            for focused in [Some(false), None] {
                let st = Situation { focused, others_clear, others_seen, ..s() };
                assert!(!st.is_focused() || (focused.is_none() && !others_seen), "{st:?}");
                if st.is_focused() {
                    continue; // a window that never got a focus report yet: as before
                }
                assert!(!st.background(), "{st:?}");
                assert_eq!(gate(&st), Gate::GuideOnly, "{st:?}");
                assert_eq!(filter(&st, ALL.to_vec()), vec![Nav::Guide], "{st:?}");
                let off = Situation { guide_summons: false, ..st };
                assert!(filter(&off, ALL.to_vec()).is_empty(), "{off:?}");
            }
        }
        // A game was seen and no focus was ever reported: unknown focus is no focus.
        let st = Situation { focused: None, focus_seen: false, others_clear: false, others_seen: true, ..s() };
        assert!(!st.is_focused());
        assert_eq!(filter(&st, ALL.to_vec()), vec![Nav::Guide]);
        // Focused: the user is using Lodestar, whatever runs elsewhere.
        let st = Situation { focused: Some(true), others_clear: false, others_seen: true, ..s() };
        assert_eq!(filter(&st, ALL.to_vec()), ALL.to_vec());
        // A fresh look found nothing: the background controller works.
        let st = Situation { focused: Some(false), ..s() };
        assert!(st.background());
        assert_eq!(filter(&st, ALL.to_vec()), ALL.to_vec());
    }

    /// Hidden to the tray or minimised: background input never acts (only Guide brings
    /// Lodestar back), with or without a game.
    #[test]
    fn hidden_window_ignores_background_input() {
        for game_up in [false, true] {
            let st = Situation { hidden: true, game_up, focused: effective_focus(Some(true), true, None), ..s() };
            assert_eq!(filter(&st, ALL.to_vec()), vec![Nav::Guide], "{st:?}");
            assert!(!st.wants_background_look());
        }
    }

    /// The background look runs exactly while background input would otherwise act.
    #[test]
    fn background_look_wanted() {
        let st = Situation { focused: Some(false), others_clear: false, ..s() };
        assert!(st.wants_background_look());
        assert!(!Situation { focused: Some(true), ..st }.wants_background_look());
        assert!(!Situation { background_input: false, ..st }.wants_background_look());
        assert!(!Situation { game_up: true, ..st }.wants_background_look());
        assert!(!Situation { scripted: true, ..st }.wants_background_look());
    }

    #[test]
    fn screenshot_runs_never_use_the_pad() {
        for game_up in [false, true] {
            for focused in [Some(true), Some(false), None] {
                let st = Situation { scripted: true, game_up, focused, ..s() };
                assert_eq!(gate(&st), Gate::Off);
                assert!(filter(&st, ALL.to_vec()).is_empty());
            }
        }
    }

    #[test]
    fn guide_debounce() {
        let t0 = std::time::Instant::now();
        let mut d = GuideDebounce::default();
        assert!(d.press(t0));
        assert!(!d.press(t0 + std::time::Duration::from_millis(100)));
        assert!(d.press(t0 + std::time::Duration::from_millis(500)));
    }

    /// The pad thread under this frame's gate: `presses` (ms, button) are held from then on,
    /// `releases` let go; the menu combination is checked whenever it falls due. What reaches
    /// Lodestar is what [`filter`] keeps of the pad's output.
    fn through(
        st: &Situation,
        cfg: &crate::config::ControllerConfig,
        presses: &[(u64, gilrs::Button)],
        releases: &[(u64, gilrs::Button)],
    ) -> Vec<Nav> {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut m = crate::input::PadMapper::new(crate::input::PadConfig::from_controller(cfg));
        let g = gate(st);
        m.set_enabled(g != Gate::Off);
        m.set_guide_only(g == Gate::GuideOnly);
        m.set_home(cfg.home_opens_menu());
        let mut out = vec![];
        for t in 0..=3000 {
            for &(_, b) in presses.iter().filter(|p| p.0 == t) {
                m.feed(crate::input::PadInput::Pressed(0, b), at(t), &mut out);
            }
            for &(_, b) in releases.iter().filter(|p| p.0 == t) {
                m.feed(crate::input::PadInput::Released(0, b), at(t), &mut out);
            }
            m.tick_menu_combo(at(t), &mut out);
        }
        filter(st, out)
    }

    /// Every situation the pad can be in: no game (focused, background), a game with Lodestar
    /// unfocused, a game with Lodestar focused (the Playing screen), Lodestar's in-game menu open.
    fn situations() -> Vec<Situation> {
        vec![
            s(),
            Situation { focused: Some(false), ..s() },
            Situation { game_up: true, focused: Some(false), ..s() },
            Situation { game_up: true, focused: None, ..s() },
            Situation { game_up: true, ..s() },
            Situation { game_up: true, lodestar_menu: true, ..s() },
        ]
    }

    fn game_unfocused() -> Situation {
        Situation { game_up: true, focused: Some(false), ..s() }
    }

    /// The presets and recorded ones: two to four buttons, ones games use all the time too.
    fn combos() -> Vec<crate::config::MenuCombo> {
        use crate::config::{MenuCombo, PadButton::*};
        let mut v = MenuCombo::PRESETS.to_vec();
        v.extend([
            MenuCombo::of([A, B]),
            MenuCombo::of([Lb, Y]),
            MenuCombo::of([DpadUp, Select, X]),
            MenuCombo::of([Lt, Rt, Start, R3]),
        ]);
        v
    }

    /// A combination's buttons as the pad reports them.
    fn pad_buttons(c: crate::config::MenuCombo) -> Vec<gilrs::Button> {
        c.buttons().map(crate::input::pad::gilrs_button).collect()
    }

    /// REGRESSION (critical): the menu combination held → the menu button (Guide), in every
    /// situation, a game running in front included.
    #[test]
    fn menu_combo_held_is_the_menu_button() {
        use gilrs::Button::{LeftThumb as L3, RightThumb as R3};
        let cfg = crate::config::ControllerConfig::default();
        for st in situations() {
            assert_eq!(through(&st, &cfg, &[(0, L3), (100, R3)], &[]), vec![Nav::Guide], "{st:?}");
            assert_eq!(through(&st, &cfg, &[(0, R3), (40, L3)], &[(2000, L3)]), vec![Nav::Guide], "{st:?}");
        }
    }

    /// REGRESSION (critical): a quick L3 + R3 in a game, or one too slow to be pressed together,
    /// never opens Lodestar.
    #[test]
    fn menu_combo_tapped_does_nothing() {
        use gilrs::Button::{LeftThumb as L3, RightThumb as R3};
        let cfg = crate::config::ControllerConfig::default();
        for st in situations() {
            assert!(through(&st, &cfg, &[(0, L3), (20, R3)], &[(250, L3), (260, R3)]).is_empty(), "{st:?}");
            assert!(through(&st, &cfg, &[(0, L3), (400, R3)], &[]).is_empty(), "{st:?}");
        }
    }

    /// REGRESSION (critical): while a game runs (Lodestar unfocused), a single stick click — or
    /// any button of a combination, a preset or a recorded one, alone or tapped — never reaches
    /// Lodestar.
    #[test]
    fn single_combo_buttons_never_get_through_while_a_game_runs() {
        use crate::config::ControllerConfig;
        use gilrs::Button;
        for combo in combos() {
            let cfg = ControllerConfig { menu_combo: combo, ..Default::default() };
            let buttons = pad_buttons(combo);
            for &b in &buttons {
                let st = game_unfocused();
                assert!(through(&st, &cfg, &[(0, b)], &[]).is_empty(), "{combo:?} {b:?} held");
                assert!(through(&st, &cfg, &[(0, b)], &[(80, b)]).is_empty(), "{combo:?} {b:?} tapped");
            }
            // All but the last: nothing.
            let some: Vec<(u64, Button)> = buttons[..buttons.len() - 1].iter().map(|&b| (0, b)).collect();
            assert!(through(&game_unfocused(), &cfg, &some, &[]).is_empty(), "{combo:?}");
            // All, tapped: nothing; all, held: the menu button only.
            let all: Vec<(u64, Button)> = buttons.iter().enumerate().map(|(i, &b)| (i as u64 * 30, b)).collect();
            let up: Vec<(u64, Button)> = buttons.iter().map(|&b| (200, b)).collect();
            assert!(through(&game_unfocused(), &cfg, &all, &up).is_empty(), "{combo:?}");
            assert_eq!(through(&game_unfocused(), &cfg, &all, &[]), vec![Nav::Guide], "{combo:?}");
        }
    }

    /// REGRESSION (critical): a combination begun in Lodestar and finished once a game runs in
    /// front never lets its buttons through, and doesn't count (fail safe: it starts over under
    /// the new gate); begun in a game and finished in Lodestar, it doesn't count either.
    #[test]
    fn menu_combo_across_a_game_starting_never_leaks_its_buttons() {
        use crate::config::ControllerConfig;
        use crate::input::{PadConfig, PadInput, PadMapper};
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        for combo in combos() {
            let cfg = ControllerConfig { menu_combo: combo, ..Default::default() };
            let buttons = pad_buttons(combo);
            for (before, after) in [(s(), game_unfocused()), (game_unfocused(), s())] {
                let mut m = PadMapper::new(PadConfig::from_controller(&cfg));
                let mut out = vec![];
                let step =
                    |m: &mut PadMapper, st: &Situation, ms: u64, b: Option<gilrs::Button>, out: &mut Vec<Nav>| {
                        let g = gate(st);
                        m.set_enabled(g != Gate::Off);
                        m.set_guide_only(g == Gate::GuideOnly);
                        if let Some(b) = b {
                            m.feed(PadInput::Pressed(0, b), at(ms), out);
                        }
                        m.tick_menu_combo(at(ms), out);
                        m.tick(at(ms), out);
                    };
                step(&mut m, &before, 0, Some(buttons[0]), &mut out);
                // What the first press did in Lodestar is Lodestar's; from here on, the game's.
                out.clear();
                for (i, &b) in buttons[1..].iter().enumerate() {
                    step(&mut m, &after, 20 + i as u64 * 20, Some(b), &mut out);
                }
                for ms in (100..3000).step_by(10) {
                    step(&mut m, &after, ms, None, &mut out);
                }
                let got = filter(&after, out);
                if after.game_up {
                    assert!(got.is_empty(), "{combo:?}: {got:?}");
                } else {
                    assert!(!got.contains(&Nav::Guide), "{combo:?}: {got:?}");
                }
            }
        }
    }

    /// Steam Input's virtual pad mirrors the real one: the combination held on both counts once.
    #[test]
    fn menu_combo_on_a_mirrored_pad_counts_once() {
        use crate::input::{PadConfig, PadInput, PadMapper};
        use gilrs::Button::{LeftThumb as L3, RightThumb as R3};
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut m = PadMapper::new(PadConfig::from_controller(&crate::config::ControllerConfig::default()));
        m.set_guide_only(true);
        let mut out = vec![];
        for (ms, pad, b) in [(0, 0, L3), (3, 1, L3), (50, 0, R3), (54, 1, R3)] {
            m.feed(PadInput::Pressed(pad, b), at(ms), &mut out);
        }
        let mut debounce = GuideDebounce::default();
        let mut counted = 0;
        for ms in 0..2000 {
            m.tick_menu_combo(at(ms), &mut out);
            for n in out.drain(..) {
                if n == Nav::Guide && debounce.press(at(ms)) {
                    counted += 1;
                }
            }
        }
        assert_eq!(counted, 1);
    }

    /// The combination chosen: Home is ignored everywhere (left to Steam and emulators), once
    /// the question was answered. Home chosen: the combination's buttons are just buttons.
    #[test]
    fn the_button_not_chosen_never_reaches_lodestar() {
        use crate::config::{ControllerConfig, MenuButton};
        use gilrs::Button::{LeftThumb as L3, Mode as Home, RightThumb as R3};
        let combo = ControllerConfig { menu_button_asked: true, ..Default::default() };
        for st in situations() {
            assert!(through(&st, &combo, &[(0, Home)], &[(100, Home)]).is_empty(), "{st:?}");
            assert_eq!(through(&st, &combo, &[(0, L3), (10, R3)], &[]), vec![Nav::Guide], "{st:?}");
        }
        let st = game_unfocused();
        // Not asked yet: Home too, so its first press can ask.
        let unasked = ControllerConfig::default();
        assert_eq!(through(&st, &unasked, &[(0, Home)], &[]), vec![Nav::Guide]);
        assert_eq!(through(&st, &unasked, &[(0, L3), (10, R3)], &[]), vec![Nav::Guide]);
        let home = ControllerConfig { menu_button: MenuButton::Home, menu_button_asked: true, ..Default::default() };
        assert!(through(&st, &home, &[(0, L3), (10, R3)], &[]).is_empty());
        assert_eq!(through(&st, &home, &[(0, Home)], &[]), vec![Nav::Guide]);
        // Turned off altogether ("brings up Lodestar" off): nothing while a game runs.
        let off = Situation { guide_summons: false, ..st };
        assert!(through(&off, &unasked, &[(0, L3), (10, R3), (20, Home)], &[]).is_empty());
    }

    #[test]
    fn gate_round_trips() {
        for g in [Gate::Off, Gate::GuideOnly, Gate::All] {
            assert_eq!(Gate::from_u8(g.to_u8()), g);
        }
    }

    /// Desktop mode, the setting on, a clean look done.
    fn desk() -> DesktopPad {
        DesktopPad { enabled: true, looked: true, ..Default::default() }
    }

    /// Every situation and desktop state, built from the bits of `bits` and a focus report.
    fn every_desktop_case() -> impl Iterator<Item = (Situation, DesktopPad)> {
        [Some(true), Some(false), None].into_iter().flat_map(|focused| {
            (0..1u16 << 11).map(move |bits| {
                let on = |n: u16| bits & (1 << n) != 0;
                let sit = Situation {
                    focused,
                    focus_seen: on(0),
                    game_up: on(1),
                    others_seen: on(2),
                    others_clear: !on(2),
                    hidden: on(3),
                    scripted: on(4),
                    background_input: on(5),
                    ..s()
                };
                let d =
                    DesktopPad { enabled: on(6), launching: on(7), settling: on(8), looked: on(9), overlay: on(10) };
                (sit, d)
            })
        })
    }

    /// REGRESSION (critical, user-requested): the pad drives desktop mode only while Lodestar is
    /// reported focused, visible, with no game up, on its way, just ended or seen running, a look
    /// done, the in-game menu closed, the setting on, and never in scripted runs.
    #[test]
    fn desktop_pad_only_when_focused_and_nothing_runs() {
        let mut allowed = 0;
        for (sit, d) in every_desktop_case() {
            if !desktop_pad(&sit, d) {
                continue;
            }
            allowed += 1;
            assert_eq!(sit.focused, Some(true), "{sit:?} {d:?}");
            assert!(!sit.game_up && !sit.others_seen && !sit.hidden && !sit.scripted, "{sit:?} {d:?}");
            assert!(d.enabled && d.looked && !d.launching && !d.settling && !d.overlay, "{sit:?} {d:?}");
        }
        // That idle window, whether a focus event came yet and whatever background input says.
        assert_eq!(allowed, 4);
        assert!(desktop_pad(&s(), desk()));
    }

    /// REGRESSION: background input lets the controller drive an unfocused controller mode, but
    /// never desktop mode.
    #[test]
    fn desktop_pad_ignores_background_input() {
        let st = Situation { focused: Some(false), background_input: true, others_clear: true, ..s() };
        assert_eq!(filter(&st, ALL.to_vec()), ALL.to_vec());
        assert!(!desktop_pad(&st, desk()));
    }

    /// An unknown focus counts as focused for controller mode before the first focus event, but
    /// never for desktop mode.
    #[test]
    fn desktop_pad_never_trusts_an_unknown_focus() {
        for focus_seen in [false, true] {
            let st = Situation { focused: None, focus_seen, ..s() };
            assert_eq!(st.is_focused(), !focus_seen);
            assert!(!desktop_pad(&st, desk()), "{st:?}");
        }
    }

    /// REGRESSION (critical): whenever the desktop rule lets the pad through, the safety rule
    /// already lets all of it through: the desktop rule only ever narrows it.
    #[test]
    fn desktop_pad_never_widens_the_gate() {
        let cases = situations().into_iter().flat_map(|sit| {
            (0..32u8).map(move |b| {
                let on = |n: u8| b & (1 << n) != 0;
                (sit, DesktopPad { enabled: on(0), launching: on(1), settling: on(2), looked: on(3), overlay: on(4) })
            })
        });
        for (sit, d) in cases.chain(every_desktop_case()) {
            if desktop_pad(&sit, d) {
                assert_eq!(gate(&sit), Gate::All, "{sit:?} {d:?}");
                assert_eq!(filter(&sit, ALL.to_vec()), ALL.to_vec(), "{sit:?} {d:?}");
            }
        }
    }

    /// Settings in its own window, focused: that is Lodestar being used, as for the gate. A
    /// stale or minimised dialog window isn't.
    #[test]
    fn focused_dialog_window_counts_for_the_desktop_pad() {
        let w = |reported, minimised, latest| OtherWindow { reported, minimised, latest };
        let focused = lodestar_focus(Some(false), false, Some(false), [w(Some(true), Some(false), None)]);
        assert!(desktop_pad(&Situation { focused, ..s() }, desk()));
        let stale = lodestar_focus(Some(false), false, Some(false), [w(Some(true), Some(false), Some(false))]);
        assert!(!desktop_pad(&Situation { focused: stale, ..s() }, desk()));
        let minimised = lodestar_focus(Some(false), false, Some(false), [w(Some(true), Some(true), None)]);
        assert!(!desktop_pad(&Situation { focused: minimised, ..s() }, desk()));
    }

    /// REGRESSION: a look counts for the desktop pad only when it started after the last game
    /// ended; before any game ended this run, any look does.
    #[test]
    fn only_a_look_after_the_last_game_counts() {
        use std::time::{Duration, Instant};
        let t0 = Instant::now();
        let later = t0 + Duration::from_secs(1);
        assert!(!looked_since(None, None), "no look yet");
        assert!(looked_since(Some(t0), None), "no game ended this run");
        assert!(looked_since(Some(later), Some(t0)));
        assert!(!looked_since(Some(t0), Some(later)), "a look from before the game ended");
        assert!(!looked_since(None, Some(t0)));
    }

    #[test]
    fn desktop_look_wanted() {
        // Unfocused: looking, so a game left running behind Lodestar is known when it comes back.
        let unfocused = Situation { focused: Some(false), background_input: false, ..s() };
        assert!(desktop_look(&unfocused, desk()));
        // Focused: until there is a look since the last game ended, and while the last one saw
        // a game.
        assert!(desktop_look(&s(), DesktopPad { looked: false, ..desk() }));
        assert!(desktop_look(&Situation { others_seen: true, others_clear: false, ..s() }, desk()));
        // A focused, idle window with a clean look stays idle.
        assert!(!desktop_look(&s(), desk()));
        // Off, scripted, hidden or with a game up: never.
        assert!(!desktop_look(&unfocused, DesktopPad { enabled: false, ..desk() }));
        for st in [
            Situation { scripted: true, ..unfocused },
            Situation { hidden: true, ..unfocused },
            Situation { game_up: true, ..unfocused },
        ] {
            assert!(!desktop_look(&st, desk()), "{st:?}");
        }
    }

    /// What one frame hands desktop mode: the pad thread under that frame's gate, then the
    /// frame's filter, then the desktop rule.
    fn desktop_gets(st: &Situation, d: DesktopPad, presses: &[(u64, gilrs::Button)]) -> Vec<Nav> {
        let navs = through(st, &crate::config::ControllerConfig::default(), presses, &[]);
        if desktop_pad(st, d) { navs } else { vec![] }
    }

    /// REGRESSION (critical): a whole evening with only a controller. The library works in
    /// front, a game in front gets the pad (only the menu button reaches Lodestar), the in-game
    /// menu takes it, and the library gets it back only once the game is over and settled.
    #[test]
    fn desktop_round_trip_with_only_a_controller() {
        use gilrs::Button::{DPadDown as Down, Mode as Home, South as A};
        let cfg = crate::config::ControllerConfig::default();
        // 1. In the library.
        assert_eq!(desktop_gets(&s(), desk(), &[(0, Down)]), vec![Nav::Down]);
        // 2. A game in front: only Home gets through, and not to the library.
        let playing = Situation { game_up: true, focused: Some(false), ..s() };
        assert_eq!(through(&playing, &cfg, &[(0, Down), (10, A), (20, Home)], &[]), vec![Nav::Guide]);
        assert!(desktop_gets(&playing, desk(), &[(0, Down), (10, A)]).is_empty());
        // 3. Home brought the in-game menu up over a focused Lodestar: the menu has the pad.
        let menu = Situation { game_up: true, lodestar_menu: true, ..s() };
        assert!(!desktop_pad(&menu, DesktopPad { overlay: true, ..desk() }));
        assert_eq!(through(&menu, &cfg, &[(0, Down)], &[]), vec![Nav::Down]);
        // 4. Quit the game: a moment of Home only. The look from before the game no longer
        // counts, so Lodestar looks again, and the pad waits for that look even once settled.
        let t0 = std::time::Instant::now();
        let ended = t0 + std::time::Duration::from_secs(60);
        let stale = DesktopPad { looked: looked_since(Some(t0), Some(ended)), ..desk() };
        assert!(desktop_gets(&s(), DesktopPad { settling: true, ..stale }, &[(0, Down)]).is_empty());
        assert!(desktop_gets(&s(), stale, &[(0, Down)]).is_empty());
        assert!(desktop_look(&s(), stale));
        // 5. Settled, focused, a clean look since the game ended: the library again, and no
        // more looking.
        let after = ended + std::time::Duration::from_millis(5);
        let fresh = DesktopPad { looked: looked_since(Some(after), Some(ended)), ..desk() };
        assert_eq!(desktop_gets(&s(), fresh, &[(0, Down)]), vec![Nav::Down]);
        assert!(!desktop_look(&s(), fresh));

        // (a) The game closed itself and the focus stayed elsewhere: Home only, until Home
        // brought Lodestar to the front.
        let behind = Situation { focused: Some(false), ..s() };
        assert_eq!(through(&behind, &cfg, &[(0, Down), (20, Home)], &[]), vec![Nav::Down, Nav::Guide]);
        assert!(desktop_gets(&behind, desk(), &[(0, Down)]).is_empty());
        assert_eq!(desktop_gets(&s(), desk(), &[(0, Down)]), vec![Nav::Down]);
        // (b) A game still seen running after the exit: no pad, the look keeps going until a
        // clean one.
        let seen = Situation { others_seen: true, others_clear: false, ..s() };
        assert!(desktop_gets(&seen, desk(), &[(0, Down)]).is_empty());
        assert!(desktop_look(&seen, desk()));
        assert!(!desktop_look(&s(), desk()));
        // (c) Hidden to the tray meanwhile: nothing until it is shown again.
        let hidden = Situation { hidden: true, focused: effective_focus(Some(true), true, None), ..s() };
        assert!(desktop_gets(&hidden, desk(), &[(0, Down)]).is_empty());
    }
}
