//! The menu button (Home / Guide / PS and/or a held button combination, input/pad.rs) and
//! what it brings up, in both modes:
//! * the IN-GAME MENU while a game runs: the game and its session time, Resume (back to the
//!   game's window), Screenshot (spectacle / grim / gnome-screenshot, when one is installed),
//!   Volume (wpctl / pactl, when installed), Library (browse while the game runs; the menu button or Back
//!   on the first screen returns), Quit game (asks first; asked to close politely, forced if it
//!   ignores that). Controller mode draws it over the library, desktop mode as a centred panel.
//! * "WHICH BUTTON OPENS LODESTAR'S MENU?", once, on the first press: a button combination
//!   (default L3 + R3, preselected; "Change combination" picks a preset or records one, see
//!   combo.rs) or the Home button. Each choice explains itself while selected or hovered (Steam
//!   and some emulators react to Home too, and where Steam's use of it is turned off). Answered
//!   once; Back puts it off (Home keeps working, asked again in a later session); Settings →
//!   Input changes it later.
//! * the QUICK MENU in controller mode with no game running: the menu button opens it and
//!   closes it again, whichever button was chosen (or while the question is put off). Which
//!   presses count at all is decided in the pad thread (input/pad.rs); the gate
//!   (input/gate.rs) is the same either way.
//!
//! App calls `on_guide` for every menu button press that counts (debounced; after it summoned a
//! window that wasn't focused, `summoned`), `wants_full_input` decides with input/gate.rs what
//! reaches Lodestar while a game runs, and `sys_nav` / `draw_sys` / `desktop_overlay` drive it.
//! Navigation reaches these menus only while Lodestar is focused (the gate's safety rule).

use super::ingame_sys::{self as sys, Mixer, ShotTool, VolOp};
use super::*;
use crate::config::MenuButton;
use parking_lot::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// A game asked to quit that still runs this long later is stopped for good.
const FORCE_AFTER_SECS: f64 = 8.0;
/// Resume first, then the screenshot: time for the game's window to come back.
const SHOT_DELAY: std::time::Duration = std::time::Duration::from_millis(900);

#[derive(Default)]
pub(super) struct SysState {
    /// In-game menu (+ its "Quit?" confirmation) and the menu button screens on top: centred
    /// panels.
    pub menus: Vec<Menu>,
    /// "Which button opens Lodestar's menu?" was put off with Back: not asked again this
    /// session (Home keeps working until it's answered).
    pub menu_question_put_off: bool,
    /// Library chosen in the in-game menu: browsing while the game runs.
    pub library: bool,
    /// The focus went back to the game while the menu button screens showed: they stay, but
    /// until the menu button is pressed again they ignore the controller entirely and "Record a
    /// combination" stops recording (a focus Lodestar got on its own mustn't let the game's
    /// presses answer the question, pick a combination or become one).
    pub(super) left_for_game: bool,
    /// "Record a combination": the take, once the buttons were let go (None: still waiting).
    pub take: Option<crate::input::Recorded>,
    /// The pad is recording for that screen (stopped when it closes).
    pub recording: bool,
    /// A game asked to quit (and when, ui time): forced after FORCE_AFTER_SECS.
    quitting: Option<(GameId, f64)>,
    /// Volume as last read ((percent, muted)); written by the volume thread.
    volume: Arc<Mutex<Option<(u32, bool)>>>,
    /// Detected once: Some(None) = looked, none there.
    mixer: Option<Option<Mixer>>,
    shot: Option<Option<ShotTool>>,
    /// Resume found no game window and minimised Lodestar instead: show it again when the
    /// game ends (set from the activation thread).
    resumed_minimised: Arc<AtomicBool>,
    pub cursor: settings::CursorHider,
    battery: Option<(f64, Option<(u8, bool)>)>,
}

impl ControllerUi {
    /// Lodestar's own menus are open while a game runs (or the user browses from the in-game
    /// menu): the gate then lets all of the controller through to a focused Lodestar.
    pub fn wants_full_input(&self) -> bool {
        (!self.sys.menus.is_empty() && !self.sys.left_for_game) || self.sys.library
    }

    /// A menu button press that counts (see the module docs). `summoned`: Lodestar wasn't
    /// focused and was just brought to the front for it.
    pub fn on_guide(&mut self, st: &mut AppState, summoned: bool, now: f64) {
        self.sys.left_for_game = false;
        if self.top_sys().is_some_and(|k| on_top(&k)) {
            // Pressed again while the question (or its combination screens) shows: puts it off
            // like Back (nothing decided). Just brought to the front: they show.
            if !summoned {
                self.put_off_menu_button_question(st);
            }
            return;
        }
        let asked_now = self.maybe_ask_menu_button(st, now);
        let running = crate::ui::launching::running(st);
        let c = &st.config.controller;
        let (lodestar, desktop_switch) = (c.guide_opens_lodestar, c.guide_opens_controller_mode);
        if let Some(id) = running.filter(|_| lodestar) {
            if asked_now {
                // The question sits on top; the in-game menu waits under it.
                self.open_ingame(st, id, now);
                self.raise_menu_button_screens();
                return;
            }
            if self.sys.library {
                // Back from browsing to the in-game menu.
                self.sys.library = false;
                self.open_ingame(st, id, now);
            } else if self.sys.menus.iter().any(|m| m.kind == MenuKind::InGame) && !summoned {
                // The menu button again: back to the game.
                self.resume(st);
            } else {
                self.open_ingame(st, id, now);
            }
            return;
        }
        if asked_now || summoned {
            return;
        }
        match st.mode {
            // In controller mode the menu button is Lodestar's menu button whatever was chosen
            // (or put off): it opens the quick menu and closes it again. The switch below is
            // only about desktop mode, where the same button would otherwise reach nothing.
            Mode::Controller => {
                if self.menus.is_empty() {
                    self.open_quick_menu(st, now);
                } else {
                    self.close_menus(st);
                }
            }
            Mode::Desktop if desktop_switch => st.switch_mode(Mode::Controller),
            Mode::Desktop => {}
        }
    }

    fn top_sys(&self) -> Option<MenuKind> {
        self.sys.menus.last().map(|m| m.kind.clone())
    }

    // ------------------------------------------------------------------ "Which button opens Lodestar's menu?"

    /// Ask "Which button opens Lodestar's menu?" if it was never answered.
    fn maybe_ask_menu_button(&mut self, st: &AppState, now: f64) -> bool {
        if st.config.controller.menu_button_asked || self.sys.menu_question_put_off {
            return false;
        }
        let c = &st.config.controller;
        // Back (or the menu button again) only puts the question off: the hint says so.
        let mut m = Menu::new(
            MenuKind::MenuButtonAsk,
            "Which button opens Lodestar's menu?",
            Self::menu_button_question(c),
            now,
        )
        .with_subtitle("Or decide later: Lodestar asks again next time.")
        .with_hints(vec![Hint::new(Button::A, "Select"), Hint::new(Button::B, "Decide later")])
        .centred();
        m.select_act(&Act::MenuButtonPick(c.menu_button));
        self.sys.menus.push(m);
        true
    }

    pub(super) fn menu_button_question(c: &crate::config::ControllerConfig) -> Vec<Item> {
        let combo = c.menu_combo;
        vec![
            Item::action(format!("A button combination ({})", combo.label()), Act::MenuButtonPick(MenuButton::Combo))
                .with_help(settings::combo_note(combo)),
            Item::action("Change combination", Act::PickMenuCombo)
                .with_help("L3 + R3, Select + Start, LB + RB + Start, or record the buttons you want."),
            Item::action("The Home button", Act::MenuButtonPick(MenuButton::Home))
                .with_help(settings::home_button_note()),
        ]
    }

    /// The answer: remembered.
    fn pick_menu_button(&mut self, st: &mut AppState, choice: MenuButton) {
        st.config.controller.menu_button = choice;
        self.close_menu_button_screens(st);
    }

    /// Back on the question: nothing is decided (Home keeps opening Lodestar's menu) and it isn't
    /// asked again this session; a later session asks once more.
    fn put_off_menu_button_question(&mut self, st: &mut AppState) {
        self.sys.menus.retain(|m| !on_top(&m.kind));
        self.sys.menu_question_put_off = true;
        self.refresh_settings(st);
    }

    /// The question and its combination screens close after an answer: never asked again.
    fn close_menu_button_screens(&mut self, st: &mut AppState) {
        let asked = self.sys.menus.iter().any(|m| m.kind == MenuKind::MenuButtonAsk);
        self.sys.menus.retain(|m| !on_top(&m.kind));
        if asked {
            st.config.controller.menu_button_asked = true;
            st.save_config();
        }
        self.refresh_settings(st);
    }

    /// Keep the menu button screens on top of a menu opened after them.
    fn raise_menu_button_screens(&mut self) {
        let (mut top, rest): (Vec<Menu>, Vec<Menu>) =
            std::mem::take(&mut self.sys.menus).into_iter().partition(|m| on_top(&m.kind));
        self.sys.menus = rest;
        self.sys.menus.append(&mut top);
    }

    /// Where a menu opened now goes: under the menu button screens.
    fn under_menu_button_screens(&self) -> usize {
        self.sys.menus.iter().position(|m| on_top(&m.kind)).unwrap_or(self.sys.menus.len())
    }

    // ------------------------------------------------------------------ the in-game menu

    fn open_ingame(&mut self, st: &mut AppState, id: GameId, now: f64) {
        self.sys.menus.retain(|m| !matches!(m.kind, MenuKind::InGame | MenuKind::InGameConfirm));
        // The library's own menus / keyboard close: the in-game menu is what's in front now.
        self.close_menus(st);
        self.close_osk(st);
        self.viewer = None;
        if self.sys.mixer.is_none() {
            self.sys.mixer = Some(if st.no_launch { None } else { sys::mixer(super::power::has_tool) });
        }
        if self.sys.shot.is_none() {
            let wayland = crate::app::is_wayland(|k| std::env::var_os(k));
            self.sys.shot = Some(if st.no_launch {
                None
            } else {
                sys::screenshot_tool(wayland, crate::launch::winhide_wayland::detect(), super::power::has_tool)
            });
        }
        if let Some(m) = self.sys.mixer.flatten() {
            let vol = self.sys.volume.clone();
            let ctx = st.ctx().clone();
            let _ = std::thread::Builder::new().name("volume".into()).spawn(move || {
                *vol.lock() = sys::read_volume(m);
                ctx.request_repaint();
            });
        }
        let name = st.library.get(id).map(|g| g.display_name().to_string()).unwrap_or_else(|| "Game".into());
        let m = Menu::new(MenuKind::InGame, name, self.ingame_items(st, id), now)
            .with_subtitle(self.session_line(st, id))
            .centred();
        self.sys.menus.insert(0, m);
        self.sys.library = false;
    }

    /// "Playing for 1 h 05 min" ("Starting…" while the launch is still on its way).
    fn session_line(&self, st: &AppState, id: GameId) -> String {
        if crate::ui::launching::starting(st, id) {
            return "Starting…".into();
        }
        let since = self
            .running
            .filter(|r| r.0 == id)
            .map(|r| r.1)
            .or_else(|| st.launcher.adopted_start(id))
            .unwrap_or_else(now_unix);
        let secs = (now_unix() - since).max(0);
        let (h, m) = (secs / 3600, (secs % 3600) / 60);
        if h > 0 {
            format!("Playing for {h} h {m:02} min")
        } else if m > 0 {
            format!("Playing for {m} min")
        } else {
            "Playing · just started".into()
        }
    }

    fn ingame_items(&self, st: &AppState, id: GameId) -> Vec<Item> {
        // A launch still on its way (a store client signing in, updating …): Resume brings back
        // whatever the launch shows, and quitting cancels it.
        let starting = crate::ui::launching::starting(st, id);
        let mut v = vec![Item::action(if starting { "Back to the launch" } else { "Resume game" }, Act::Resume)];
        if self.sys.shot.flatten().is_some() {
            v.push(Item::action("Take a screenshot", Act::Screenshot));
        }
        if self.sys.mixer.flatten().is_some() {
            let val = match *self.sys.volume.lock() {
                Some((_, true)) => "Muted".to_string(),
                Some((p, false)) => format!("◀  {p} %  ▶"),
                None => "◀  ▶".to_string(),
            };
            v.push(Item::value("Volume", val, Act::Volume));
        }
        v.push(Item::action("Library", Act::Library));
        let quitting = self.sys.quitting.is_some_and(|(g, _)| g == id);
        let label = if starting { "Cancel launch" } else { "Quit game" };
        v.push(Item::value(label, if quitting { "Closing…" } else { "" }, Act::QuitGameAsk(id)).danger());
        v
    }

    fn refresh_ingame(&mut self, st: &AppState) {
        let Some(id) = crate::ui::launching::running(st) else { return };
        let items = self.ingame_items(st, id);
        let line = self.session_line(st, id);
        if let Some(m) = self.sys.menus.iter_mut().find(|m| m.kind == MenuKind::InGame) {
            m.replace_items(items);
            m.subtitle = Some(line);
        }
    }

    /// Focus went back to the game without Resume (alt-tab, the game took it): the in-game
    /// menu and library browsing close, so the next time Lodestar gets the focus on its own
    /// the game-running rule applies again (only the Playing overlay's buttons).
    pub fn focus_left_for_game(&mut self) {
        self.sys.menus.retain(|m| on_top(&m.kind));
        self.sys.library = false;
        self.sys.left_for_game = !self.sys.menus.is_empty();
    }

    /// Back to the game: the in-game menu closes, the game's window comes to the front (or,
    /// when none is found, Lodestar steps aside).
    fn resume(&mut self, st: &mut AppState) {
        self.sys.menus.retain(|m| on_top(&m.kind));
        self.sys.library = false;
        let Some(id) = crate::ui::launching::running(st) else { return };
        if st.no_launch {
            log::info!("screenshot run: not switching to game {id}");
            return;
        }
        let appid =
            st.library.get(id).filter(|g| g.source == Source::Steam).and_then(|g| g.source_id.parse::<u32>().ok());
        let ctx = st.ctx().clone();
        let flag = self.sys.resumed_minimised.clone();
        // Reading every process' environment takes a moment: not on the UI thread.
        let _ = std::thread::Builder::new().name("resume".into()).spawn(move || {
            let pids = sys::game_pids(std::path::Path::new("/proc"), id, appid);
            crate::winctl::activate_pids(crate::winctl::Platform::detect(), pids, move || {
                flag.store(true, Ordering::SeqCst);
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                ctx.request_repaint();
            });
        });
    }

    fn screenshot(&mut self, st: &mut AppState) {
        let Some(tool) = self.sys.shot.flatten() else { return };
        let Some(id) = crate::ui::launching::running(st) else { return };
        let name = crate::ui::game_name(st, id);
        self.resume(st);
        if st.no_launch {
            return;
        }
        let pictures = dirs::picture_dir().unwrap_or_else(|| st.paths.home.join("Pictures"));
        let path = sys::screenshot_path(&pictures, &name, now_unix());
        let _ = std::thread::Builder::new().name("screenshot".into()).spawn(move || {
            std::thread::sleep(SHOT_DELAY);
            if let Some(d) = path.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let argv = sys::screenshot_argv(tool, &path);
            let ok = std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
                && path.exists();
            let text = if ok {
                format!("Screenshot saved to {}", path.display())
            } else {
                "The screenshot didn't work".to_string()
            };
            log::info!("{text}");
            if let Some(ns) = crate::launch::find_in_path("notify-send") {
                let _ = std::process::Command::new(ns)
                    .args(["--app-name=Lodestar", "--icon=lodestar", "--expire-time=4000", "Lodestar", &text])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status();
            }
        });
    }

    fn volume(&mut self, st: &AppState, op: VolOp) {
        let Some(m) = self.sys.mixer.flatten() else { return };
        if st.no_launch {
            return;
        }
        let vol = self.sys.volume.clone();
        let ctx = st.ctx().clone();
        let _ = std::thread::Builder::new().name("volume".into()).spawn(move || {
            if let Some(v) = sys::change_volume(m, op) {
                *vol.lock() = Some(v);
            }
            ctx.request_repaint();
        });
    }

    fn ask_quit(&mut self, st: &AppState, id: GameId, now: f64) {
        let name = st.library.get(id).map(|g| g.display_name().to_string()).unwrap_or_else(|| "the game".into());
        let (title, items) = if crate::ui::launching::starting(st, id) {
            (
                format!("Cancel starting {name}?"),
                vec![
                    Item::action("Cancel launch", Act::QuitGameDo(id)).danger(),
                    Item::action("Keep waiting", Act::CloseMenu),
                ],
            )
        } else {
            (
                format!("Quit {name}?"),
                vec![
                    Item::info("Unsaved progress is lost", "Save in the game first if you can."),
                    Item::action("Quit game", Act::QuitGameDo(id)).danger(),
                    Item::action("Keep playing", Act::CloseMenu),
                ],
            )
        };
        let mut m = Menu::new(MenuKind::InGameConfirm, title, items, now).centred();
        m.select_act(&Act::CloseMenu);
        self.sys.menus.insert(self.under_menu_button_screens(), m);
    }

    fn sys_act(&mut self, st: &mut AppState, act: Act, dir: i32, now: f64) {
        match act {
            Act::MenuButtonPick(choice) => self.pick_menu_button(st, choice),
            Act::PickMenuCombo => self.open_combo_picker(st, now),
            Act::MenuComboPreset(combo) => self.use_menu_combo(st, combo),
            Act::RecordMenuCombo => self.open_combo_recorder(st, now),
            Act::RecordedComboUse(combo) => self.use_menu_combo(st, combo),
            Act::RecordComboAgain => self.record_combo_again(st, now),
            Act::Resume => self.resume(st),
            Act::Screenshot => self.screenshot(st),
            Act::Volume => {
                let op = match dir {
                    d if d < 0 => VolOp::Down,
                    d if d > 0 => VolOp::Up,
                    _ => VolOp::ToggleMute,
                };
                self.volume(st, op);
            }
            Act::Library => {
                self.sys.menus.retain(|m| on_top(&m.kind));
                if st.mode == Mode::Desktop {
                    // The desktop library is right there under the panel.
                    return;
                }
                self.sys.library = true;
                self.overlay_hidden = true;
                self.stop_armed = None;
                self.home.rail_focus = false;
                self.go(self.first_screen(st), now);
            }
            Act::QuitGameAsk(id) => self.ask_quit(st, id, now),
            Act::QuitGameDo(id) => {
                self.sys.menus.retain(|m| m.kind != MenuKind::InGameConfirm);
                crate::ui::launching::stop(st, id);
                self.sys.quitting = Some((id, now));
                self.refresh_ingame(st);
            }
            Act::CloseMenu => {
                self.sys.menus.pop();
            }
            _ => {}
        }
    }

    /// Navigation for the in-game menu / the menu button screens; false when none is open (or the library is
    /// browsed, except Back on its first screen, which returns to the menu).
    pub(super) fn sys_nav(&mut self, st: &mut AppState, nav: Nav, now: f64) -> bool {
        if self.sys.left_for_game && !self.sys.menus.is_empty() {
            // SAFETY RULE: the menu button screens left on screen for the game. The gate passes
            // the Playing overlay's buttons (A among them) to a focus Lodestar got on its own;
            // those presses were meant for the game and must not answer anything here. They are
            // swallowed until the menu button brings the screens back.
            return true;
        }
        if self.sys.menus.is_empty() {
            if self.sys.library
                && nav == Nav::Back
                && self.screen == self.first_screen(st)
                && !self.home.rail_focus
                && self.menus.is_empty()
                && self.osk.is_none()
                && self.viewer.is_none()
                && let Some(id) = crate::ui::launching::running(st)
            {
                self.open_ingame(st, id, now);
                return true;
            }
            return false;
        }
        match nav {
            Nav::Up | Nav::Down => {
                if let Some(m) = self.sys.menus.last_mut() {
                    m.move_sel(if nav == Nav::Up { -1 } else { 1 });
                }
            }
            Nav::Back | Nav::Menu => {
                let top = self.top_sys();
                if top == Some(MenuKind::MenuButtonAsk) {
                    self.put_off_menu_button_question(st);
                } else if matches!(
                    top,
                    Some(MenuKind::InGameConfirm | MenuKind::MenuComboPick | MenuKind::MenuComboRecord)
                ) {
                    self.sys.menus.pop();
                } else {
                    // B on the in-game menu: back to the game, like Resume.
                    self.resume(st);
                }
            }
            Nav::Accept | Nav::Left | Nav::Right => {
                let act = self.sys.menus.last().and_then(|m| m.selected_act()).cloned();
                let dir = match nav {
                    Nav::Left => -1,
                    Nav::Right => 1,
                    _ => 0,
                };
                match act {
                    Some(Act::Volume) => self.sys_act(st, Act::Volume, dir, now),
                    Some(a) if nav == Nav::Accept => self.sys_act(st, a, 0, now),
                    _ => {}
                }
            }
            _ => {}
        }
        true
    }

    /// Per frame (both modes): a game that ended closes the in-game menu; a quit the game
    /// ignored is forced; the volume read in the background shows.
    pub(super) fn sys_tick(&mut self, st: &mut AppState, now: f64) {
        self.combo_tick(st);
        let running = crate::ui::launching::running(st);
        if running.is_none() {
            if self.sys.menus.iter().any(|m| !on_top(&m.kind)) || self.sys.library {
                self.sys.menus.retain(|m| on_top(&m.kind));
                self.sys.library = false;
            }
            self.sys.quitting = None;
            if self.sys.resumed_minimised.swap(false, Ordering::SeqCst) {
                st.show_window();
            }
            return;
        }
        if let Some((id, t)) = self.sys.quitting {
            if !crate::ui::launching::is_active(st, id) {
                self.sys.quitting = None;
            } else if now - t >= FORCE_AFTER_SECS {
                // Asking again once it ignored the first request stops it for good.
                log::info!("game {id} ignored the request to quit: stopping it");
                crate::ui::launching::stop(st, id);
                self.sys.quitting = None;
            } else {
                st.ctx().request_repaint_after(std::time::Duration::from_secs_f64(FORCE_AFTER_SECS - (now - t) + 0.05));
            }
        }
        if self.sys.menus.iter().any(|m| m.kind == MenuKind::InGame) {
            self.refresh_ingame(st);
            // The session time ticks.
            st.ctx().request_repaint_after(std::time::Duration::from_secs(20));
        }
    }

    /// Draw the in-game menu / "Which button opens Lodestar's menu?" (top one only). Returns "animating".
    pub(super) fn draw_sys(
        &mut self,
        painter: &egui::Painter,
        st: &mut AppState,
        screen: Rect,
        look: &Look,
        now: f64,
        pointer: Option<(egui::Pos2, bool)>,
    ) -> bool {
        let Some(m) = self.sys.menus.last_mut() else { return false };
        let dt = painter.ctx().input(|i| i.stable_dt.clamp(1.0 / 240.0, 1.0 / 20.0));
        let out = m.draw(painter, screen, look, now, dt, pointer);
        if let Some(i) = out.clicked {
            m.sel = i;
            if let Some(a) = m.selected_act().cloned() {
                self.sys_act(st, a, 0, now);
            }
        }
        out.animating
    }

    /// Desktop mode: the in-game menu / "Which button opens Lodestar's menu?" as a centred panel over the window, fed with the
    /// frame's controller navigation.
    pub fn desktop_overlay(&mut self, ctx: &egui::Context, st: &mut AppState, navs: &[Nav]) {
        let now = ctx.input(|i| i.time);
        self.sys_tick(st, now);
        if self.sys.menus.is_empty() {
            return;
        }
        for &n in navs {
            self.sys_nav(st, n, now);
        }
        if self.sys.menus.is_empty() {
            return;
        }
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("lodestar_ingame_overlay"))
            .fixed_pos(screen.min)
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                // Takes the clicks so nothing under the panel gets them.
                let _ = ui.allocate_rect(screen, egui::Sense::click());
                let look = Look::new(st, screen);
                let pointer = ui.input(|i| i.pointer.hover_pos().map(|p| (p, i.pointer.primary_clicked())));
                let painter = ui.painter().clone();
                if self.draw_sys(&painter, st, screen, &look, now, pointer) {
                    ctx.request_repaint();
                }
            });
    }

    /// Controller mode: clock and battery in the top-right corner (settings → General).
    pub(super) fn draw_status(&mut self, painter: &egui::Painter, st: &AppState, screen: Rect, look: &Look, now: f64) {
        let c = &st.config.controller;
        if !c.show_clock && !c.show_battery {
            return;
        }
        // The battery is read every 30 s.
        let fresh = self.sys.battery.is_some_and(|(t, _)| now - t < 30.0 && now >= t);
        if !fresh {
            let b = match crate::theme::render::bind::fake_battery() {
                Some(fake) => fake,
                None if st.no_launch => None,
                // Read on a helper thread; the reading before stays until it answers.
                None => sys::battery_now("/sys"),
            };
            self.sys.battery = Some((now, b));
        }
        let view = self.status_view(st);
        let Some(text) = settings::status_text(st, self.sys.battery.and_then(|b| b.1), view) else { return };
        let g = painter.layout_no_wrap(text, look.medium(0.022), look.text);
        let pad = look.h(0.012);
        let size = g.size() + egui::vec2(pad * 2.0, pad);
        let r =
            Rect::from_min_size(egui::pos2(screen.right() - size.x - look.h(0.03), screen.top() + look.h(0.025)), size);
        painter.rect_filled(r, crate::theme::render::draw::cr(size.y / 2.0), Color32::from_black_alpha(110));
        painter.galley(r.min + egui::vec2(pad, pad / 2.0), g, look.text);
        let secs_left = 60 - now_unix().rem_euclid(60);
        st.ctx().request_repaint_after(std::time::Duration::from_secs(secs_left.max(1) as u64));
    }
}

/// The menu button screens (the question, changing the combination): on top of the in-game
/// menu, and kept when the focus goes back to the game.
pub(super) fn on_top(kind: &MenuKind) -> bool {
    matches!(kind, MenuKind::MenuButtonAsk | MenuKind::MenuComboPick | MenuKind::MenuComboRecord)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st_with_game(dir: &std::path::Path) -> (AppState, GameId) {
        let mut st = AppState::for_test(dir);
        st.add_manual_game(
            "Test Game",
            LaunchSpec::Command { program: "/bin/true".into(), args: vec![], cwd: None, env: vec![] },
            Art::default(),
        );
        let id = st.library.games[0].id;
        (st, id)
    }

    #[test]
    fn stray_presses_while_a_game_runs_never_reach_the_library() {
        // Lodestar focused while a game runs (the game never took the focus, alt-tab …): the
        // gate lets A / B / Start through, and they must only ever reach the Playing overlay —
        // B, Start, A used to hide the overlay, open the quick menu and pick "Stop game".
        let dir = tempfile::tempdir().unwrap();
        let (mut st, id) = st_with_game(dir.path());
        st.no_launch = true;
        st.mode = Mode::Controller;
        let mut ui = ControllerUi::new();
        let _fake = crate::ui::launching::FAKE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(crate::ui::launching::debug_fake(&st, id, "starting"));
        ui.track_running(&st, 0.0);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
        for (i, nav) in [Nav::Back, Nav::Menu, Nav::Accept, Nav::Back, Nav::Accept, Nav::Accept].into_iter().enumerate()
        {
            // Past the overlay's grace time, one press every 5 s (never a quick A, A).
            ui.handle_nav(&mut st, nav, screen, 10.0 + 5.0 * i as f64, (false, false));
            assert!(ui.menus.is_empty(), "no quick menu after {nav:?}");
            assert!(ui.overlay_shown(), "the Playing overlay stays after {nav:?}");
        }
        assert_eq!(crate::ui::launching::running(&st), Some(id), "the game still runs");
    }

    #[test]
    fn guide_opens_the_in_game_menu_while_a_game_runs() {
        let dir = tempfile::tempdir().unwrap();
        let (mut st, id) = st_with_game(dir.path());
        // Never touch real windows / processes from a test (and the launch fake needs it).
        st.no_launch = true;
        st.mode = Mode::Controller;
        st.config.controller.menu_button_asked = true;
        // The launch fake is shared: no other test's game may run during this one.
        let _fake = crate::ui::launching::FAKE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut ui = ControllerUi::new();
        // No game: the menu button opens the quick menu and closes it again; no full input.
        ui.on_guide(&mut st, false, 0.0);
        assert_eq!(ui.menus.last().map(|m| m.kind.clone()), Some(MenuKind::Quick));
        ui.on_guide(&mut st, false, 0.5);
        assert!(ui.sys.menus.is_empty() && ui.menus.is_empty() && !ui.wants_full_input());
        // A game "starts" (screenshot fake).
        assert!(crate::ui::launching::debug_fake(&st, id, "starting"));
        ui.on_guide(&mut st, true, 1.0);
        assert_eq!(ui.sys.menus.last().map(|m| m.kind.clone()), Some(MenuKind::InGame));
        assert!(ui.wants_full_input(), "the user asked for Lodestar: the gate lets the pad through");
        let labels: Vec<String> = ui.sys.menus[0].items.iter().map(|i| i.label.clone()).collect();
        // The fake game is still starting: the menu says so and offers to cancel the launch.
        assert_eq!(
            labels,
            ["Back to the launch", "Library", "Cancel launch"],
            "no screenshot / volume tools in a test run"
        );
        assert_eq!(ui.sys.menus[0].subtitle.as_deref(), Some("Starting…"));
        // Cancelling asks first; Keep waiting is selected.
        ui.sys_nav(&mut st, Nav::Down, 2.0);
        ui.sys_nav(&mut st, Nav::Down, 2.0);
        ui.sys_nav(&mut st, Nav::Accept, 2.0);
        assert_eq!(ui.sys.menus.last().map(|m| m.kind.clone()), Some(MenuKind::InGameConfirm));
        assert_eq!(ui.sys.menus.last().unwrap().selected_act(), Some(&Act::CloseMenu));
        let confirm: Vec<String> = ui.sys.menus.last().unwrap().items.iter().map(|i| i.label.clone()).collect();
        assert_eq!(confirm, ["Cancel launch", "Keep waiting"]);
        ui.sys_nav(&mut st, Nav::Back, 2.0);
        assert_eq!(ui.sys.menus.last().map(|m| m.kind.clone()), Some(MenuKind::InGame));
        // Library: browse, Back on the first screen returns to the menu.
        ui.sys_nav(&mut st, Nav::Up, 3.0);
        ui.sys_nav(&mut st, Nav::Accept, 3.0);
        assert!(ui.sys.menus.is_empty() && ui.sys.library && ui.wants_full_input());
        assert!(ui.sys_nav(&mut st, Nav::Back, 4.0));
        assert_eq!(ui.sys.menus.last().map(|m| m.kind.clone()), Some(MenuKind::InGame));
        // Home again: Resume (closes the menu).
        ui.on_guide(&mut st, false, 5.0);
        assert!(ui.sys.menus.is_empty() && !ui.wants_full_input());
        // The game ends: nothing stays open.
        ui.on_guide(&mut st, false, 6.0);
        crate::ui::launching::stop(&mut st, id);
        ui.sys_tick(&mut st, 7.0);
        assert!(ui.sys.menus.is_empty() && !ui.sys.library);
    }

    /// In controller mode with no game the menu button always toggles the quick menu: after
    /// the question was answered (combination or Home) and after it was put off, whatever the
    /// desktop-mode switch says.
    #[test]
    fn menu_button_toggles_the_quick_menu_in_the_library() {
        let _fake = crate::ui::launching::FAKE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let (mut st, _) = st_with_game(dir.path());
        st.mode = Mode::Controller;
        assert!(!st.config.controller.guide_opens_controller_mode, "the desktop switch is off by default");
        let toggles = |ui: &mut ControllerUi, st: &mut AppState, t: f64| {
            ui.on_guide(st, false, t);
            assert_eq!(ui.menus.last().map(|m| m.kind.clone()), Some(MenuKind::Quick), "opens the menu");
            assert!(ui.sys.menus.is_empty());
            ui.on_guide(st, false, t + 0.5);
            assert!(ui.menus.is_empty(), "the same button closes it");
        };
        for answer in [MenuButton::Combo, MenuButton::Home] {
            let mut ui = ControllerUi::new();
            st.config.controller.menu_button_asked = false;
            // The first press asks; the answer closes the question and nothing else opens.
            ui.on_guide(&mut st, false, 0.0);
            assert_eq!(top(&ui), Some(MenuKind::MenuButtonAsk));
            ui.sys_act(&mut st, Act::MenuButtonPick(answer), 0, 1.0);
            assert!(ui.sys.menus.is_empty() && ui.menus.is_empty());
            toggles(&mut ui, &mut st, 2.0);
            // Opened from a submenu (Settings over the quick menu): one press closes them all.
            ui.on_guide(&mut st, false, 3.0);
            ui.open_settings(&st, 3.1);
            ui.on_guide(&mut st, false, 3.2);
            assert!(ui.menus.is_empty());
        }
        // Put off with Back: the button still works this session.
        st.config.controller.menu_button_asked = false;
        let mut ui = ControllerUi::new();
        ui.on_guide(&mut st, false, 0.0);
        ui.sys_nav(&mut st, Nav::Back, 1.0);
        assert!(ui.sys.menus.is_empty() && ui.menus.is_empty());
        toggles(&mut ui, &mut st, 2.0);
        // Brought to the front from elsewhere: only that, no menu.
        ui.on_guide(&mut st, true, 4.0);
        assert!(ui.menus.is_empty());
    }

    #[test]
    fn guide_without_a_game_in_desktop_mode_follows_the_switch() {
        // No other test's fake game may run during this one.
        let _fake = crate::ui::launching::FAKE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let (mut st, _) = st_with_game(dir.path());
        st.config.controller.menu_button_asked = true;
        let mut ui = ControllerUi::new();
        st.mode = Mode::Desktop;
        ui.on_guide(&mut st, false, 0.0);
        st.config.controller.guide_opens_controller_mode = true;
        ui.on_guide(&mut st, true, 0.0);
        assert!(st.pending_mode_for_test().is_none(), "a summon only brings Lodestar up");
        ui.on_guide(&mut st, false, 0.0);
        assert_eq!(st.pending_mode_for_test(), Some(Mode::Controller));
        st.mode = Mode::Controller;
        ui.on_guide(&mut st, false, 0.0);
        assert_eq!(ui.menus.last().map(|m| m.kind.clone()), Some(MenuKind::Quick));
        ui.on_guide(&mut st, false, 0.0);
        assert!(ui.menus.is_empty());
    }

    fn top(ui: &ControllerUi) -> Option<MenuKind> {
        ui.sys.menus.last().map(|m| m.kind.clone())
    }

    fn choices(ui: &ControllerUi) -> Vec<String> {
        let m = ui.sys.menus.last().unwrap();
        m.items.iter().filter(|i| !i.info).map(|i| i.label.clone()).collect()
    }

    #[test]
    fn menu_button_question_asked_once_on_the_first_press() {
        // No other test's fake game may run during this one.
        let _fake = crate::ui::launching::FAKE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let (mut st, _) = st_with_game(dir.path());
        let mut ui = ControllerUi::new();
        ui.on_guide(&mut st, false, 0.0);
        assert_eq!(top(&ui), Some(MenuKind::MenuButtonAsk));
        let m = ui.sys.menus.last().unwrap();
        assert_eq!(m.title, "Which button opens Lodestar's menu?");
        assert_eq!(choices(&ui), ["A button combination (L3 + R3)", "Change combination", "The Home button"]);
        assert_eq!(m.selected_act(), Some(&Act::MenuButtonPick(MenuButton::Combo)), "the combination is preselected");
        // Each choice explains itself when selected (or hovered), nothing up front.
        assert!(m.items.iter().all(|i| !i.info));
        assert_eq!(
            m.items[m.help_item(None).unwrap()].help.as_deref(),
            Some("Click both sticks in and hold for a moment. Steam and emulators leave it alone.")
        );
        let home = m.items[2].help.clone().unwrap();
        for part in [
            "Steam also reacts to Home",
            "Guide button focuses Steam",
            "Enable the Steam Overlay while in-game",
            "PCSX2",
        ] {
            assert!(home.contains(part), "{home}");
        }
        assert!(ui.wants_full_input(), "the pad reaches the question");
        // A: the combination, remembered; nothing else opens (no game, the press only asked).
        ui.sys_nav(&mut st, Nav::Accept, 1.0);
        assert!(ui.sys.menus.is_empty() && ui.menus.is_empty());
        let saved = crate::config::Config::load(&st.paths.config_file()).controller;
        assert_eq!((saved.menu_button, saved.menu_button_asked), (MenuButton::Combo, true));
        assert!(!saved.home_opens_menu(), "Home is left to Steam and emulators now");
        // Never again, this session or the next.
        ui.on_guide(&mut st, false, 2.0);
        assert!(ui.sys.menus.is_empty());
        let mut ui = ControllerUi::new();
        ui.on_guide(&mut st, false, 3.0);
        assert!(ui.sys.menus.is_empty());
    }

    #[test]
    fn menu_button_question_put_off_keeps_home_and_asks_in_a_later_session() {
        // No other test's fake game may run during this one.
        let _fake = crate::ui::launching::FAKE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let (mut st, _) = st_with_game(dir.path());
        st.config.controller.menu_combo = crate::config::MenuCombo::SELECT_START;
        // Back: put off. Nothing is decided, Home keeps opening the menu, and this session
        // doesn't ask again.
        let mut ui = ControllerUi::new();
        ui.on_guide(&mut st, false, 0.0);
        assert_eq!(choices(&ui)[0], "A button combination (Select + Start)", "names the combination");
        ui.sys_nav(&mut st, Nav::Back, 1.0);
        assert!(ui.sys.menus.is_empty());
        assert!(!st.config.controller.menu_button_asked);
        assert!(st.config.controller.home_opens_menu(), "Home still works");
        ui.on_guide(&mut st, false, 1.5);
        assert!(!ui.sys.menus.iter().any(|m| m.kind == MenuKind::MenuButtonAsk), "not again this session");
        // A later session asks once more; the menu button pressed again while it shows (or
        // "Change combination" over it) puts it off too; brought to the front, it stays.
        let mut ui = ControllerUi::new();
        ui.on_guide(&mut st, false, 2.0);
        ui.sys_nav(&mut st, Nav::Down, 2.1);
        ui.sys_nav(&mut st, Nav::Accept, 2.1);
        assert_eq!(top(&ui), Some(MenuKind::MenuComboPick));
        ui.on_guide(&mut st, true, 2.5);
        assert_eq!(top(&ui), Some(MenuKind::MenuComboPick));
        ui.on_guide(&mut st, false, 3.0);
        assert!(!ui.sys.menus.iter().any(|m| m.kind == MenuKind::MenuButtonAsk || m.kind == MenuKind::MenuComboPick));
        assert!(!st.config.controller.menu_button_asked, "still unanswered");
    }

    #[test]
    fn choosing_home_in_a_game_then_the_in_game_menu() {
        let dir = tempfile::tempdir().unwrap();
        let (mut st, id) = st_with_game(dir.path());
        st.no_launch = true;
        st.mode = Mode::Controller;
        let _fake = crate::ui::launching::FAKE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(crate::ui::launching::debug_fake(&st, id, "starting"));
        let mut ui = ControllerUi::new();
        // Not asked yet: Home counts, so the first press can ask.
        assert!(st.config.controller.home_opens_menu());
        ui.on_guide(&mut st, true, 1.0);
        let kinds: Vec<MenuKind> = ui.sys.menus.iter().map(|m| m.kind.clone()).collect();
        assert_eq!(kinds, [MenuKind::InGame, MenuKind::MenuButtonAsk]);
        ui.sys_nav(&mut st, Nav::Down, 2.0);
        ui.sys_nav(&mut st, Nav::Down, 2.0);
        ui.sys_nav(&mut st, Nav::Accept, 2.0);
        assert_eq!(st.config.controller.menu_button, MenuButton::Home);
        assert_eq!(top(&ui), Some(MenuKind::InGame), "the in-game menu waited under it");
        assert!(st.config.controller.home_opens_menu());
        assert_eq!(st.config.controller.active_menu_combo(), None);
        assert_eq!(settings::menu_button_name(&st), "Home", "prompts name the chosen button");
        // The menu button again: back to the game; next time the in-game menu straight away.
        ui.on_guide(&mut st, false, 4.0);
        assert!(ui.sys.menus.is_empty());
        ui.on_guide(&mut st, true, 5.0);
        assert_eq!(ui.sys.menus.len(), 1);
        assert_eq!(top(&ui), Some(MenuKind::InGame));
        crate::ui::launching::stop(&mut st, id);
    }

    /// REGRESSION (safety rule): the question left on screen when the focus went back to the
    /// game doesn't let the game's presses through if Lodestar gets the focus on its own; the
    /// menu button brings it back under the pad.
    #[test]
    fn menu_button_question_left_for_the_game_gets_only_the_overlay_buttons() {
        use crate::input::gate::{Situation, filter};
        let dir = tempfile::tempdir().unwrap();
        let (mut st, id) = st_with_game(dir.path());
        st.no_launch = true;
        let _fake = crate::ui::launching::FAKE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(crate::ui::launching::debug_fake(&st, id, "starting"));
        let mut ui = ControllerUi::new();
        ui.on_guide(&mut st, true, 1.0);
        assert_eq!(top(&ui), Some(MenuKind::MenuButtonAsk));
        let focused = |ui: &ControllerUi| Situation {
            game_up: true,
            focused: Some(true),
            guide_summons: true,
            lodestar_menu: ui.wants_full_input(),
            ..Default::default()
        };
        assert_eq!(
            filter(&focused(&ui), vec![Nav::Down, Nav::Accept]),
            [Nav::Down, Nav::Accept],
            "asked for: the pad reaches it"
        );
        // The game takes the focus back; the question stays, but a focus Lodestar gets on its own
        // passes only what the Playing overlay takes (the preselected choice can't change).
        ui.focus_left_for_game();
        assert_eq!(top(&ui), Some(MenuKind::MenuButtonAsk));
        assert_eq!(filter(&focused(&ui), vec![Nav::Down, Nav::Up, Nav::Accept]), [Nav::Accept]);
        // The menu button again: the question is what the user is using.
        ui.on_guide(&mut st, true, 2.0);
        assert_eq!(top(&ui), Some(MenuKind::MenuButtonAsk));
        assert!(ui.wants_full_input());
        assert!(!st.config.controller.menu_button_asked);
        crate::ui::launching::stop(&mut st, id);
    }

    /// REGRESSION (safety rule): the game's A press, reaching a focus Lodestar got on its own,
    /// must not answer the question left on screen (it used to save the highlighted choice:
    /// Home as the menu button).
    #[test]
    fn the_games_a_press_never_answers_the_question_left_for_the_game() {
        use crate::input::gate::{Situation, filter};
        let dir = tempfile::tempdir().unwrap();
        let (mut st, id) = st_with_game(dir.path());
        st.no_launch = true;
        let _fake = crate::ui::launching::FAKE_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        assert!(crate::ui::launching::debug_fake(&st, id, "starting"));
        let mut ui = ControllerUi::new();
        ui.on_guide(&mut st, true, 1.0);
        ui.sys_nav(&mut st, Nav::Down, 1.1);
        ui.sys_nav(&mut st, Nav::Down, 1.2);
        assert_eq!(ui.sys.menus.last().unwrap().selected_act(), Some(&Act::MenuButtonPick(MenuButton::Home)));
        ui.focus_left_for_game();
        let sit = Situation {
            game_up: true,
            focused: Some(true),
            guide_summons: true,
            lodestar_menu: ui.wants_full_input(),
            ..Default::default()
        };
        for n in filter(&sit, vec![Nav::Accept, Nav::Back, Nav::Menu, Nav::Accept]) {
            ui.sys_nav(&mut st, n, 2.0);
        }
        assert!(!st.config.controller.menu_button_asked, "the game's presses answered the question");
        assert_eq!(st.config.controller.menu_button, MenuButton::Combo);
        assert_eq!(top(&ui), Some(MenuKind::MenuButtonAsk), "the question is still there for the user");
        // Brought back with the menu button, A answers it.
        ui.on_guide(&mut st, true, 3.0);
        ui.sys_nav(&mut st, Nav::Accept, 3.1);
        assert!(st.config.controller.menu_button_asked);
        crate::ui::launching::stop(&mut st, id);
    }
}
