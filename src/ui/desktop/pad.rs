//! The controller in desktop mode. The App hands the pad's presses over only while the rule in
//! input/gate.rs (`desktop_pad`) allows it; here each press becomes what the keyboard would do in
//! the same place, so the library, its menus and the dialogs need no code of their own:
//!
//! * the library: the arrow keys, Enter, Esc and Page Up / Down as keys.rs and explorer/nav.rs
//!   read them; X opens the selected game's right-click menu, Y the search box, Start the app
//!   menu, View the next view, LB / RB the quick filters.
//! * menus, drop-downs and pickers ("Choose what to play"): egui's own arrow keys; A on an
//!   entry also closes the menu, as a click does (egui closes menus on pointer clicks only).
//! * dialogs and their windows: ↑ ↓ ← → in Tab order (or adjusting a focused slider), Enter
//!   presses the focused widget (a text field keeps the focus for typing), Esc closes.
//! * the Statistics page: like a dialog, through the page's own controls only; at either end
//!   ↑ ↓ scroll the page instead.
//!
//! B is Esc everywhere, so it always gets out. A focus ring marks the focused widget while the
//! pad was used last (Lodestar's own widgets paint no focus look); the mouse or the keyboard
//! hide it.
//!
//! Presses go into the pass before anything is drawn, as the screenshot hook's keys do
//! (debug_input.rs), one per pass: two arrow presses in one pass would be one move. Tab order is
//! used in dialogs because egui's arrow search still knows the library's widgets drawn behind a
//! dialog and could move the focus there; Tab order only visits widgets that take input.

use super::DesktopUi;
use crate::app::AppState;
use crate::config::{DesktopView, QuickPreset};
use crate::input::Nav;
use egui::containers::menu::{MenuState, SubMenu};
use egui::{Event, FocusDirection, Id, Key, Modifiers, Role, accesskit};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Where a press lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Mode {
    Library,
    /// A dialog (in the main window or in its own).
    Dialog,
    /// A menu, a drop-down or a picker drawn without a dialog around it ("Choose what to play"):
    /// egui's arrow-key focus moves inside it.
    Menu,
}

/// The widget with the keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Focused {
    Nothing,
    /// A slider, a number box or the menus' star row: ← → change it.
    Value,
    /// A text field: A leaves it focused (Enter would end the typing and drop the focus).
    Text,
    Other,
}

impl Focused {
    fn of(role: Option<Role>) -> Focused {
        match role {
            Some(Role::Slider | Role::SpinButton) => Focused::Value,
            _ => Focused::Other,
        }
    }
}

/// What a press does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PadAct {
    Key(Key),
    /// Esc, and the focused widget lets go of the focus (a real Esc does that before the pass).
    Escape,
    Focus(FocusDirection),
    /// One step up / down on the focused slider or number box.
    Step(bool),
    /// The selected game's right-click menu.
    GameMenu,
    AppMenu,
    Search,
    NextView,
    /// Previous (-1) / next (+1) quick filter.
    Preset(i8),
    Nothing,
}

/// What `nav` does in `mode` with `focused` focused.
pub(super) fn plan(nav: Nav, mode: Mode, focused: Focused) -> PadAct {
    use FocusDirection as F;
    if nav == Nav::Back {
        return PadAct::Escape;
    }
    match mode {
        Mode::Library => match nav {
            Nav::Up => PadAct::Key(Key::ArrowUp),
            Nav::Down => PadAct::Key(Key::ArrowDown),
            Nav::Left => PadAct::Key(Key::ArrowLeft),
            Nav::Right => PadAct::Key(Key::ArrowRight),
            Nav::Accept => PadAct::Key(Key::Enter),
            Nav::Favorite => PadAct::GameMenu,
            Nav::Search => PadAct::Search,
            Nav::Menu => PadAct::AppMenu,
            Nav::Select => PadAct::NextView,
            Nav::PageUp => PadAct::Preset(-1),
            Nav::PageDown => PadAct::Preset(1),
            Nav::JumpUp => PadAct::Key(Key::PageUp),
            Nav::JumpDown => PadAct::Key(Key::PageDown),
            _ => PadAct::Nothing,
        },
        Mode::Menu => match (nav, focused) {
            (Nav::Left, Focused::Value) => PadAct::Step(false),
            (Nav::Right, Focused::Value) => PadAct::Step(true),
            (Nav::Up, _) => PadAct::Focus(F::Up),
            (Nav::Down, _) | (Nav::Accept, Focused::Nothing) => PadAct::Focus(F::Down),
            (Nav::Left, _) => PadAct::Focus(F::Left),
            (Nav::Right, _) => PadAct::Focus(F::Right),
            (Nav::Accept, Focused::Text) => PadAct::Nothing,
            (Nav::Accept, _) => PadAct::Key(Key::Enter),
            _ => PadAct::Nothing,
        },
        Mode::Dialog => match (nav, focused) {
            (Nav::Left, Focused::Value) => PadAct::Step(false),
            (Nav::Right, Focused::Value) => PadAct::Step(true),
            (Nav::Up | Nav::Left, _) => PadAct::Focus(F::Previous),
            // A with nothing focused first shows where you are.
            (Nav::Down | Nav::Right, _) | (Nav::Accept, Focused::Nothing) => PadAct::Focus(F::Next),
            (Nav::Accept, Focused::Text) => PadAct::Nothing,
            (Nav::Accept, _) => PadAct::Key(Key::Enter),
            _ => PadAct::Nothing,
        },
    }
}

/// Where ↑ ↓ ← → go on the Statistics page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PageStep {
    /// Focus this control.
    To(Id),
    /// Past the first (true) or last control: scroll the page up / down.
    Scroll(bool),
}

/// The control after (`back`: before) `focused` among the page's `controls`; the first one when
/// the focus isn't on the page (nothing focused, or the top bar).
pub(super) fn page_step(controls: &[Id], focused: Option<Id>, back: bool) -> Option<PageStep> {
    let Some(i) = focused.and_then(|id| controls.iter().position(|c| *c == id)) else {
        return controls.first().map(|&id| PageStep::To(id));
    };
    let next = if back { i.checked_sub(1) } else { Some(i + 1) };
    Some(match next.and_then(|j| controls.get(j)) {
        Some(&id) => PageStep::To(id),
        None => PageStep::Scroll(back),
    })
}

/// How far this pass's drawing on the layer of `ui` has got (see `controls_since`).
pub(super) fn widget_mark(ui: &egui::Ui) -> usize {
    ui.ctx().viewport(|v| v.this_pass.widgets.get_layer(ui.layer_id()).count())
}

/// The controls drawn on the layer of `ui` since `mark` (enabled ones that take the focus), in
/// drawing order, which is egui's Tab order.
pub(super) fn controls_since(ui: &egui::Ui, mark: usize) -> Vec<Id> {
    ui.ctx().viewport(|v| {
        let widgets = v.this_pass.widgets.get_layer(ui.layer_id()).skip(mark);
        widgets.filter(|w| w.enabled && w.sense.is_focusable()).map(|w| w.id).collect()
    })
}

/// Details → Grid → List → Details.
pub(super) fn next_view(v: DesktopView) -> DesktopView {
    match v {
        DesktopView::Details => DesktopView::Grid,
        DesktopView::Grid => DesktopView::List,
        DesktopView::List => DesktopView::Details,
    }
}

/// The quick filter `by` steps from `p`, stopping at the ends.
pub(super) fn step_preset(p: QuickPreset, by: i8) -> QuickPreset {
    let all = QuickPreset::ALL;
    let i = all.iter().position(|x| *x == p).unwrap_or(0) as isize + isize::from(by);
    all[i.clamp(0, all.len() as isize - 1) as usize]
}

/// Mouse or keyboard input among a pass's raw events (the pad's are injected later, never raw).
fn real_input(raw: &[Event]) -> bool {
    raw.iter().any(|e| {
        matches!(e, Event::PointerMoved(_) | Event::PointerButton { .. } | Event::MouseWheel { .. } | Event::Key { .. })
    })
}

fn push_key(ctx: &egui::Context, key: Key) {
    ctx.input_mut(|i| {
        i.events.push(Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE })
    });
}

/// Ask the widget `id` for `action` as a screen reader would (egui's sliders, number boxes and
/// scroll areas answer these).
fn push_action(ctx: &egui::Context, id: Id, action: accesskit::Action) {
    let request = accesskit::ActionRequest {
        action,
        target_node: id.accesskit_id(),
        target_tree: accesskit::TreeId::ROOT,
        data: None,
    };
    ctx.input_mut(|i| i.events.push(Event::AccessKitActionRequest(request)));
}

/// egui keeps a menu state for every open popup and submenu, under the id of its layer.
fn has_menu_state(ctx: &egui::Context, id: Id) -> bool {
    ctx.data(|d| d.get_temp::<MenuState>(id.with(MenuState::ID)).is_some())
}

fn is_text_field(ctx: &egui::Context, id: Id) -> bool {
    egui::text_edit::TextEditState::load(ctx, id).is_some()
}

/// A on the menu entry `id` closes the menu, as a click on it does. Not on a submenu's row (it
/// opens the submenu), a tick box (several can be ticked in one go), a text field or a value.
fn accept_closes(ctx: &egui::Context, id: Id, role: Option<Role>) -> bool {
    let in_menu = ctx.read_response(id).is_some_and(|r| has_menu_state(ctx, r.layer_id.id));
    let keeps = matches!(role, Some(Role::CheckBox | Role::Slider | Role::SpinButton)) || is_text_field(ctx, id);
    in_menu && !keeps && !has_menu_state(ctx, SubMenu::id_from_widget_id(id))
}

/// Feed `act` to egui (false: one of the library's own, see `DesktopUi::pad_act`).
fn send(ctx: &egui::Context, act: PadAct) -> bool {
    let focused = ctx.memory(|m| m.focused());
    match act {
        PadAct::Key(k) => push_key(ctx, k),
        PadAct::Escape => {
            push_key(ctx, Key::Escape);
            if let Some(id) = focused {
                ctx.memory_mut(|m| m.surrender_focus(id));
            }
        }
        // Next / Previous resolve while this pass draws; the arrows inside egui's menus.
        PadAct::Focus(d) => ctx.memory_mut(|m| m.move_focus(d)),
        PadAct::Step(up) => {
            if let Some(id) = focused {
                push_action(ctx, id, if up { accesskit::Action::Increment } else { accesskit::Action::Decrement });
            }
        }
        _ => return false,
    }
    true
}

/// The screenshot hook's made-up pad (LODESTAR_DESKTOP_KEYS `pad:`, `game:`, `focus:`, `seen:`,
/// `wait:`, see debug_input.rs).
#[derive(Clone, Copy, Debug)]
pub(crate) struct MadeUp {
    pub game_up: bool,
    pub focused: bool,
    /// The look for running games saw a game, and when it started (`seen:` is a look; the
    /// first token stands for one taken then).
    pub seen: bool,
    pub looked_at: Instant,
}

impl Default for MadeUp {
    fn default() -> Self {
        // Xvfb has no window manager to hand out the focus: focused unless told otherwise.
        MadeUp { game_up: false, focused: true, seen: false, looked_at: Instant::now() }
    }
}

#[derive(Default)]
pub(crate) struct PadState {
    queue: VecDeque<Nav>,
    /// The pad was used last: the focused widget gets the ring.
    ring: bool,
    /// The widget that got the focus last, and what it is.
    gained: Option<(Id, Role)>,
    /// What had the focus before a menu or a drop-down opened: B goes back there.
    before_menu: Option<Id>,
    /// A or B left a menu or a drop-down: close it once its entries have seen the press.
    leave_menu: bool,
    /// The focus moved: scroll the newly focused widget into view next pass.
    reveal: bool,
    /// X was pressed: open the selected game's menu when the game is drawn, within this many
    /// passes.
    menu_for: u8,
    /// The game menu X opened (it hangs under the game, not at the pointer).
    menu_on: Option<Id>,
    pub(super) open_app_menu: bool,
    /// Start or X opened a menu: focus its first entry once it shows.
    focus_first: bool,
    /// The Statistics page's controls as last drawn (empty: it isn't shown), see `page_step`.
    page: Vec<Id>,
    /// The screenshot hook: presses waiting for App to run them through the rule, the situation
    /// it makes up (once it was used), and the end of a `wait:`.
    script: Vec<Nav>,
    made_up: Option<MadeUp>,
    wait_until: Option<Instant>,
}

impl PadState {
    fn role(&self, id: Id) -> Option<Role> {
        self.gained.filter(|(g, _)| *g == id).map(|(_, r)| r)
    }

    fn focused(&self, ctx: &egui::Context, id: Option<Id>) -> Focused {
        match id {
            None => Focused::Nothing,
            Some(id) if is_text_field(ctx, id) => Focused::Text,
            Some(id) => Focused::of(self.role(id)),
        }
    }

    /// Mouse or keyboard input hides the ring.
    fn note_input(&mut self, raw: &[Event]) {
        if real_input(raw) {
            self.ring = false;
        }
    }

    /// The Statistics page was drawn with these controls (`controls_since`).
    pub(super) fn page_shown(&mut self, controls: Vec<Id>) {
        self.page = controls;
    }
}

impl DesktopUi {
    /// Presses the pad rule let through this frame (input/gate.rs `desktop_pad`).
    pub(crate) fn take_pad(&mut self, navs: &[Nav]) {
        self.pad.queue.extend(navs.iter().filter(|n| **n != Nav::Guide));
    }

    /// The rule says no this frame: presses still waiting are dropped, not kept for later, and
    /// so is what the last one left to do (open a menu once it shows).
    pub(crate) fn drop_pad(&mut self) {
        let p = &mut self.pad;
        p.queue.clear();
        p.menu_for = 0;
        p.open_app_menu = false;
        p.focus_first = false;
    }

    /// Screenshot runs: the hook's pad presses since the last call, and its made-up situation.
    pub(crate) fn scripted_pad(&mut self) -> (Vec<Nav>, MadeUp) {
        (std::mem::take(&mut self.pad.script), self.pad.made_up.unwrap_or_default())
    }

    /// Screenshot runs driving the pad: whether the window counts as focused. egui only lets a
    /// focused window's widgets act on the arrow keys (submenus, the search box), and Xvfb has
    /// no window manager to focus it.
    pub(crate) fn scripted_focus(&self) -> Option<bool> {
        self.pad.made_up.map(|m| m.focused)
    }

    /// One of the hook's pad tokens (debug_input.rs); false when `k` isn't one.
    pub(super) fn pad_script_key(&mut self, st: &mut AppState, k: &str) -> bool {
        let p = &mut self.pad;
        if let Some(ms) = k.strip_prefix("wait:") {
            p.wait_until = ms.parse().ok().map(|ms| Instant::now() + Duration::from_millis(ms));
            return true;
        }
        let Some((what, how)) = k.split_once(':') else { return false };
        if !matches!(what, "pad" | "game" | "focus" | "seen") {
            return false;
        }
        let made_up = p.made_up.get_or_insert_default();
        match (what, how) {
            ("pad", names) => p.script.extend(crate::cli::parse_navs(names)),
            ("game", "up") => made_up.game_up = true,
            ("game", "down") => {
                made_up.game_up = false;
                st.game_ended_at = Some(Instant::now());
            }
            ("focus", "on" | "off") => made_up.focused = how == "on",
            ("seen", "on" | "off") => {
                made_up.seen = how == "on";
                made_up.looked_at = Instant::now();
            }
            _ => return false,
        }
        true
    }

    /// The hook's `wait:` hasn't run out yet.
    pub(super) fn pad_script_waits(&mut self, ctx: &egui::Context) -> bool {
        match self.pad.wait_until {
            Some(t) if Instant::now() < t => {
                ctx.request_repaint_after(t.saturating_duration_since(Instant::now()));
                true
            }
            _ => {
                self.pad.wait_until = None;
                false
            }
        }
    }

    pub(super) fn pad_pending(&self) -> bool {
        !self.pad.queue.is_empty() || !self.pad.script.is_empty()
    }

    /// At the start of the pass of the window that gets the pad: apply the next press.
    pub(super) fn pad_pass(&mut self, ctx: &egui::Context, st: &mut AppState) {
        ctx.input(|i| self.pad.note_input(&i.raw.events));
        let page = std::mem::take(&mut self.pad.page);
        let focused = ctx.memory(|m| m.focused());
        let menu_open = egui::Popup::is_any_open(ctx);
        if !menu_open {
            self.pad.before_menu = focused;
        }
        // egui doesn't scroll to a widget that got the focus from the keyboard. In a menu only
        // when the entry is cut off: the request goes to whichever scroll area ends next, and
        // most menus have none of their own, so the list behind the menu would scroll.
        if std::mem::take(&mut self.pad.reveal)
            && let Some(r) = focused.and_then(|id| ctx.read_response(id))
            && !(menu_open && r.interact_rect.contains_rect(r.rect))
        {
            push_action(ctx, r.id, accesskit::Action::ScrollIntoView);
        }
        if std::mem::take(&mut self.pad.focus_first) && menu_open {
            ctx.memory_mut(|m| m.move_focus(FocusDirection::Down));
            ctx.request_repaint();
            return;
        }
        let Some(nav) = self.pad.queue.pop_front() else { return };
        if !self.pad.queue.is_empty() {
            ctx.request_repaint();
        }
        // The welcome step counts as a dialog from its first pass.
        let dialog =
            ctx.memory(|m| m.top_modal_layer().is_some()) || super::own_window::in_own_window(ctx) || st.welcome.open;
        // Tab order only stays inside a modal dialog: a picker over the library (or a dialog
        // not drawn yet) gets the arrows the keyboard would give it. The Statistics page is
        // moved around like a dialog, through its own controls.
        let mode = if menu_open {
            Mode::Menu
        } else if dialog {
            Mode::Dialog
        } else if self.keyboard_taken(ctx, st) {
            Mode::Menu
        } else if !page.is_empty() {
            Mode::Dialog
        } else {
            Mode::Library
        };
        let act = plan(nav, mode, self.pad.focused(ctx, focused));
        if act == PadAct::Nothing {
            return;
        }
        self.pad.ring = true;
        ctx.request_repaint();
        // The Statistics page: its own controls in order (Tab order would go on into the top
        // bar and the sidebar), and the page scrolls past its ends.
        if mode == Mode::Dialog
            && !dialog
            && let PadAct::Focus(d) = act
        {
            match page_step(&page, focused, d == FocusDirection::Previous) {
                Some(PageStep::To(id)) => {
                    ctx.memory_mut(|m| m.request_focus(id));
                    self.pad.reveal = true;
                }
                Some(PageStep::Scroll(up)) => {
                    let scroll = if up { accesskit::Action::ScrollUp } else { accesskit::Action::ScrollDown };
                    if let Some(id) = focused {
                        push_action(ctx, id, scroll);
                    }
                }
                None => {}
            }
            return;
        }
        if menu_open {
            self.pad.leave_menu = match (act, focused) {
                (PadAct::Escape, _) => true,
                (PadAct::Key(Key::Enter), Some(id)) => accept_closes(ctx, id, self.pad.role(id)),
                _ => false,
            };
        }
        if !send(ctx, act) {
            self.pad_act(st, act);
        }
        self.pad.reveal = matches!(act, PadAct::Focus(_));
    }

    /// The library's own presses.
    fn pad_act(&mut self, st: &mut AppState, act: PadAct) {
        let d = &mut st.config.desktop;
        match act {
            PadAct::GameMenu => {
                self.pad.menu_for = 3;
                self.scroll_to_sel = true;
            }
            PadAct::AppMenu => {
                self.pad.open_app_menu = true;
                self.pad.focus_first = true;
            }
            PadAct::Search => self.focus_search = true,
            PadAct::NextView => {
                d.view = next_view(d.view);
                self.scroll_to_sel = true;
                self.mark_config_dirty();
            }
            PadAct::Preset(by) => {
                let p = step_preset(d.preset, by);
                if p != d.preset {
                    self.set_preset(d, p);
                }
            }
            _ => {}
        }
    }

    /// After the drawing of the window that got the pad: close a menu left with A or B, note
    /// what has the focus, paint the ring.
    pub(super) fn pad_end(&mut self, ctx: &egui::Context) {
        if std::mem::take(&mut self.pad.leave_menu) {
            egui::Popup::close_all(ctx);
            // Back on the drop-down's button, not at the top of a long dialog.
            if let Some(id) = self.pad.before_menu {
                ctx.memory_mut(|m| m.request_focus(id));
            }
        }
        let focused = ctx.memory(|m| m.focused());
        let gained = ctx.output(|o| {
            o.events.iter().rev().find_map(|e| match e {
                egui::output::OutputEvent::FocusGained(info) => Some(info.role),
                _ => None,
            })
        });
        if let (Some(id), Some(role)) = (focused, gained) {
            self.pad.gained = Some((id, role));
        }
        self.pad.menu_for = self.pad.menu_for.saturating_sub(1);
        if self.pad.menu_on.is_some_and(|id| !egui::Popup::is_id_open(ctx, id)) {
            self.pad.menu_on = None;
        }
        if !self.pad.ring {
            return;
        }
        // The clipped rect: nothing shows for a widget scrolled out of sight. Text fields show
        // their own focus (their frame, the cursor): a ring would double it.
        if let Some(r) = focused.filter(|id| !is_text_field(ctx, *id)).and_then(|id| ctx.read_response(id)) {
            let layer = egui::LayerId::new(egui::Order::Tooltip, Id::unique("ls_pad_ring"));
            ctx.layer_painter(layer).rect_stroke(
                r.interact_rect.expand(3.0),
                6.0,
                egui::Stroke::new(2.5, self.pal.accent),
                egui::StrokeKind::Outside,
            );
        }
    }

    /// The game's right-click menu for a game in a view (at `pos` in the shown list): opened
    /// by the mouse as before, or by X on the selected game, then hanging under the game.
    pub(super) fn game_menu_popup<'a>(&mut self, resp: &'a egui::Response, pos: usize) -> egui::Popup<'a> {
        let id = egui::Popup::default_response_id(resp);
        if self.pad.menu_for > 0 && self.sel_pos == Some(pos) {
            self.pad.menu_for = 0;
            self.pad.menu_on = Some(id);
            self.pad.focus_first = true;
            egui::Popup::open_id(&resp.ctx, id);
        }
        // A context menu opens at the pointer, which the pad doesn't move.
        if self.pad.menu_on == Some(id) && egui::Popup::is_id_open(&resp.ctx, id) {
            egui::Popup::menu(resp)
        } else {
            egui::Popup::context_menu(resp)
        }
    }
}

#[cfg(test)]
mod tests;
