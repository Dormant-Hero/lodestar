//! What the controller menus' entries do (`menu_act`), and the theme preview.

use super::power::Power;
use super::*;
use crate::ui::game_name;

/// A random number below `n` (xorshift seeded from the clock).
fn random_below(n: usize) -> usize {
    use std::sync::atomic::{AtomicU64, Ordering};
    static STATE: AtomicU64 = AtomicU64::new(0);
    let mut x = STATE.load(Ordering::Relaxed);
    if x == 0 {
        x = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9E37_79B9_7F4A_7C15, |d| d.as_nanos() as u64)
            | 1;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    STATE.store(x, Ordering::Relaxed);
    (x % n.max(1) as u64) as usize
}

impl ControllerUi {
    /// Live preview of the theme under the cursor in the theme menu.
    pub(super) fn preview_theme(&mut self, st: &mut AppState) {
        let Some(Act::SetTheme(key)) = self.menus.last().and_then(|m| m.selected_act()).cloned() else {
            return;
        };
        let Some(pv) = &mut self.theme_preview else {
            return;
        };
        if st.controller_theme.key == key {
            return;
        }
        let t = pv.cache.entry(key.clone()).or_insert_with(|| st.themes.load(&key)).clone();
        st.controller_theme = t;
    }

    pub(super) fn end_theme_preview(&mut self, st: &mut AppState, keep: bool) {
        if let Some(pv) = self.theme_preview.take()
            && !keep
        {
            st.controller_theme = pv.original;
        }
    }

    pub(super) fn menu_act(&mut self, st: &mut AppState, act: Act, now: f64) {
        match act {
            Act::SwitchDesktop => {
                self.close_menus(st);
                st.switch_mode(Mode::Desktop);
            }
            Act::Rescan => {
                st.rescan();
                st.notify("Rescanning library…");
                self.close_menus(st);
            }
            Act::ThemeMenu => self.open_theme_menu(st, now),
            Act::SetTheme(key) => {
                self.end_theme_preview(st, true);
                st.set_controller_theme(&key);
                self.menus.pop();
                self.renderer.reset_view(ViewKind::Collections);
                self.renderer.reset_view(ViewKind::Games);
                self.refresh_quick(st);
            }
            Act::VariantMenu => self.open_variant_menu(st, now),
            Act::SchemeMenu => self.open_scheme_menu(st, now),
            Act::SetScheme(s) => {
                st.config.controller.color_scheme = s;
                let key = st.config.controller.theme.clone();
                st.set_controller_theme(&key);
                self.menus.pop();
                self.refresh_quick(st);
            }
            Act::ThemeOptionsMenu => {
                let items = Self::theme_option_items(st);
                self.menus.push(
                    Menu::new(MenuKind::ThemeOptions, "Theme options", items, now)
                        .with_subtitle("The theme's own choices · ← / → change"),
                );
            }
            Act::ThemeOption(i, dir) => self.step_theme_option(st, i, dir),
            Act::SetVariant(v) => {
                st.config.controller.variant = v;
                st.save_config();
                self.menus.pop();
                self.refresh_quick(st);
            }
            Act::CycleButtonPrompts => {
                let c = &mut st.config.controller;
                c.button_prompts = c.button_prompts.next();
                st.save_config();
                self.refresh_quick(st);
            }
            Act::ToggleSwapAb => {
                st.config.controller.swap_ab = !st.config.controller.swap_ab;
                st.save_config();
                self.refresh_quick(st);
            }
            Act::ToggleFullscreen => self.toggle_fullscreen(st),
            Act::Quit => {
                self.close_menus(st);
                st.quit();
            }
            Act::Play(id) => super::emu::play_or_choose(self, st, id, now),
            Act::PlayWindowed(id) => {
                self.close_menus(st);
                st.launch_windowed(id);
            }
            Act::OpenEmulator(id) => {
                self.close_menus(st);
                super::emu::open(st, id);
            }
            Act::PlayWithMenu(id) => super::emu::open_play_with(self, st, id, now),
            Act::PlayWith(id, emu, profile) => {
                let always = super::emu::always(st, id);
                self.close_menus(st);
                st.launch_with(id, &emu, &profile, always);
            }
            Act::ToggleAlwaysUse(id) => super::emu::toggle_always(self, st, id),
            Act::PlayChoice(id, c) => {
                let always = super::emu::choose_always(st, id);
                self.close_menus(st);
                st.launch_choice(id, c, always);
            }
            Act::ToggleChooseAlways(id) => super::emu::toggle_choose_always(self, st, id),
            Act::Install(id) => {
                self.close_menus(st);
                Self::install(st, id);
            }
            Act::CancelDownload(id) => {
                self.close_menus(st);
                st.romm_cancel(id);
            }
            Act::Uninstall(id) => {
                self.close_menus(st);
                Self::uninstall(st, id);
            }
            Act::StopGame(id) => {
                self.close_menus(st);
                crate::ui::launching::stop(st, id);
            }
            Act::ViewArt(id) => {
                self.close_menus(st);
                self.open_viewer(st, id);
            }
            Act::ShowDetails(id) => self.show_details_of(st, id, now),
            Act::ToggleFavorite(id) => {
                st.toggle_favorite(id);
                self.close_menus(st);
            }
            Act::Hide(id) => {
                if !self.hide_game(st, id, now) {
                    // Only armed: the menu stays as it is, showing the prompt.
                    return;
                }
            }
            Act::Unhide(id) => self.unhide_game(st, id, now),
            Act::ToggleShowHidden => self.toggle_show_hidden(st, now),
            Act::LaunchOptions(id) => {
                let m = Menu::new(MenuKind::LaunchOptions(id), "Launch options", Self::launch_items(st, id), now)
                    .with_subtitle(game_name(st, id));
                self.menus.push(m);
            }
            Act::EditOptions(id) => self.edit_launch_options(st, id, now),
            Act::EditCommand(id) => self.edit_launch_command(st, id, now),
            Act::UseSourceOptions(id) => {
                if let Some(g) = st.library.get(id) {
                    let mut ov = g.launch_override.clone().unwrap_or_default();
                    ov.options = g.source_launch_options.clone().unwrap_or_default();
                    st.set_launch_override(id, Some(ov));
                }
                self.refresh_launch_menu(st, id, now);
            }
            Act::ClearOverride(id) => {
                st.set_launch_override(id, None);
                self.refresh_launch_menu(st, id, now);
            }
            Act::LaunchAction(id, i) => {
                self.close_menus(st);
                st.launch_action(id, i);
            }
            Act::CompletionMenu(id) => {
                let mut m =
                    Menu::new(MenuKind::Completion(id), "Completion status", Self::completion_items(st, id), now)
                        .with_subtitle(game_name(st, id));
                let cur = st.library.get(id).and_then(|g| g.completion());
                m.select_act(&Act::SetCompletion(id, cur));
                self.menus.push(m);
            }
            Act::SetCompletion(id, c) => {
                st.set_completion(id, c);
                self.menus.pop();
                self.refresh_game_options(st, id);
            }
            Act::ControllerMenu(id) => {
                let mut m =
                    Menu::new(MenuKind::Controller(id), "Controller support", Self::controller_items(st, id), now)
                        .with_subtitle(game_name(st, id));
                let cur = st.library.get(id).and_then(|g| g.controller_override);
                m.select_act(&Act::SetController(id, cur));
                self.menus.push(m);
            }
            Act::SetController(id, c) => {
                st.set_controller_override(id, c);
                self.menus.pop();
                self.refresh_game_options(st, id);
            }
            Act::SortMenu => {
                self.menus.push(Menu::new(MenuKind::Sort, "Sort by", self.sort_items(), now));
            }
            Act::LaunchersMenu => {
                self.menus.push(Menu::new(MenuKind::Launchers, "Launchers", Self::launchers_items(st), now));
            }
            Act::ToggleHideLaunchers
            | Act::ToggleCloseLaunchers
            | Act::ToggleHideSteam
            | Act::ToggleCloseSteam
            | Act::ToggleKeepSteam
            | Act::CycleOneGame
            | Act::CycleCloseWhilePlaying
            | Act::ToggleTrayIcon => self.step_launchers_setting(st, &act),
            Act::SetSort(s) => self.set_sort(s),
            Act::ToggleDescending => {
                self.descending = !self.descending;
                let items = self.sort_items();
                if let Some(m) = self.menus.last_mut() {
                    m.replace_items(items);
                }
            }
            Act::RandomGame => {
                self.close_menus(st);
                self.random_game(st, now);
            }
            Act::SettingsMenu => self.open_settings(st, now),
            Act::Setting(s) => self.apply_setting(st, s),
            Act::PowerAsk(p) => self.ask_power(st, p, now),
            Act::PowerDo(p) => {
                self.close_menus(st);
                if st.no_launch {
                    log::info!("screenshot run: not running {:?}", p.argv(None));
                    st.notify(format!("{} (skipped: screenshot run)", p.label()));
                } else if let Err(e) = super::power::run(p) {
                    st.notify_error(e);
                }
            }
            Act::CloseMenu => {
                self.menus.pop();
            }
            Act::Credits => self.open_credits(now),
            Act::CopyText(t) => {
                st.ctx().copy_text(t.clone());
                st.notify("Copied. Paste it in a terminal.");
            }
            Act::Read => {}
            // Settings → Input → Combination: its own panel on top (combo.rs).
            Act::PickMenuCombo => self.open_combo_picker(st, now),
            // In-game menu / menu button screen entries live in their own panels (ingame.rs,
            // combo.rs).
            Act::Resume
            | Act::Screenshot
            | Act::Volume
            | Act::Library
            | Act::QuitGameAsk(_)
            | Act::QuitGameDo(_)
            | Act::MenuButtonPick(_)
            | Act::MenuComboPreset(_)
            | Act::RecordMenuCombo
            | Act::RecordedComboUse(_)
            | Act::RecordComboAgain => {}
        }
        // Settings shown in the settings screen too (prompts, swap, fullscreen, theme …).
        self.refresh_settings(st);
    }

    /// Menu → Controller theme: the themes, previewed live while the cursor moves.
    fn open_theme_menu(&mut self, st: &AppState, now: f64) {
        self.theme_preview = Some(ThemePreview { original: st.controller_theme.clone(), cache: HashMap::new() });
        let mut m = Menu::new(MenuKind::Themes, "Controller theme", Self::theme_items(st), now).with_subtitle({
            let p = st.prompts();
            format!("Preview live · {} to keep · {} to cancel", p.name(Button::A), p.name(Button::B))
        });
        m.select_act(&Act::SetTheme(st.config.controller.theme.clone()));
        self.menus.push(m);
    }

    fn open_variant_menu(&mut self, st: &AppState, now: f64) {
        let mut items = vec![Item::value(
            "Default",
            if st.config.controller.variant.is_none() { "Current" } else { "" },
            Act::SetVariant(None),
        )];
        for k in st.controller_theme.file.variants.keys() {
            let on = st.config.controller.variant.as_deref() == Some(k.as_str());
            items.push(Item::value(k.clone(), if on { "Current" } else { "" }, Act::SetVariant(Some(k.clone()))));
        }
        self.menus.push(Menu::new(MenuKind::Variants, "Theme variant", items, now));
    }

    fn open_scheme_menu(&mut self, st: &AppState, now: f64) {
        let cur = st.config.controller.color_scheme.clone();
        let mut items =
            vec![Item::value("Theme's own", if cur.is_none() { "Current" } else { "" }, Act::SetScheme(None))];
        for (k, s) in &st.controller_theme.file.color_schemes {
            let on = cur.as_deref() == Some(k.as_str());
            items.push(Item::value(
                s.label_or(k).to_string(),
                if on { "Current" } else { "" },
                Act::SetScheme(Some(k.clone())),
            ));
        }
        self.menus.push(Menu::new(MenuKind::Schemes, "Theme colours", items, now));
    }

    fn toggle_fullscreen(&mut self, st: &mut AppState) {
        // Toggle what the window is now (a `--windowed` session shows Off), and make the
        // session follow the new setting.
        let fs = !st.wants_fullscreen();
        st.config.controller.fullscreen = fs;
        if fs {
            st.force_windowed = false;
        }
        st.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(fs));
        st.save_config();
        self.refresh_quick(st);
    }

    /// A game menu's "Details": its details page (from a home-page cover, B goes back home).
    fn show_details_of(&mut self, st: &mut AppState, id: GameId, now: f64) {
        self.close_menus(st);
        if self.screen == Screen::Home {
            self.open_home_game(st, id, now);
        } else if let Some(pos) = self.list_ids.iter().position(|&g| g == id) {
            self.game_sel = pos;
            self.open_details(st, now);
        }
    }

    /// "Hide game" takes two presses: the first arms it, a second one within
    /// HIDE_CONFIRM_SECS hides. Returns whether the game was hidden.
    fn hide_game(&mut self, st: &mut AppState, id: GameId, now: f64) -> bool {
        if !self.hide_armed.is_some_and(|(g, t)| g == id && now - t < HIDE_CONFIRM_SECS) {
            self.hide_armed = Some((id, now));
            self.set_hide_prompt(id, Some("Press again to hide"));
            return false;
        }
        self.hide_armed = None;
        let name = game_name(st, id);
        st.set_hidden(id, true);
        st.notify(format!("Hid {name} · to undo: Menu → Show hidden games"));
        self.close_menus(st);
        if self.screen == Screen::Details {
            self.go(self.details_back(), now);
        }
        true
    }

    fn unhide_game(&mut self, st: &mut AppState, id: GameId, now: f64) {
        let name = game_name(st, id);
        st.set_hidden(id, false);
        st.notify(format!("{name} is visible again"));
        self.close_menus(st);
        if self.screen == Screen::Details && self.cur_is_hidden() {
            self.go(self.details_back(), now);
        }
    }

    fn toggle_show_hidden(&mut self, st: &mut AppState, now: f64) {
        self.show_hidden = !self.show_hidden;
        if self.show_hidden {
            // Take the user straight to the Hidden Games collection.
            self.close_menus(st);
            self.coll_key = Some(hidden_collection().key());
            self.remember();
            self.details = None;
            self.go(Screen::Collections, now);
            self.refresh_list(&st.library);
        } else {
            self.refresh_quick(st);
        }
    }

    /// The on-screen keyboard on the game's launch options.
    fn edit_launch_options(&mut self, st: &AppState, id: GameId, now: f64) {
        let text =
            st.library.get(id).and_then(|g| g.launch_override.as_ref()).map(|o| o.options.clone()).unwrap_or_default();
        let mut o = Osk::new(
            Purpose::LaunchOptions(id),
            "Launch options  (%command% = the game; VAR=value sets env)",
            text,
            now,
        );
        o.nav(Nav::PageUp, false, false); // start with capitals: env vars are upper case
        self.osk = Some(o);
    }

    /// The on-screen keyboard on the game's custom command (starting from the source's).
    fn edit_launch_command(&mut self, st: &AppState, id: GameId, now: f64) {
        let g = st.library.get(id);
        let text = g
            .and_then(|g| g.launch_override.as_ref())
            .and_then(|o| o.command.clone())
            .or_else(|| g.and_then(crate::launch::command_template))
            .unwrap_or_default();
        self.osk =
            Some(Osk::new(Purpose::Command(id), "Custom command (replaces the source's launch command)", text, now));
    }

    /// One of the Launchers menu's switches / choices: change it, save, redraw the menu.
    fn step_launchers_setting(&mut self, st: &mut AppState, act: &Act) {
        use crate::config::{OneGameAtATime, WhenClosing};
        let l = &mut st.config.launchers;
        match act {
            Act::ToggleTrayIcon => st.config.tray_icon = !st.config.tray_icon,
            Act::ToggleHideLaunchers => l.hide_launchers = !l.hide_launchers,
            Act::ToggleCloseLaunchers => l.close_launchers_after = !l.close_launchers_after,
            Act::ToggleHideSteam => l.hide_steam = !l.hide_steam,
            Act::ToggleCloseSteam => l.set_close_steam(!l.close_steam_after),
            Act::ToggleKeepSteam => l.set_keep_steam(!l.keep_steam_for_controller),
            Act::CycleCloseWhilePlaying => {
                let all = WhenClosing::ALL;
                let now = WhenClosing::effective(st.config.when_closing, st.tray_shown());
                let i = all.iter().position(|v| *v == now).unwrap_or(0);
                st.config.when_closing = Some(all[(i + 1) % all.len()]);
            }
            _ => {
                let all = OneGameAtATime::ALL;
                let i = all.iter().position(|v| *v == l.one_game_at_a_time).unwrap_or(0);
                l.one_game_at_a_time = all[(i + 1) % all.len()];
            }
        }
        st.save_config();
        let items = Self::launchers_items(st);
        if let Some(m) = self.menus.iter_mut().rev().find(|m| m.kind == MenuKind::Launchers) {
            m.replace_items(items);
        }
    }

    /// Sort by …: back to the menu that opened it, which shows the new sort.
    fn set_sort(&mut self, s: Option<SortBy>) {
        self.sort = s;
        self.menus.pop();
        let label = self.sort_label();
        for m in &mut self.menus {
            for it in &mut m.items {
                if it.act == Some(Act::SortMenu) {
                    it.value = Some(label.clone());
                }
            }
        }
    }

    /// Lock / suspend / restart / power off: a confirmation first (Cancel focused).
    fn ask_power(&mut self, st: &AppState, p: Power, now: f64) {
        let mut text = match p {
            Power::Lock => "The screen locks; your games and Lodestar keep running.".to_string(),
            Power::Suspend => "Everything stays as it is; press the power button to wake it.".to_string(),
            _ => "Lodestar and all programs close.".to_string(),
        };
        if let Some(id) = crate::ui::launching::running(st)
            && !matches!(p, Power::Lock | Power::Suspend)
        {
            let name = game_name(st, id);
            text.push_str(&format!(" {name} is still running: unsaved progress is lost."));
        }
        let mut m = Menu::new(
            MenuKind::Confirm,
            p.question(),
            vec![
                Item::info(p.label(), text),
                Item::action(p.label(), Act::PowerDo(p)).danger(),
                Item::action("Cancel", Act::CloseMenu),
            ],
            now,
        )
        .centred();
        // A stray double press must not shut the computer down.
        m.select_act(&Act::CloseMenu);
        self.menus.push(m);
    }

    /// A random game: of the list on screen (Games / Details), else of all games; its details
    /// page opens.
    pub(super) fn random_game(&mut self, st: &mut AppState, now: f64) {
        // The home page has no list of its own (the one left over from an earlier grid isn't
        // what's on screen): pick from all games, and B comes back home.
        let from_home = self.screen == Screen::Home || (self.screen == Screen::Details && self.home.details_from_home);
        let from_list =
            matches!(self.screen, Screen::Games | Screen::Details) && !from_home && !self.list_ids.is_empty();
        if !from_list {
            let Some(i) = self.colls.iter().position(|c| c.collection.kind == CollectionKind::All) else {
                st.notify("No games to pick from");
                return;
            };
            self.coll_sel = i;
            self.coll_key = Some(self.colls[i].collection.key());
            self.enter_collection(&st.library, i, now);
        }
        let n = self.list_ids.len();
        if n == 0 {
            st.notify("No games to pick from");
            return;
        }
        let cur = if self.screen == Screen::Details { self.details.as_ref().map(|d| d.game) } else { None };
        let mut pick = random_below(n);
        if n > 1 && Some(self.list_ids[pick]) == cur {
            pick = (pick + 1 + random_below(n - 1)) % n;
        }
        self.game_sel = pick;
        self.remember();
        self.open_details(st, now);
        self.home.details_from_home = from_home && Self::has_home(st);
        if let Some(name) = st.library.get(self.list_ids[pick]).map(|g| g.display_name().to_string()) {
            st.notify(format!("Random pick: {name}"));
        }
    }

    /// Show (or clear, with `None`) the confirmation prompt on a game's "Hide game" item.
    pub(super) fn set_hide_prompt(&mut self, id: GameId, prompt: Option<&str>) {
        for m in &mut self.menus {
            for it in &mut m.items {
                if it.act == Some(Act::Hide(id)) {
                    it.value = prompt.map(str::to_string);
                }
            }
        }
    }

    pub(super) fn disarm_hide(&mut self) {
        if let Some((id, _)) = self.hide_armed.take() {
            self.set_hide_prompt(id, None);
        }
    }
}
