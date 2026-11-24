//! "Change combination" (from "Which button opens Lodestar's menu?" and Settings → Input): the
//! preset menu button combinations, the one in use, and "Record a combination", which records
//! the buttons the user holds together on the pad (input.rs `record_combo`; the pad does nothing
//! else meanwhile) and offers Use / Try again / Cancel. A take needs at least two buttons and
//! can't include Home; one made only of buttons games use all the time is allowed with a warning.
//! While it waits the pad's buttons don't act, so the screen shows no button hints; the Back
//! button pressed on its own (never a usable take) cancels.
//! Both are centred panels on top of everything (ingame.rs `on_top`), in either mode.

use super::*;
use crate::config::{MenuButton, MenuCombo, PadButton};
use crate::input::{RecordPoll, Recorded};

/// How often the recording screen renews the pad's recording (input.rs lets it lapse after
/// 1.5 s without).
const RENEW: std::time::Duration = std::time::Duration::from_millis(500);

impl ControllerUi {
    pub(super) fn open_combo_picker(&mut self, st: &AppState, now: f64) {
        let mut m = Menu::new(
            MenuKind::MenuComboPick,
            "Change combination",
            combo_picker_items(st.config.controller.menu_combo),
            now,
        )
        .with_subtitle("Held together for a moment, it opens Lodestar's menu")
        .centred();
        m.select_act(&Act::MenuComboPreset(st.config.controller.menu_combo));
        self.sys.menus.push(m);
    }

    pub(super) fn open_combo_recorder(&mut self, st: &AppState, now: f64) {
        self.sys.take = None;
        let m =
            Menu::new(MenuKind::MenuComboRecord, "Record a combination", waiting_items(st, MenuCombo::empty()), now)
                .with_subtitle("Hold the buttons together, then let go")
                .with_hints(vec![])
                .centred();
        self.sys.menus.push(m);
        st.ctx().request_repaint();
    }

    pub(super) fn record_combo_again(&mut self, st: &AppState, now: f64) {
        self.sys.menus.retain(|m| m.kind != MenuKind::MenuComboRecord);
        self.open_combo_recorder(st, now);
    }

    /// A preset or a recorded take becomes the combination: saved, the combination screens close
    /// (back to the question or Settings, both showing it).
    pub(super) fn use_menu_combo(&mut self, st: &mut AppState, combo: MenuCombo) {
        st.config.controller.menu_combo = combo;
        if st.config.controller.menu_button_asked {
            // Picked in Settings: the combination is what opens the menu now.
            st.config.controller.menu_button = MenuButton::Combo;
        }
        st.save_config();
        self.sys.menus.retain(|m| !matches!(m.kind, MenuKind::MenuComboPick | MenuKind::MenuComboRecord));
        let question = Self::menu_button_question(&st.config.controller);
        if let Some(m) = self.sys.menus.iter_mut().find(|m| m.kind == MenuKind::MenuButtonAsk) {
            m.replace_items(question);
            m.select_act(&Act::MenuButtonPick(MenuButton::Combo));
        }
        self.refresh_settings(st);
    }

    /// Per frame: while the recording screen waits, keep the pad recording and show what is held;
    /// the take replaces it with the verdict. Any other way it closed stops the recording.
    pub(super) fn combo_tick(&mut self, st: &mut AppState) {
        // Not while the focus is with the game (the screen was left for it): the pad's presses
        // are the game's, never a combination. The menu button brings it back and it records
        // afresh.
        let waiting = self.sys.menus.last().is_some_and(|m| m.kind == MenuKind::MenuComboRecord)
            && self.sys.take.is_none()
            && !self.sys.left_for_game;
        if !waiting {
            if std::mem::take(&mut self.sys.recording) {
                st.input.stop_recording();
            }
            return;
        }
        self.sys.recording = true;
        match st.input.record_combo() {
            RecordPoll::Waiting(live) => {
                let items = waiting_items(st, live);
                let Some(m) = self.sys.menus.last_mut() else { return };
                if m.items.first().map(|i| &i.label) != items.first().map(|i| &i.label) {
                    m.replace_items(items);
                }
                st.ctx().request_repaint_after(RENEW);
            }
            RecordPoll::Done(take) if is_back_alone(take, st.config.controller.swap_ab) => {
                // Back on its own: cancel, back to the combination list.
                self.sys.recording = false;
                self.sys.menus.retain(|m| m.kind != MenuKind::MenuComboRecord);
            }
            RecordPoll::Done(take) => {
                self.sys.recording = false;
                self.show_take(take);
            }
        }
    }

    /// The recording screen shows the take's verdict, its first choice selected (Use, when the
    /// take can be used).
    fn show_take(&mut self, take: Recorded) {
        self.sys.take = Some(take);
        let Some(m) = self.sys.menus.last_mut().filter(|m| m.kind == MenuKind::MenuComboRecord) else { return };
        let items = take_items(take);
        let first = items.iter().find_map(|i| i.act.clone());
        m.replace_items(items);
        // The pad acts normally again: Select / Back.
        m.hints = None;
        if let Some(a) = first {
            m.select_act(&a);
        }
    }
}

/// The presets, the combination in use when it was recorded, and "Record a combination".
fn combo_picker_items(current: MenuCombo) -> Vec<Item> {
    let mut v = vec![];
    let in_use = |c: MenuCombo| if c == current { "In use" } else { "" };
    if !MenuCombo::PRESETS.contains(&current) {
        v.push(Item::value(current.label(), "In use", Act::MenuComboPreset(current)).with_help(current.how()));
    }
    for c in MenuCombo::PRESETS {
        let help =
            if c == MenuCombo::STICKS { format!("{} The safest: games rarely use it.", c.how()) } else { c.how() };
        v.push(Item::value(c.label(), in_use(c), Act::MenuComboPreset(c)).with_help(help));
    }
    v.push(
        Item::action("Record a combination", Act::RecordMenuCombo)
            .with_help("Hold the buttons you want on the controller, then let go."),
    );
    v
}

/// The recording screen while it waits: what is held so far. Cancel is for the mouse / Esc; on
/// the pad it's the Back button alone.
fn waiting_items(st: &AppState, held: MenuCombo) -> Vec<Item> {
    let now = if held.is_empty() { "Nothing yet".to_string() } else { held.label() };
    let back = st.prompts().name(crate::theme::render::draw::Button::B);
    vec![
        Item::info(
            now,
            format!(
                "The controller's buttons are recorded, not used, until you let go of them. Use at least two; \
                 Home can't be one of them. Press {back} on its own to cancel."
            ),
        ),
        Item::action("Cancel", Act::CloseMenu),
    ]
}

/// A take of only the Back button (B, or A with confirm / back swapped): it can never be a
/// combination, so it means Cancel.
fn is_back_alone(take: Recorded, swap_ab: bool) -> bool {
    let back = if swap_ab { PadButton::A } else { PadButton::B };
    !take.home && take.combo == MenuCombo::of([back])
}

/// The recording screen with a take: the verdict, Use (when it can be) / Try again / Cancel.
fn take_items(take: Recorded) -> Vec<Item> {
    let again = Item::action("Try again", Act::RecordComboAgain);
    let cancel = Item::action("Cancel", Act::CloseMenu);
    let v = take_verdict(take);
    let mut items = vec![Item::info(v.heading, v.text)];
    if v.usable {
        items.push(Item::action("Use this combination", Act::RecordedComboUse(take.combo)));
    }
    items.extend([again, cancel]);
    items
}

/// What a recording screen says about a take (both modes).
pub(crate) struct Verdict {
    pub heading: String,
    pub text: String,
    /// It can be used (maybe with a warning in `text`).
    pub usable: bool,
}

pub(crate) fn take_verdict(take: Recorded) -> Verdict {
    let combo = take.combo;
    let refuse = |heading: String, text: &str| Verdict { heading, text: text.into(), usable: false };
    if take.home {
        return refuse(
            "Home can't be part of it".into(),
            "Steam and emulators use the Home button. Try again without it.",
        );
    }
    if !combo.valid() {
        let what = if combo.is_empty() { "No buttons".to_string() } else { combo.label() };
        return refuse(what, "Hold at least two buttons together, then let go of them.");
    }
    let text = if combo.busy() {
        format!(
            "Games use these buttons all the time, so holding them in a game could open Lodestar's menu. {} is safer.",
            MenuCombo::STICKS.label()
        )
    } else {
        "Held together for a moment, they open Lodestar's menu.".to_string()
    };
    Verdict { heading: combo.label(), text, usable: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verdict(take: Recorded) -> (String, Vec<Act>) {
        let items = take_items(take);
        (items[0].value.clone().unwrap_or_default(), items.iter().filter_map(|i| i.act.clone()).collect())
    }

    #[test]
    fn a_take_needs_two_buttons_and_no_home_and_busy_ones_are_warned_about() {
        let use_it = |c| Act::RecordedComboUse(c);
        let (text, acts) = verdict(Recorded { combo: MenuCombo::BUMPERS_START, home: false });
        assert!(text.contains("open Lodestar's menu"), "{text}");
        assert_eq!(acts, [use_it(MenuCombo::BUMPERS_START), Act::RecordComboAgain, Act::CloseMenu]);
        let face = MenuCombo::of([PadButton::A, PadButton::B]);
        let (text, acts) = verdict(Recorded { combo: face, home: false });
        assert!(text.contains("Games use these buttons all the time") && text.contains("L3 + R3 is safer"), "{text}");
        assert_eq!(acts[0], use_it(face), "allowed anyway");
        for take in [
            Recorded { combo: MenuCombo::of([PadButton::A]), home: false },
            Recorded { combo: MenuCombo::empty(), home: false },
            Recorded { combo: MenuCombo::STICKS, home: true },
        ] {
            let (_, acts) = verdict(take);
            assert_eq!(acts, [Act::RecordComboAgain, Act::CloseMenu], "{take:?}");
        }
    }

    #[test]
    fn the_picker_offers_the_presets_and_a_recorded_one_in_use() {
        let labels = |c| {
            combo_picker_items(c)
                .iter()
                .map(|i| (i.label.clone(), i.value.clone().unwrap_or_default()))
                .collect::<Vec<_>>()
        };
        let l = labels(MenuCombo::SELECT_START);
        assert_eq!(l.len(), 4);
        assert_eq!(l[1], ("Select + Start".to_string(), "In use".to_string()));
        assert_eq!(l[3].0, "Record a combination");
        let custom = MenuCombo::of([PadButton::Lt, PadButton::Rt, PadButton::Y]);
        let l = labels(custom);
        assert_eq!(l[0], ("LT + RT + Y".to_string(), "In use".to_string()));
        assert_eq!(l.len(), 5);
    }

    #[test]
    fn recording_from_the_question_and_using_the_take() {
        // No other test's fake game may run during this one.
        let _fake = crate::ui::launching::FAKE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let mut ui = ControllerUi::new();
        ui.on_guide(&mut st, false, 0.0);
        assert_eq!(ui.sys.menus.last().map(|m| m.kind.clone()), Some(MenuKind::MenuButtonAsk));
        // Change combination → Record a combination.
        ui.sys_nav(&mut st, Nav::Down, 1.0);
        ui.sys_nav(&mut st, Nav::Accept, 1.0);
        assert_eq!(ui.sys.menus.last().map(|m| m.kind.clone()), Some(MenuKind::MenuComboPick));
        for _ in 0..3 {
            ui.sys_nav(&mut st, Nav::Down, 1.0);
        }
        ui.sys_nav(&mut st, Nav::Accept, 1.0);
        assert_eq!(ui.sys.menus.last().map(|m| m.kind.clone()), Some(MenuKind::MenuComboRecord));
        ui.sys_tick(&mut st, 1.1);
        assert!(ui.sys.recording);
        assert_eq!(ui.sys.menus.last().unwrap().items[0].label, "Nothing yet", "nothing is held at the start");
        // The pad thread's take arrives (stood in for here).
        let take = Recorded { combo: MenuCombo::of([PadButton::Lb, PadButton::Rb]), home: false };
        ui.show_take(take);
        ui.sys_tick(&mut st, 1.2);
        assert!(!ui.sys.recording, "the take stops the recording");
        ui.sys_nav(&mut st, Nav::Accept, 1.3);
        // Back on the question, which names the take; not answered yet.
        assert_eq!(ui.sys.menus.last().map(|m| m.kind.clone()), Some(MenuKind::MenuButtonAsk));
        assert_eq!(st.config.controller.menu_combo, take.combo);
        assert!(!st.config.controller.menu_button_asked);
        let first = &ui.sys.menus.last().unwrap().items[0];
        assert_eq!(first.label, "A button combination (LB + RB)");
        assert_eq!(ui.sys.menus.last().unwrap().selected_act(), Some(&Act::MenuButtonPick(MenuButton::Combo)));
        ui.sys_nav(&mut st, Nav::Accept, 1.4);
        assert!(ui.sys.menus.is_empty() && st.config.controller.menu_button_asked);
        let saved = crate::config::Config::load(&st.paths.config_file()).controller;
        assert_eq!((saved.menu_button, saved.menu_combo), (MenuButton::Combo, take.combo));
    }

    /// REGRESSION (safety rule): "Record a combination" left on screen when the focus went to
    /// the game stops recording at once, so the game's presses never become the combination,
    /// and the game's A can't use a take.
    #[test]
    fn the_recorder_left_for_the_game_records_nothing() {
        use crate::input::gate::{Situation, filter};
        let _fake = crate::ui::launching::FAKE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        st.no_launch = true;
        st.add_manual_game(
            "Test Game",
            LaunchSpec::Command { program: "/bin/true".into(), args: vec![], cwd: None, env: vec![] },
            Art::default(),
        );
        let id = st.library.games[0].id;
        assert!(crate::ui::launching::debug_fake(&st, id, "starting"));
        let mut ui = ControllerUi::new();
        ui.on_guide(&mut st, true, 0.0);
        ui.open_combo_picker(&st, 0.1);
        ui.open_combo_recorder(&st, 0.2);
        ui.sys_tick(&mut st, 0.3);
        assert!(ui.sys.recording);
        ui.focus_left_for_game();
        ui.sys_tick(&mut st, 0.4);
        assert!(!ui.sys.recording, "the focus went to the game: no recording");
        // Even a take that got through can't be used by the game's A.
        let take = Recorded { combo: MenuCombo::of([PadButton::A, PadButton::B]), home: false };
        ui.show_take(take);
        let sit = Situation {
            game_up: true,
            focused: Some(true),
            guide_summons: true,
            lodestar_menu: ui.wants_full_input(),
            ..Default::default()
        };
        for n in filter(&sit, vec![Nav::Accept]) {
            ui.sys_nav(&mut st, n, 0.5);
        }
        assert_eq!(st.config.controller.menu_combo, MenuCombo::STICKS, "the game's presses became the combination");
        // The menu button brings the recorder back under the pad: it records again.
        ui.record_combo_again(&st, 0.6);
        ui.on_guide(&mut st, true, 0.7);
        ui.sys_tick(&mut st, 0.8);
        assert!(ui.sys.recording);
        crate::ui::launching::stop(&mut st, id);
    }

    #[test]
    fn back_alone_cancels_the_recording() {
        assert!(is_back_alone(Recorded { combo: MenuCombo::of([PadButton::B]), home: false }, false));
        assert!(
            !is_back_alone(Recorded { combo: MenuCombo::of([PadButton::B]), home: false }, true),
            "swapped: A is Back"
        );
        assert!(is_back_alone(Recorded { combo: MenuCombo::of([PadButton::A]), home: false }, true));
        assert!(!is_back_alone(Recorded { combo: MenuCombo::of([PadButton::B, PadButton::Lb]), home: false }, false));
        assert!(!is_back_alone(Recorded { combo: MenuCombo::of([PadButton::B]), home: true }, false));
        // While waiting the screen shows no button hints; a take brings Select / Back back.
        let dir = tempfile::tempdir().unwrap();
        let st = AppState::for_test(dir.path());
        let mut ui = ControllerUi::new();
        ui.open_combo_recorder(&st, 0.0);
        assert_eq!(ui.sys.menus.last().unwrap().hints, Some(vec![]));
        ui.show_take(Recorded { combo: MenuCombo::STICKS, home: false });
        assert_eq!(ui.sys.menus.last().unwrap().hints, None);
    }

    #[test]
    fn closing_the_recorder_any_way_stops_the_recording() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        st.config.controller.menu_button_asked = true;
        let mut ui = ControllerUi::new();
        ui.open_combo_picker(&st, 0.0);
        ui.open_combo_recorder(&st, 0.0);
        ui.sys_tick(&mut st, 0.1);
        assert!(ui.sys.recording);
        // Back (Escape on the keyboard: the pad is the recorder's) → the picker; no recording.
        ui.sys_nav(&mut st, Nav::Back, 0.2);
        ui.sys_tick(&mut st, 0.3);
        assert_eq!(ui.sys.menus.last().map(|m| m.kind.clone()), Some(MenuKind::MenuComboPick));
        assert!(!ui.sys.recording);
        // A preset from Settings: in use, and the combination is what opens the menu now.
        st.config.controller.menu_button = MenuButton::Home;
        ui.sys_nav(&mut st, Nav::Down, 0.4);
        ui.sys_nav(&mut st, Nav::Accept, 0.4);
        assert!(ui.sys.menus.is_empty());
        assert_eq!(
            (st.config.controller.menu_button, st.config.controller.menu_combo),
            (MenuButton::Combo, MenuCombo::SELECT_START)
        );
    }
}
