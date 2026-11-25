use super::*;
use egui::RawInput;

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
const MODES: [Mode; 3] = [Mode::Library, Mode::Dialog, Mode::Menu];
const FOCUSED: [Focused; 4] = [Focused::Nothing, Focused::Value, Focused::Text, Focused::Other];

#[test]
fn library_presses_are_the_librarys_keys() {
    use PadAct as A;
    for f in FOCUSED {
        let p = |n| plan(n, Mode::Library, f);
        assert_eq!(p(Nav::Up), A::Key(Key::ArrowUp));
        assert_eq!(p(Nav::Down), A::Key(Key::ArrowDown));
        assert_eq!(p(Nav::Left), A::Key(Key::ArrowLeft));
        assert_eq!(p(Nav::Right), A::Key(Key::ArrowRight));
        assert_eq!(p(Nav::Accept), A::Key(Key::Enter));
        assert_eq!(p(Nav::Favorite), A::GameMenu);
        assert_eq!(p(Nav::Search), A::Search);
        assert_eq!(p(Nav::Menu), A::AppMenu);
        assert_eq!(p(Nav::Select), A::NextView);
        assert_eq!(p(Nav::PageUp), A::Preset(-1));
        assert_eq!(p(Nav::PageDown), A::Preset(1));
        assert_eq!(p(Nav::JumpUp), A::Key(Key::PageUp));
        assert_eq!(p(Nav::JumpDown), A::Key(Key::PageDown));
    }
}

#[test]
fn dialog_presses_follow_the_tab_order() {
    use FocusDirection as F;
    use PadAct as A;
    for f in [Focused::Nothing, Focused::Text, Focused::Other] {
        assert_eq!(plan(Nav::Up, Mode::Dialog, f), A::Focus(F::Previous));
        assert_eq!(plan(Nav::Left, Mode::Dialog, f), A::Focus(F::Previous));
        assert_eq!(plan(Nav::Down, Mode::Dialog, f), A::Focus(F::Next));
        assert_eq!(plan(Nav::Right, Mode::Dialog, f), A::Focus(F::Next));
    }
    // A slider or number box: ← → change it, ↑ ↓ still move on.
    assert_eq!(plan(Nav::Left, Mode::Dialog, Focused::Value), A::Step(false));
    assert_eq!(plan(Nav::Right, Mode::Dialog, Focused::Value), A::Step(true));
    assert_eq!(plan(Nav::Down, Mode::Dialog, Focused::Value), A::Focus(F::Next));
    // A presses the focused widget; with nothing focused it first shows where you are.
    assert_eq!(plan(Nav::Accept, Mode::Dialog, Focused::Other), A::Key(Key::Enter));
    assert_eq!(plan(Nav::Accept, Mode::Dialog, Focused::Nothing), A::Focus(F::Next));
    // A text field keeps the focus (Enter would end the typing and let go of it).
    assert_eq!(plan(Nav::Accept, Mode::Dialog, Focused::Text), A::Nothing);
    // The library's own buttons do nothing in a dialog.
    for n in [Nav::Favorite, Nav::Search, Nav::Menu, Nav::Select, Nav::PageUp, Nav::JumpDown] {
        assert_eq!(plan(n, Mode::Dialog, Focused::Other), A::Nothing, "{n:?}");
    }
}

#[test]
fn menu_presses_are_egui_menu_keys() {
    use FocusDirection as F;
    use PadAct as A;
    for f in [Focused::Value, Focused::Other] {
        assert_eq!(plan(Nav::Up, Mode::Menu, f), A::Focus(F::Up));
        assert_eq!(plan(Nav::Down, Mode::Menu, f), A::Focus(F::Down));
        assert_eq!(plan(Nav::Accept, Mode::Menu, f), A::Key(Key::Enter));
    }
    assert_eq!(plan(Nav::Accept, Mode::Menu, Focused::Text), A::Nothing);
    assert_eq!(plan(Nav::Left, Mode::Menu, Focused::Other), A::Focus(F::Left));
    assert_eq!(plan(Nav::Right, Mode::Menu, Focused::Other), A::Focus(F::Right));
    assert_eq!(plan(Nav::Accept, Mode::Menu, Focused::Nothing), A::Focus(F::Down));
    // The star row of the game's menu.
    assert_eq!(plan(Nav::Left, Mode::Menu, Focused::Value), A::Step(false));
    assert_eq!(plan(Nav::Right, Mode::Menu, Focused::Value), A::Step(true));
    for n in [Nav::Favorite, Nav::Search, Nav::Menu, Nav::Select, Nav::PageDown, Nav::JumpUp] {
        assert_eq!(plan(n, Mode::Menu, Focused::Other), A::Nothing, "{n:?}");
    }
}

/// B gets out of everything (a text field, a menu, a drop-down, a dialog, a window), and only B
/// does.
#[test]
fn back_always_escapes() {
    for m in MODES {
        for f in FOCUSED {
            for n in ALL {
                assert_eq!(plan(n, m, f) == PadAct::Escape, n == Nav::Back, "{n:?} {m:?} {f:?}");
            }
        }
    }
}

/// Guide keeps its own way (App handles it before the pad's queue).
#[test]
fn guide_is_never_planned() {
    for m in MODES {
        for f in FOCUSED {
            assert_eq!(plan(Nav::Guide, m, f), PadAct::Nothing);
        }
    }
}

#[test]
fn views_cycle_and_presets_stop_at_the_ends() {
    let mut v = DesktopView::Details;
    for want in [DesktopView::Grid, DesktopView::List, DesktopView::Details] {
        v = next_view(v);
        assert_eq!(v, want);
    }
    assert_eq!(step_preset(QuickPreset::All, -1), QuickPreset::All);
    assert_eq!(step_preset(QuickPreset::All, 1), QuickPreset::Installed);
    assert_eq!(step_preset(QuickPreset::Recent, 1), QuickPreset::MostPlayed);
    assert_eq!(step_preset(QuickPreset::MostPlayed, 1), QuickPreset::MostPlayed);
    assert_eq!(step_preset(QuickPreset::Installed, -1), QuickPreset::All);
}

/// A desktop UI over a test state, run pass by pass the way show_inner runs the pad: the next
/// press first, the drawing, then the ring.
struct Rig {
    ctx: egui::Context,
    ui: DesktopUi,
    st: AppState,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let st = AppState::for_test(dir.path());
        Rig { ctx: egui::Context::default(), ui: DesktopUi::new(), st, _dir: dir }
    }

    fn pass_with(&mut self, raw: RawInput, draw: impl FnOnce(&mut DesktopUi, &mut egui::Ui)) {
        let (ui, st) = (&mut self.ui, &mut self.st);
        let mut draw = Some(draw);
        let _ = self.ctx.run_ui(raw, |eui| {
            ui.pad_pass(eui.ctx(), st);
            if let Some(d) = draw.take() {
                d(ui, eui);
            }
            ui.pad_end(eui.ctx());
        });
    }

    fn pass(&mut self, draw: impl FnOnce(&mut DesktopUi, &mut egui::Ui)) {
        self.pass_with(RawInput::default(), draw);
    }

    fn press(&mut self, nav: Nav) {
        self.ui.take_pad(&[nav]);
    }

    fn focused(&self) -> Option<Id> {
        self.ctx.memory(|m| m.focused())
    }

    /// One press, and the passes egui needs to show its effect (a focus request lands a pass
    /// later).
    fn step(&mut self, nav: Nav, mut draw: impl FnMut(&mut DesktopUi, &mut egui::Ui)) {
        self.press(nav);
        for _ in 0..3 {
            self.pass(&mut draw);
        }
    }
}

/// A dialog with three buttons over a button of the library: ↓ ↓ A presses the second, and the
/// focus never goes behind the dialog (egui's arrow search would still find that button).
#[test]
fn a_dialog_takes_the_focus_in_tab_order() {
    let mut rig = Rig::new();
    let mut ids = vec![];
    let mut clicked = vec![];
    let draw = |ui: &mut egui::Ui, ids: &mut Vec<Id>, clicked: &mut Vec<&'static str>| {
        let behind = ui.button("Behind");
        ids.push(behind.id);
        egui::Modal::new(Id::unique("pad_test_modal")).show(ui.ctx(), |ui| {
            for name in ["One", "Two", "Three"] {
                if ui.button(name).clicked() {
                    clicked.push(name);
                }
            }
        });
    };
    for _ in 0..2 {
        rig.pass(|_, ui| draw(ui, &mut ids, &mut clicked));
    }
    let behind = ids[0];
    for nav in [Nav::Down, Nav::Down, Nav::Accept] {
        rig.press(nav);
        rig.pass(|_, ui| draw(ui, &mut ids, &mut clicked));
        rig.pass(|_, ui| draw(ui, &mut ids, &mut clicked));
        assert_ne!(rig.focused(), Some(behind));
    }
    assert_eq!(clicked, ["Two"]);
    // The way up wraps around inside the dialog too.
    for _ in 0..4 {
        rig.press(Nav::Up);
        rig.pass(|_, ui| draw(ui, &mut ids, &mut clicked));
        rig.pass(|_, ui| draw(ui, &mut ids, &mut clicked));
        assert_ne!(rig.focused(), Some(behind));
    }
}

/// An open menu: ↓ focuses its first entry, B closes it.
#[test]
fn a_menu_takes_the_arrows_and_b_closes_it() {
    let mut rig = Rig::new();
    let popup = std::cell::Cell::new(None);
    let entries = std::cell::RefCell::new(vec![]);
    let draw = |_: &mut DesktopUi, ui: &mut egui::Ui| {
        let r = ui.button("Menu");
        popup.set(Some(egui::Popup::default_response_id(&r)));
        egui::Popup::menu(&r).show(|ui| {
            *entries.borrow_mut() = ["Play", "Edit", "Hide"].map(|n| ui.button(n).id).to_vec();
        });
    };
    rig.pass(draw);
    egui::Popup::open_id(&rig.ctx, popup.get().unwrap());
    for _ in 0..2 {
        rig.pass(draw);
    }
    rig.press(Nav::Down);
    for _ in 0..2 {
        rig.pass(draw);
    }
    assert_eq!(rig.focused(), entries.borrow().first().copied());
    rig.press(Nav::Down);
    for _ in 0..2 {
        rig.pass(draw);
    }
    assert_eq!(rig.focused(), entries.borrow().get(1).copied());
    rig.press(Nav::Back);
    rig.pass(draw);
    assert!(!egui::Popup::is_id_open(&rig.ctx, popup.get().unwrap()));
}

/// A long dialog: the focused widget is scrolled into view.
#[test]
fn the_focus_is_scrolled_into_view() {
    let mut rig = Rig::new();
    let offset = std::cell::Cell::new(0.0f32);
    let ids = std::cell::RefCell::new(vec![]);
    let draw = |_: &mut DesktopUi, ui: &mut egui::Ui| {
        egui::Modal::new(Id::unique("pad_test_long")).show(ui.ctx(), |ui| {
            let out = egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
                *ids.borrow_mut() = (0..50).map(|i| ui.button(format!("Button {i}")).id).collect();
            });
            offset.set(out.state.clamped_offset().y);
        });
    };
    for _ in 0..2 {
        rig.pass(draw);
    }
    for _ in 0..40 {
        rig.press(Nav::Down);
        rig.pass(draw);
    }
    rig.pass(draw);
    rig.pass(draw);
    let fortieth = ids.borrow()[39];
    assert_eq!(rig.focused(), Some(fortieth));
    let shown = offset.get();
    assert!(shown > 500.0, "scrolled down to it: {shown}");
    let r = rig.ctx.read_response(fortieth).unwrap();
    assert!(r.interact_rect.height() > 1.0, "on screen, not clipped away");
}

/// A on a text field in a dialog leaves it focused for typing; ↓ moves on past it, B leaves it.
#[test]
fn a_on_a_text_field_keeps_the_place() {
    let mut rig = Rig::new();
    let text = std::cell::RefCell::new(String::new());
    let ids = std::cell::RefCell::new(vec![]);
    let draw = |_: &mut DesktopUi, ui: &mut egui::Ui| {
        egui::Modal::new(Id::unique("pad_test_text")).show(ui.ctx(), |ui| {
            let field = ui.text_edit_singleline(&mut *text.borrow_mut());
            let after = ui.button("After");
            *ids.borrow_mut() = vec![field.id, after.id];
        });
    };
    for _ in 0..2 {
        rig.pass(draw);
    }
    rig.step(Nav::Down, draw);
    let field = ids.borrow()[0];
    assert_eq!(rig.focused(), Some(field));
    for _ in 0..2 {
        rig.step(Nav::Accept, draw);
        assert_eq!(rig.focused(), Some(field), "still in the field");
    }
    rig.step(Nav::Down, draw);
    assert_eq!(rig.focused(), Some(ids.borrow()[1]));
}

/// The Statistics page's walk: from nowhere to its first control, on in order, and past either
/// end a scroll of the page instead of leaving it.
#[test]
fn the_page_walks_its_own_controls() {
    let [a, b, c, outside] = ["a", "b", "c", "top bar"].map(Id::unique);
    let page = [a, b, c];
    assert_eq!(page_step(&page, None, false), Some(PageStep::To(a)));
    assert_eq!(page_step(&page, Some(outside), true), Some(PageStep::To(a)));
    assert_eq!(page_step(&page, Some(a), false), Some(PageStep::To(b)));
    assert_eq!(page_step(&page, Some(c), true), Some(PageStep::To(b)));
    assert_eq!(page_step(&page, Some(c), false), Some(PageStep::Scroll(false)));
    assert_eq!(page_step(&page, Some(a), true), Some(PageStep::Scroll(true)));
    assert_eq!(page_step(&[], None, false), None);
}

/// A page like the Statistics one, under a top bar: ↓ starts on the page's first control (not the
/// top bar's), walks its controls in order, scrolls the page down past the last one, and A
/// presses the focused control.
#[test]
fn a_page_takes_the_pad_without_leaving_it() {
    let mut rig = Rig::new();
    let offset = std::cell::Cell::new(0.0f32);
    let controls = std::cell::RefCell::new(vec![]);
    let top = std::cell::Cell::new(None);
    let clicked = std::cell::RefCell::new(vec![]);
    let draw = |pad: &mut DesktopUi, ui: &mut egui::Ui| {
        top.set(Some(ui.button("Top bar").id));
        let out = egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
            let mark = widget_mark(ui);
            for i in 0..6 {
                if ui.button(format!("Row {i}")).clicked() {
                    clicked.borrow_mut().push(i);
                }
                ui.add_space(40.0);
            }
            ui.label("The end of the page");
            ui.add_space(300.0);
            controls_since(ui, mark)
        });
        *controls.borrow_mut() = out.inner.clone();
        pad.pad.page_shown(out.inner);
        offset.set(out.state.clamped_offset().y);
    };
    for _ in 0..2 {
        rig.pass(draw);
    }
    assert_eq!(controls.borrow().len(), 6, "only the page's controls");
    assert!(!controls.borrow().contains(&top.get().unwrap()));
    for i in 0..6 {
        rig.step(Nav::Down, draw);
        assert_eq!(rig.focused(), Some(controls.borrow()[i]), "row {i}");
    }
    let at_last = offset.get();
    rig.step(Nav::Down, draw);
    assert_eq!(rig.focused(), Some(controls.borrow()[5]), "stays on the page");
    assert!(offset.get() > at_last, "scrolled on: {} > {at_last}", offset.get());
    rig.step(Nav::Up, draw);
    assert_eq!(rig.focused(), Some(controls.borrow()[4]));
    rig.step(Nav::Accept, draw);
    assert_eq!(*clicked.borrow(), [4]);
}

/// Presses taken just before the rule said no, and what they left to do, never act later.
#[test]
fn dropped_presses_leave_nothing_behind() {
    let mut rig = Rig::new();
    rig.ui.take_pad(&[Nav::Down, Nav::Menu]);
    rig.ui.pad.menu_for = 3;
    rig.ui.pad.open_app_menu = true;
    rig.ui.pad.focus_first = true;
    rig.ui.drop_pad();
    assert!(!rig.ui.pad_pending());
    assert_eq!(rig.ui.pad.menu_for, 0);
    assert!(!rig.ui.pad.open_app_menu && !rig.ui.pad.focus_first);
}

/// Two presses in one frame are two moves: one per pass.
#[test]
fn one_press_per_pass() {
    let mut rig = Rig::new();
    rig.ui.take_pad(&[Nav::Down, Nav::Down]);
    let mut seen = vec![];
    for _ in 0..3 {
        rig.pass(|_, ui| {
            seen.push(
                ui.input(|i| i.events.iter().filter(|e| matches!(e, Event::Key { key: Key::ArrowDown, .. })).count()),
            );
        });
    }
    assert_eq!(seen, [1, 1, 0]);
}

/// A focused slider: → changes it and keeps the focus.
#[test]
fn a_slider_moves_with_left_and_right() {
    let mut rig = Rig::new();
    let value = std::cell::Cell::new(50.0f32);
    let slider = std::cell::Cell::new(None);
    let draw = |_: &mut DesktopUi, ui: &mut egui::Ui| {
        egui::Modal::new(Id::unique("pad_test_slider")).show(ui.ctx(), |ui| {
            let mut v = value.get();
            slider.set(Some(ui.add(egui::Slider::new(&mut v, 0.0..=100.0)).id));
            value.set(v);
            let _ = ui.button("After");
        });
    };
    for _ in 0..2 {
        rig.pass(draw);
    }
    rig.press(Nav::Down);
    rig.pass(draw);
    rig.pass(draw);
    assert_eq!(rig.focused(), slider.get());
    rig.press(Nav::Right);
    rig.pass(draw);
    rig.pass(draw);
    assert!(value.get() > 50.0, "{}", value.get());
    assert_eq!(rig.focused(), slider.get());
    rig.press(Nav::Left);
    rig.press(Nav::Left);
    for _ in 0..3 {
        rig.pass(draw);
    }
    assert!(value.get() < 50.0, "{}", value.get());
}

/// The ring shows after a press, and the mouse or a real key hides it; the pad's own injected
/// keys never do.
#[test]
fn the_ring_follows_the_last_input() {
    let mut rig = Rig::new();
    rig.press(Nav::Down);
    rig.pass(|_, _| {});
    assert!(rig.ui.pad.ring);
    rig.pass(|_, _| {});
    assert!(rig.ui.pad.ring, "the injected ArrowDown is no real key");
    let moved = RawInput { events: vec![Event::PointerMoved(egui::pos2(10.0, 10.0))], ..Default::default() };
    rig.pass_with(moved, |_, _| {});
    assert!(!rig.ui.pad.ring);
    rig.press(Nav::Down);
    rig.pass(|_, _| {});
    assert!(rig.ui.pad.ring);
    let key = Event::Key { key: Key::A, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    rig.pass_with(RawInput { events: vec![key], ..Default::default() }, |_, _| {});
    assert!(!rig.ui.pad.ring);
}

/// X opens the selected game's right-click menu under the game, its first entry focused (as
/// Start's menu); B closes it.
#[test]
fn x_opens_the_games_menu() {
    let mut rig = Rig::new();
    rig.ui.sel_pos = Some(1);
    let shown = std::cell::Cell::new(false);
    let play = std::cell::Cell::new(None);
    let draw = |ui: &mut DesktopUi, eui: &mut egui::Ui| {
        shown.set(false);
        for pos in 0..3 {
            let r = eui.add(egui::Button::new(format!("Game {pos}")).sense(egui::Sense::click()));
            if ui.game_menu_popup(&r, pos).show(|ui| play.set(Some(ui.button("Play").id))).is_some() {
                shown.set(pos == 1);
            }
        }
    };
    rig.pass(draw);
    rig.press(Nav::Favorite);
    for _ in 0..3 {
        rig.pass(draw);
    }
    assert!(shown.get(), "open on the selected game");
    assert_eq!(rig.focused(), play.get());
    rig.press(Nav::Back);
    for _ in 0..2 {
        rig.pass(draw);
    }
    assert!(!shown.get());
    assert!(!egui::Popup::is_any_open(&rig.ctx));
}

/// A menu with a plain entry, a submenu and a tick box, opened from a button as the views and
/// the top bar open theirs.
#[derive(Default)]
struct TestMenu {
    ids: std::collections::HashMap<&'static str, Id>,
    clicked: Vec<&'static str>,
    ticked: bool,
}

impl TestMenu {
    fn draw(&mut self, ui: &mut egui::Ui) {
        let r = ui.button("Menu");
        self.ids.insert("Menu", r.id);
        egui::Popup::menu(&r).show(|ui| {
            self.button(ui, "Play");
            let sub = ui.menu_button("More", |ui| self.button(ui, "Inner"));
            self.ids.insert("More", sub.response.id);
            let t = ui.checkbox(&mut self.ticked, "Tick");
            self.note(t, "Tick");
        });
    }

    fn button(&mut self, ui: &mut egui::Ui, name: &'static str) {
        let r = ui.button(name);
        self.note(r, name);
    }

    fn note(&mut self, r: egui::Response, name: &'static str) {
        self.ids.insert(name, r.id);
        if r.clicked() {
            self.clicked.push(name);
        }
    }
}

/// The test menu, opened.
fn open_test_menu(rig: &mut Rig, menu: &std::cell::RefCell<TestMenu>) {
    let draw = |_: &mut DesktopUi, ui: &mut egui::Ui| menu.borrow_mut().draw(ui);
    rig.pass(draw);
    let id = menu.borrow().ids["Menu"];
    egui::Popup::open_id(&rig.ctx, id.with("popup"));
    rig.pass(draw);
    rig.pass(draw);
}

/// A on an entry runs it and closes the menu: egui closes menus on pointer clicks only, so a
/// menu left open would keep every later press (Edit…'s dialog would get none).
#[test]
fn a_on_an_entry_closes_the_menu() {
    let mut rig = Rig::new();
    let menu = std::cell::RefCell::new(TestMenu::default());
    let draw = |_: &mut DesktopUi, ui: &mut egui::Ui| menu.borrow_mut().draw(ui);
    let id = |name| menu.borrow().ids[name];
    open_test_menu(&mut rig, &menu);
    rig.step(Nav::Down, draw);
    assert_eq!(rig.focused(), Some(id("Play")));
    rig.step(Nav::Accept, draw);
    assert_eq!(menu.borrow().clicked, ["Play"]);
    assert!(!egui::Popup::is_any_open(&rig.ctx));
}

/// → on a submenu's row opens it with its first entry focused, A does the same, ← goes back,
/// and A inside runs the entry and closes the whole menu.
#[test]
fn submenus_open_with_right_or_a() {
    let mut rig = Rig::new();
    let menu = std::cell::RefCell::new(TestMenu::default());
    let draw = |_: &mut DesktopUi, ui: &mut egui::Ui| menu.borrow_mut().draw(ui);
    let id = |name| menu.borrow().ids[name];
    open_test_menu(&mut rig, &menu);
    rig.step(Nav::Down, draw);
    rig.step(Nav::Down, draw);
    assert_eq!(rig.focused(), Some(id("More")));
    rig.step(Nav::Right, draw);
    assert_eq!(rig.focused(), Some(id("Inner")));
    rig.step(Nav::Left, draw);
    assert_eq!(rig.focused(), Some(id("More")));
    rig.step(Nav::Accept, draw);
    assert_eq!(rig.focused(), Some(id("Inner")), "A opens the submenu, it doesn't close it");
    assert!(egui::Popup::is_any_open(&rig.ctx));
    rig.step(Nav::Accept, draw);
    assert_eq!(menu.borrow().clicked, ["Inner"]);
    assert!(!egui::Popup::is_any_open(&rig.ctx));
}

/// A tick box in a menu stays open after A, so several can be ticked in one go.
#[test]
fn a_on_a_tick_box_keeps_the_menu() {
    let mut rig = Rig::new();
    let menu = std::cell::RefCell::new(TestMenu::default());
    let draw = |_: &mut DesktopUi, ui: &mut egui::Ui| menu.borrow_mut().draw(ui);
    open_test_menu(&mut rig, &menu);
    rig.step(Nav::Up, draw);
    assert_eq!(rig.focused(), Some(menu.borrow().ids["Tick"]));
    rig.step(Nav::Accept, draw);
    assert_eq!(menu.borrow().clicked, ["Tick"]);
    assert!(menu.borrow().ticked);
    assert!(egui::Popup::is_any_open(&rig.ctx));
}

/// Moving through a menu doesn't scroll the list it hangs from (the entries are in plain view).
#[test]
fn a_menu_leaves_the_list_behind_it_alone() {
    let mut rig = Rig::new();
    let offset = std::cell::Cell::new(0.0f32);
    let anchor = std::cell::Cell::new(None);
    let draw = |_: &mut DesktopUi, ui: &mut egui::Ui| {
        let out = egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
            for i in 0..40 {
                let r = ui.button(format!("Game {i}"));
                if i == 3 {
                    anchor.set(Some(r.id));
                    egui::Popup::menu(&r).show(|ui| {
                        for n in 0..12 {
                            let _ = ui.button(format!("Entry {n}"));
                        }
                    });
                }
            }
        });
        offset.set(out.state.clamped_offset().y);
    };
    rig.pass(draw);
    egui::Popup::open_id(&rig.ctx, anchor.get().unwrap().with("popup"));
    rig.pass(draw);
    for _ in 0..8 {
        rig.step(Nav::Down, draw);
    }
    assert!(rig.focused().is_some());
    assert_eq!(offset.get(), 0.0);
}

/// A drop-down in a dialog: A on a choice picks it, and B closes it; either way the focus is back
/// on the drop-down, so ↓ goes on from there instead of from the top of the dialog.
#[test]
fn a_drop_down_hands_the_focus_back() {
    let mut rig = Rig::new();
    let choice = std::cell::Cell::new(0);
    let combo = std::cell::Cell::new(None);
    let draw = |_: &mut DesktopUi, ui: &mut egui::Ui| {
        egui::Modal::new(Id::unique("pad_test_combo")).show(ui.ctx(), |ui| {
            let _ = ui.button("First");
            let mut c = choice.get();
            let r = egui::ComboBox::from_id_salt("pad_test_choice").selected_text(format!("Choice {c}")).show_ui(
                ui,
                |ui| {
                    for i in 0..3 {
                        ui.selectable_value(&mut c, i, format!("Choice {i}"));
                    }
                },
            );
            combo.set(Some(r.response.id));
            choice.set(c);
            let _ = ui.button("After");
        });
    };
    for _ in 0..2 {
        rig.pass(draw);
    }
    for nav in [Nav::Down, Nav::Down] {
        rig.step(nav, draw);
    }
    assert_eq!(rig.focused(), combo.get());
    for nav in [Nav::Accept, Nav::Down, Nav::Down] {
        rig.step(nav, draw);
    }
    assert!(egui::Popup::is_any_open(&rig.ctx));
    rig.step(Nav::Back, draw);
    assert!(!egui::Popup::is_any_open(&rig.ctx));
    assert_eq!(rig.focused(), combo.get(), "B");
    assert_eq!(choice.get(), 0);
    for nav in [Nav::Accept, Nav::Down, Nav::Down, Nav::Accept] {
        rig.step(nav, draw);
    }
    assert!(!egui::Popup::is_any_open(&rig.ctx));
    assert_eq!(choice.get(), 1);
    assert_eq!(rig.focused(), combo.get(), "A");
}

/// The game menu's star row: ← → light the stars, A sets the score and closes the menu.
#[test]
fn the_star_row_takes_left_and_right() {
    let mut rig = Rig::new();
    let actions = std::cell::RefCell::new(vec![]);
    let anchor = std::cell::Cell::new(None);
    let pal = crate::ui::desktop::Pal::default();
    let draw = |_: &mut DesktopUi, ui: &mut egui::Ui| {
        let r = ui.button("Game");
        anchor.set(Some(r.id));
        egui::Popup::menu(&r).show(|ui| {
            super::super::score::score_row(ui, Some(Some(20)), &[7], &pal, &mut actions.borrow_mut());
        });
    };
    rig.pass(draw);
    egui::Popup::open_id(&rig.ctx, anchor.get().unwrap().with("popup"));
    rig.pass(draw);
    rig.pass(draw);
    for nav in [Nav::Down, Nav::Right, Nav::Right, Nav::Left, Nav::Right] {
        rig.step(nav, draw);
    }
    assert!(actions.borrow().is_empty(), "nothing set before A");
    rig.step(Nav::Accept, draw);
    assert_eq!(*actions.borrow(), [crate::ui::desktop::actions::Action::SetScore(vec![7], Some(60))]);
    assert!(!egui::Popup::is_any_open(&rig.ctx));
}
