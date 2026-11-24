//! The controller menus' items: quick menu, launchers, sort, game options, completion,
//! controller support, launch options and themes.

use super::*;

impl ControllerUi {
    /// The quick menu (Start), Playnite Fullscreen's right menu: the running game, random game,
    /// desktop mode, rescan, sort, hidden games, settings, power options, quit. Everything to
    /// set up lives in Settings (settings.rs).
    pub(super) fn quick_items(&self, st: &AppState) -> Vec<Item> {
        let c = &st.config.controller;
        let on = |b: bool| if b { "On" } else { "Off" };
        let mut v = vec![];
        if let Some(id) = crate::ui::launching::running(st) {
            let name = crate::ui::game_name(st, id);
            v.push(Item::value(quit_label(st, id), name, Act::StopGame(id)).danger());
        }
        if !st.library.games.is_empty() {
            v.push(Item::action("Random game", Act::RandomGame));
        }
        v.push(Item::action("Switch to desktop mode", Act::SwitchDesktop));
        v.push(Item::value("Rescan library", if st.scan.running { "Scanning…" } else { "" }, Act::Rescan));
        if self.sorts_a_list() {
            v.push(Item::value("Sort by", self.sort_label(), Act::SortMenu));
        }
        let hidden = st.library.games.iter().filter(|g| g.hidden).count();
        if hidden > 0 || self.show_hidden {
            v.push(Item::value(format!("Show hidden games ({hidden})"), on(self.show_hidden), Act::ToggleShowHidden));
        }
        v.push(Item::action("Settings", Act::SettingsMenu));
        v.push(Item::action("About Lodestar", Act::Credits));
        for p in super::power::menu_items(&c.power, super::power::has_tool) {
            v.push(Item::action(p.label(), Act::PowerAsk(p)));
        }
        if c.allow_quit {
            v.push(Item::action("Quit Lodestar", Act::Quit).danger());
        }
        v
    }

    /// "Automatic (PlayStation)" / "Nintendo": the setting, with what Automatic picked.
    pub(super) fn button_prompts_value(st: &AppState) -> String {
        use crate::config::ButtonPrompts;
        match st.config.controller.button_prompts {
            ButtonPrompts::Auto => format!("Automatic ({})", st.prompts().style.label()),
            p => p.label().to_string(),
        }
    }

    /// Quick menu → Launchers: the same settings as desktop Settings → General → Launchers.
    pub(super) fn launchers_items(st: &AppState) -> Vec<Item> {
        use crate::config::OneGameAtATime;
        let l = &st.config.launchers;
        let on = |b: bool| if b { "On" } else { "Off" };
        vec![
            Item::info(
                "Store launchers",
                "Battle.net, EA app, Ubisoft Connect, Heroic … Lodestar only closes the ones it \
                 opened.",
            ),
            Item::value("Hide them while playing", on(l.hide_launchers), Act::ToggleHideLaunchers),
            Item::value("Close them when the game ends", on(l.close_launchers_after), Act::ToggleCloseLaunchers),
            Item::info(
                "Steam",
                "Started quietly in the background when it isn't open. Only a Steam Lodestar opened is \
                 closed, after it finished saving to the cloud.",
            ),
            Item::value("Minimise Steam while playing", on(l.hide_steam), Act::ToggleHideSteam),
            Item::value("Close Steam when the game ends", on(l.close_steam_after), Act::ToggleCloseSteam),
            Item::value("Keep Steam for controllers", on(l.keep_steam_for_controller), Act::ToggleKeepSteam),
            Item::value(
                "Another game running",
                match l.one_game_at_a_time {
                    OneGameAtATime::Ask => "Ask",
                    OneGameAtATime::CloseRunning => "Close it",
                    OneGameAtATime::AllowSeveral => "Allow several",
                },
                Act::CycleOneGame,
            ),
            Item::value("System tray icon", on(st.config.tray_icon), Act::ToggleTrayIcon),
            Item::value(
                "Closing Lodestar",
                crate::config::WhenClosing::effective(st.config.when_closing, st.tray_shown()).short_label(),
                Act::CycleCloseWhilePlaying,
            ),
        ]
    }

    /// "Sort by" changes something on screen: a game list, or the list a details page steps
    /// through with LB / RB (not the home page's rows, nor a details page opened from them).
    fn sorts_a_list(&self) -> bool {
        match self.screen {
            Screen::Games => true,
            Screen::Details => !self.home.details_from_home,
            Screen::Home | Screen::Collections => false,
        }
    }

    pub(super) fn sort_label(&self) -> String {
        let base = match self.sort {
            None => "Default".to_string(),
            Some(s) => s.label().to_string(),
        };
        if self.descending { format!("{base}, reversed") } else { base }
    }

    pub(super) fn sort_items(&self) -> Vec<Item> {
        let check = |b: bool| if b { "Active" } else { "" };
        let mut v = vec![Item::value("Default", check(self.sort.is_none()), Act::SetSort(None))];
        for s in SortBy::ALL {
            v.push(Item::value(s.label(), check(self.sort == Some(s)), Act::SetSort(Some(s))));
        }
        v.push(Item::value("Reverse order", if self.descending { "On" } else { "Off" }, Act::ToggleDescending));
        v
    }

    pub(super) fn game_option_items(&self, st: &AppState, id: GameId) -> Vec<Item> {
        let Some(g) = st.library.get(id) else {
            return vec![];
        };
        let running = crate::ui::launching::is_active(st, id);
        let mut items = vec![];
        let install = !running && !g.installed && st.can_install(id);
        if running {
            items.push(Item::action(quit_label(st, id), Act::StopGame(id)).danger());
        } else if let Some(p) = st.romm_progress(id) {
            // A RomM game downloading (the desktop shows a bar + Cancel under Download).
            let pct = p.fraction().map(|f| format!(" ({:.0} %)", f * 100.0)).unwrap_or_default();
            items.push(Item::action(format!("Cancel download{pct}"), Act::CancelDownload(id)).danger());
        } else if install {
            // Not installed: Install leads (Play would only fail or open the store anyway).
            let label = if g.source == crate::model::Source::Romm { "Download" } else { "Install" };
            items.push(Item::action(label, Act::Install(id)));
        } else {
            items.push(Item::action("Play", Act::Play(id)));
            // The game's other actions ("Settings", "Launcher" …); the main one is what Play runs.
            for (i, a) in g.edits.actions.iter().enumerate() {
                if a.play {
                    continue;
                }
                let name = a.name.trim();
                let label = if name.is_empty() { format!("Play: Action {}", i + 1) } else { format!("Play: {name}") };
                items.push(Item::action(label, Act::LaunchAction(id, i)));
            }
            items.extend(super::emu::quick_items(crate::emulation::registry::current().as_deref(), g));
        }
        if self.screen != Screen::Details {
            items.push(Item::action("Show details", Act::ShowDetails(id)));
        }
        if !g.art.is_empty() {
            items.push(Item::action("View artwork", Act::ViewArt(id)));
        }
        items.push(Item::action(
            if g.favorite { "Remove from favourites" } else { "Add to favourites" },
            Act::ToggleFavorite(id),
        ));
        items.push(Item::value(
            "Completion status",
            g.completion().map(|c| c.label()).unwrap_or(""),
            Act::CompletionMenu(id),
        ));
        items.push(Item::value("Controller support", controller_short(g.controller()), Act::ControllerMenu(id)));
        let lo = match &g.launch_override {
            Some(o) if !o.is_empty() => "Custom",
            _ => "",
        };
        items.push(Item::value("Launch options", lo, Act::LaunchOptions(id)));
        if self.sorts_a_list() {
            items.push(Item::value("Sort by", self.sort_label(), Act::SortMenu));
        }
        if !running && g.installed && st.can_uninstall(id) {
            items.push(Item::action("Uninstall", Act::Uninstall(id)));
        }
        if g.hidden {
            items.push(Item::action("Unhide game", Act::Unhide(id)));
        } else {
            items.push(Item::action("Hide game", Act::Hide(id)).danger());
        }
        items
    }

    /// Rebuild an open game-options menu after one of its values changed.
    pub(super) fn refresh_game_options(&mut self, st: &AppState, id: GameId) {
        let items = self.game_option_items(st, id);
        for m in self.menus.iter_mut().filter(|m| m.kind == MenuKind::GameOptions(id)) {
            m.replace_items(items.clone());
        }
    }

    pub(super) fn completion_items(st: &AppState, id: GameId) -> Vec<Item> {
        let cur = st.library.get(id).and_then(|g| g.completion());
        let mark = |b: bool| if b { "Current" } else { "" };
        let mut v = vec![Item::value("Not set", mark(cur.is_none()), Act::SetCompletion(id, None))];
        for c in CompletionStatus::ALL {
            v.push(Item::value(c.label(), mark(cur == Some(c)), Act::SetCompletion(id, Some(c))));
        }
        v
    }

    pub(super) fn controller_items(st: &AppState, id: GameId) -> Vec<Item> {
        let Some(g) = st.library.get(id) else {
            return vec![];
        };
        let cur = g.controller_override;
        let mark = |b: bool| if b { "Current" } else { "" };
        // "Automatic" names what the source says, so the user sees what they'd get back.
        let auto = match g.meta.controller {
            Some(c) => format!("Automatic ({})", controller_short(Some(c))),
            None => "Automatic (unknown)".to_string(),
        };
        let mut v = vec![Item::value(auto, mark(cur.is_none()), Act::SetController(id, None))];
        for c in [ControllerSupport::Full, ControllerSupport::Partial, ControllerSupport::None] {
            v.push(Item::value(controller_short(Some(c)), mark(cur == Some(c)), Act::SetController(id, Some(c))));
        }
        v
    }

    pub(super) fn launch_items(st: &AppState, id: GameId) -> Vec<Item> {
        let Some(g) = st.library.get(id) else {
            return vec![];
        };
        let ov = g.launch_override.clone().unwrap_or_default();
        let mut v = vec![];
        let src_name = g.source.display();
        let src_opts = g.source_launch_options.as_deref().map(str::trim).filter(|s| !s.is_empty());
        v.push(Item::info(
            format!("Set in {src_name}"),
            src_opts.map(str::to_string).unwrap_or_else(|| "(none)".into()),
        ));
        v.push(Item::info(
            "Lodestar launch options",
            if ov.options.trim().is_empty() { "(none)".to_string() } else { ov.options.clone() },
        ));
        if let Some(c) = ov.command.as_deref().filter(|c| !c.trim().is_empty()) {
            v.push(Item::info("Custom command (replaces the source's)", c.to_string()));
        }
        let will = match st.launch_preview(id) {
            Some(Ok(r)) => {
                let mut s = r.display.clone();
                for n in r.notes.iter().take(2) {
                    s.push_str("\n• ");
                    s.push_str(n);
                }
                s
            }
            Some(Err(e)) => format!("Can't launch: {e:#}"),
            None => String::new(),
        };
        v.push(Item::info("Will run", will));
        v.push(Item::action("Edit launch options…", Act::EditOptions(id)));
        v.push(Item::action(
            if ov.command.is_some() { "Edit custom command…" } else { "Use a custom command…" },
            Act::EditCommand(id),
        ));
        if let Some(s) = src_opts
            && ov.options.trim() != s
        {
            v.push(Item::action(format!("Copy options from {src_name}"), Act::UseSourceOptions(id)));
        }
        if !ov.is_empty() {
            v.push(Item::action("Reset to source defaults", Act::ClearOverride(id)).danger());
        }
        v
    }

    pub(super) fn refresh_launch_menu(&mut self, st: &AppState, id: GameId, _now: f64) {
        if let Some(m) = self.menus.iter_mut().rev().find(|m| m.kind == MenuKind::LaunchOptions(id)) {
            m.replace_items(Self::launch_items(st, id));
        }
        // The parent game-options entry shows "Custom" when an override exists.
        let custom = st.library.get(id).and_then(|g| g.launch_override.as_ref()).is_some_and(|o| !o.is_empty());
        for m in self.menus.iter_mut().filter(|m| m.kind == MenuKind::GameOptions(id)) {
            for it in &mut m.items {
                if it.act == Some(Act::LaunchOptions(id)) {
                    it.value = Some(if custom { "Custom".into() } else { String::new() });
                }
            }
        }
    }

    pub(super) fn theme_items(st: &AppState) -> Vec<Item> {
        st.themes
            .list()
            .iter()
            .filter(|t| t.meta.controller)
            .map(|t| {
                let tag = if t.dir.is_none() { "Built-in" } else { "" };
                let mut it = Item::value(t.meta.name.clone(), tag, Act::SetTheme(t.key.clone()));
                if t.key == st.config.controller.theme {
                    it.value = Some("Current".into());
                }
                it
            })
            .collect()
    }
}

/// One verb for stopping a game everywhere in controller mode: "Quit game", or "Cancel launch"
/// while it is still starting.
pub(super) fn quit_label(st: &AppState, id: GameId) -> &'static str {
    if crate::ui::launching::starting(st, id) { "Cancel launch" } else { "Quit game" }
}
