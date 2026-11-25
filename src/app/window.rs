//! The window: `App` runs one frame at a time for eframe. It polls the gamepad, applies the
//! controller safety gate (input/gate.rs), draws the desktop or controller UI over the shared
//! `AppState`, shows toasts and handles screenshot test runs and their probes.

use super::*;

pub struct App {
    st: AppState,
    desktop: DesktopUi,
    controller: ControllerUi,
    opts: LaunchOptions,
    frames: u64,
    screenshot_requested: bool,
    screenshot_settle: u32,
    started: Instant,
    /// `LODESTAR_FRAME_LOG=1`: log every frame with the time since the previous one and the
    /// repaint causes (finding repaint loops: an idle window must not produce frames).
    frame_log: Option<Instant>,
    /// Frames the desktop UI needs to replay `LODESTAR_DESKTOP_KEYS` (it injects one key every
    /// 3rd frame after frame 25); a screenshot waits for them before it starts settling.
    desktop_key_frames: u64,
    /// Library generation whose game names were checked for font coverage (CJK fallback).
    coverage_gen: u64,
    /// When the screenshot started settling (it also waits a minimum time, not just frames).
    settle_since: Option<Instant>,
    /// `LODESTAR_SCREENSHOT_HIDE` (see `screenshot_hide_test`).
    hide_test: HideTest,
    /// A focus event was seen this run (input/gate.rs: an unknown focus after one is no focus).
    focus_seen: bool,
    /// Last frame's focus (to see the moment it goes to another window).
    was_focused: Option<bool>,
    /// The main window's size / place / maximised, remembered (main_window.rs).
    main_win: crate::main_window::Tracker,
    /// Gamepad navigation drained in `logic` (which also runs while the window is hidden, so
    /// the Home button can summon it) for the next `ui` pass.
    pad_navs: Vec<Nav>,
    guide: input::gate::GuideDebounce,
    /// Screenshot-run performance probes (app/probe.rs).
    idle_probe: Option<probe::IdleProbe>,
    frame_stats: Option<probe::FrameStats>,
    /// `LODESTAR_LATENCY_PROBE` (screenshot runs): see app/probe.rs.
    latency: Option<probe::LatencyProbe>,
    /// `LODESTAR_INJECT_MS`: injected inputs this far apart; when the last one went in.
    inject_every: Option<(Duration, Option<Instant>)>,
}

fn hide_test_wanted() -> bool {
    std::env::var_os("LODESTAR_SCREENSHOT_HIDE").is_some_and(|v| !v.is_empty())
}

/// `App::screenshot_hide_test`'s progress.
#[derive(Clone, Copy, Debug)]
enum HideTest {
    Waiting,
    /// Hidden since, logic passes since.
    Hidden(Instant, u32),
    Done,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        paths: Paths,
        config: Config,
        db: Db,
        opts: LaunchOptions,
        instance: Option<crate::instance::Instance>,
    ) -> App {
        let ctx = cc.egui_ctx.clone();
        let t0 = Instant::now();
        let games = db.load_all().unwrap_or_else(|e| {
            log::error!("loading library: {e:#}");
            vec![]
        });
        let themes = ThemeManager::new(paths.themes_dir());
        let (desktop_theme, desktop_problem) =
            themes.load_with_scheme(&config.desktop_theme, config.desktop_color_scheme.as_deref());
        let (controller_theme, controller_problem) =
            themes.load_with_scheme(&config.controller.theme, config.controller.color_scheme.as_deref());
        let (bg_tx, bg_rx) = unbounded();
        let mode = opts.force_mode.unwrap_or(match config.start_mode {
            StartMode::Desktop => Mode::Desktop,
            StartMode::Controller => Mode::Controller,
        });
        ctx.set_zoom_factor(config.ui_scale.clamp(0.5, 3.0));
        let mut addons = crate::addons::AddonManager::new(paths.data_dir.join("addons"));
        addons.set_no_launch(opts.screenshot.is_some());
        if opts.screenshot.is_some() {
            // Screenshot runs never go online: every HTTP client refuses internet hosts (local
            // mock servers still work), and nothing queues metadata lookups.
            crate::art::net::set_local_only();
            crate::metadata::wikipedia::OFF.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        addons.refresh();
        let mut st = AppState {
            addons,
            images: ImageCache::new(&ctx, paths.thumbs_dir()),
            launcher: Launcher::new(&ctx),
            input: InputHub::new(&ctx, &config.controller),
            metadata: MetadataFetcher::new(&ctx, paths.art_dir(), &config.metadata),
            achievements: crate::achievements::Hub::new(&ctx, paths.cache_dir.join("achievements")),
            achievements_fake_for: None,
            achievements_published: false,
            art: crate::art::ArtService::new(
                &ctx,
                paths.art_dir(),
                paths.cache_dir.join("art-thumbs"),
                &config.metadata,
            ),
            art_state: Default::default(),
            auto_art: crate::art_app::AutoArt::new(opts.screenshot.is_none()),
            igdb_backfill: crate::igdb::Backfill { enabled: opts.screenshot.is_none(), ..Default::default() },
            wiki_backfill: crate::metadata::wikipedia::Backfill {
                enabled: opts.screenshot.is_none(),
                ..Default::default()
            },
            lookups: Default::default(),
            import_state: Default::default(),
            merge_suggestions: vec![],
            library: Library::default(),
            excl: Default::default(),
            fonts: FontRegistry::new(),
            themes,
            desktop_theme,
            controller_theme,
            mode,
            scan: ScanStatus::default(),
            toasts: vec![],
            ctx: ctx.clone(),
            bg_tx,
            bg_rx,
            pending_mode: None,
            quit: false,
            minimized_for_game: false,
            game_ended_at: None,
            style_generation: 1,
            scan_count: 0,
            rescan_queued: false,
            force_windowed: opts.windowed,
            no_minimize: opts.no_minimize,
            no_launch: opts.screenshot.is_some(),
            window_restorable: !is_wayland(|k| std::env::var_os(k)),
            launch_undo: Default::default(),
            rescan_at: None,
            failed_launches: Default::default(),
            non_play_sessions: Default::default(),
            edit_bg: Default::default(),
            steam_names: Default::default(),
            switch_prompt: None,
            switch: Default::default(),
            game_opts: Default::default(),
            running: Default::default(),
            shell: Default::default(),
            romm: Default::default(),
            update: Default::default(),
            welcome: Default::default(),
            paths,
            config,
            db,
        };
        let had_games = !games.is_empty();
        st.set_games(games);
        st.refresh_steam_owned();
        st.init_welcome(had_games);
        // The tray and answering later starts: the real GUI only (never screenshot runs).
        st.init_shell(opts.screenshot.is_none(), instance);
        st.apply_launcher_settings();
        st.init_game_options();
        // Lodestar's own updates: the daily check (never in screenshot runs).
        st.init_updates(opts.screenshot.is_none());
        // A theme named in the config that isn't there (deleted, typo): say so once, for the
        // mode it shows in first (the other one's message would be a stale toast by then).
        let problem = if mode == Mode::Controller {
            controller_problem.or(desktop_problem)
        } else {
            desktop_problem.or(controller_problem)
        };
        if let Some(p) = problem {
            st.notify_error(p);
        }
        if mode == Mode::Controller && st.wants_fullscreen() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
        }
        // Metadata add-ons are asked after the built-in sources.
        let (mgr, acfg) = (st.addons.clone(), st.config.addons.clone());
        st.metadata.set_addons(&mgr, &acfg);
        // Games still running from before (Lodestar was closed while they played) and Steam
        // games: shown as running again, their sessions recorded when they end.
        st.adopt_running_games();
        if st.config.scan.on_startup && !opts.no_scan {
            st.rescan();
        }
        log::info!("startup: {} games loaded in {:?}", st.library.games.len(), t0.elapsed());
        let scripted = opts.screenshot.is_some();
        App {
            st,
            desktop: DesktopUi::new(),
            controller: ControllerUi::new(),
            opts,
            frames: 0,
            screenshot_requested: false,
            screenshot_settle: 0,
            started: Instant::now(),
            frame_log: std::env::var_os("LODESTAR_FRAME_LOG").map(|_| Instant::now()),
            desktop_key_frames: desktop_key_frames(std::env::var("LODESTAR_DESKTOP_KEYS").ok().as_deref()),
            coverage_gen: 0,
            settle_since: None,
            hide_test: HideTest::Waiting,
            focus_seen: false,
            was_focused: None,
            main_win: Default::default(),
            pad_navs: vec![],
            guide: Default::default(),
            idle_probe: probe::IdleProbe::from_env().filter(|_| scripted),
            frame_stats: (scripted && probe::FrameStats::wanted()).then(Default::default),
            latency: (scripted && probe::LatencyProbe::wanted()).then(Default::default),
            inject_every: probe::inject_every().filter(|_| scripted).map(|d| (d, None)),
        }
    }

    fn apply_pending_mode(&mut self, ctx: &egui::Context) {
        if let Some(m) = self.st.pending_mode.take()
            && m != self.st.mode
        {
            if m == Mode::Controller {
                // Settings (maybe in its own window) closes; the other dialog windows wait.
                self.desktop.leave_desktop(&mut self.st);
            }
            self.st.mode = m;
            let fs = m == Mode::Controller && self.st.wants_fullscreen();
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(fs));
            self.st.style_generation += 1;
        }
    }

    /// Remember the main window as it is (desktop mode, not fullscreen / minimised / hidden;
    /// never in screenshot runs), written a moment after it stops changing.
    fn track_main_window(&mut self, ctx: &egui::Context) {
        if self.opts.screenshot.is_some() {
            return;
        }
        let mut seen = crate::main_window::seen(ctx, self.st.shell.hidden);
        seen.skip |= self.st.mode != Mode::Desktop || self.st.pending_mode.is_some();
        if self.main_win.track(&mut self.st.config.desktop.windows, &seen) {
            self.save_window_state();
        }
        // Written when due even if nothing else happens by then.
        if let Some(d) = self.main_win.due_in() {
            ctx.request_repaint_after(d);
        }
    }

    /// Just the config file (not `save_config`'s re-applying of every setting).
    fn save_window_state(&mut self) {
        if let Err(e) = self.st.config.save(&self.st.paths.config_file()) {
            log::warn!("couldn't save the window size: {e:#}");
        }
    }

    /// This frame's input situation (input/gate.rs decides from it what gets through).
    fn situation(&self, ctx: &egui::Context) -> input::gate::Situation {
        let c = &self.st.config.controller;
        // Any of Lodestar's windows (the main one, Settings … in their own window) counts.
        let focused = ctx.input(|i| {
            input::gate::lodestar_focus(
                i.viewport().focused,
                self.st.shell.hidden,
                i.viewport().minimized,
                other_windows(i, &self.desktop),
            )
        });
        input::gate::Situation {
            scripted: self.opts.screenshot.is_some(),
            focused,
            focus_seen: self.focus_seen,
            // `started()` = the game is actually up (not just "waiting for Steam"), so a Steam
            // launch stuck on an update dialog doesn't leave a focused Lodestar dead. While
            // Lodestar isn't focused, a launch still on its way (Steam's dialogs, before-start
            // scripts) counts too: those presses are meant for whatever is in front.
            game_up: self.st.launcher.started().is_some()
                || (self.st.launcher.running().is_some() && focused != Some(true)),
            background_input: c.background_input,
            others_clear: self.st.launcher.others_clear(crate::running_app::LOOK_FRESH),
            others_seen: self.st.launcher.others_seen(),
            hidden: self.st.shell.hidden || ctx.input(|i| i.viewport().minimized == Some(true)),
            guide_summons: c.guide_opens_lodestar,
            lodestar_menu: self.controller.wants_full_input(),
        }
    }

    /// Per `logic` pass (also while the window is hidden): set the gamepad gate, take the pad's
    /// navigation for the next frame, and act on the menu button (summon Lodestar, in-game
    /// menu, the menu button question).
    fn poll_pad(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().focused).is_some() {
            self.focus_seen = true;
        }
        let sit = self.situation(ctx);
        if focus_left_for_game(sit.game_up, self.was_focused, sit.focused) {
            self.controller.focus_left_for_game();
        }
        self.was_focused = sit.focused;
        self.st.input.set_gate(input::gate::gate(&sit));
        // Settings → Input "Lodestar's menu button": with the button combination, Home is
        // left to Steam and emulators; the pad thread then ignores it.
        self.st.input.set_home_button(self.st.config.controller.home_opens_menu());
        // Games started elsewhere while the controller drives Lodestar in the background (or
        // could drive desktop mode once it is in front). After a game ended, desktop mode's pad
        // waits for a look started since: that one doesn't wait for the interval.
        // Only with a controller connected: without one, desktop mode looks for running games
        // exactly as before (keyboard and mouse users see no new "is running" adoptions).
        let pad_connected = !self.st.input.gamepads().is_empty();
        let desktop_look = pad_connected && input::gate::desktop_look(&sit, self.desktop_pad_state(&sit));
        let since = if desktop_look { self.st.game_ended_at } else { None };
        self.st.background_look(sit.wants_background_look() || desktop_look, since);
        // Sounds and music (when turned on): controller mode, Lodestar in front, no game.
        let audio_active = self.st.mode == Mode::Controller
            && sit.is_focused()
            && !sit.hidden
            && !sit.game_up
            && self.st.launcher.running().is_none()
            && self.opts.screenshot.is_none();
        crate::media::audio::sync_app(&self.st, audio_active);
        let mut pads = self.st.input.drain();
        if pads.contains(&Nav::Guide) {
            pads.retain(|n| *n != Nav::Guide);
            if self.guide.press(Instant::now()) {
                let summon = !sit.is_focused();
                if !summon || self.st.config.controller.guide_opens_lodestar {
                    if summon {
                        log::info!("menu button: bringing Lodestar to the front");
                        self.st.show_window();
                    }
                    let now = ctx.input(|i| i.time);
                    self.controller.on_guide(&mut self.st, summon, now);
                    ctx.request_repaint();
                }
            }
        }
        // Only the latest pass's input: what came while the window was hidden is dropped.
        self.pad_navs = pads;
    }

    fn collect_navs(&mut self, ctx: &egui::Context) -> Vec<Nav> {
        let controller = self.st.mode == Mode::Controller;
        let sit = self.situation(ctx);
        let pads = std::mem::take(&mut self.pad_navs);
        let mut navs = if sit.scripted {
            // Scripted screenshot runs must never act on a real gamepad (evdev is read
            // regardless of which window has focus): a stray press could launch a real game.
            if controller {
                self.desktop.drop_pad();
                input::keyboard_nav(ctx)
            } else {
                self.scripted_desktop_pad(&sit)
            }
        } else {
            let mut v = pads;
            if controller && sit.is_focused() {
                v.extend(input::keyboard_nav(ctx));
            }
            // The safety rule (input/gate.rs): while a game runs, nothing but the Home button
            // unless Lodestar is focused, and then only the Playing overlay's buttons.
            let mut v = input::gate::filter(&sit, v);
            // Just after a game ended: only Home, for a moment.
            if just_ended(self.st.game_ended_at, Instant::now(), sit.game_up) {
                v.retain(|n| *n == Nav::Guide);
            }
            // (In controller mode the rule says no: presses left from desktop mode are dropped.)
            self.hand_to_desktop(&sit, self.desktop_pad_state(&sit), &v);
            v
        };
        // Injected navigation for scripted screenshots: one per frame once images settled.
        if self.opts.screenshot.is_some() && self.frames > 30 && !self.opts.inject.is_empty() {
            let due = match &mut self.inject_every {
                // Wall-clock pacing (LODESTAR_INJECT_MS): like a held direction's repeats.
                Some((every, last)) => {
                    let left = last.map_or(Duration::ZERO, |t| every.saturating_sub(t.elapsed()));
                    if left.is_zero() {
                        *last = Some(Instant::now());
                        ctx.request_repaint_after(*every);
                        true
                    } else {
                        ctx.request_repaint_after(left);
                        false
                    }
                }
                None => self.frames.is_multiple_of(4),
            };
            if due {
                let nav = self.opts.inject.remove(0);
                if let Some(p) = &mut self.latency {
                    p.injected(format!("{} {nav:?}", self.controller.probe_where()));
                }
                navs.push(nav);
                ctx.request_repaint();
            }
        }
        // Home presses that got here (keyboard-free; injected ones in screenshot runs) go the
        // same way as the pad's.
        if navs.contains(&Nav::Guide) {
            navs.retain(|n| *n != Nav::Guide);
            let now = ctx.input(|i| i.time);
            self.controller.on_guide(&mut self.st, false, now);
        }
        navs
    }

    /// Desktop mode's side of the pad rule (input/gate.rs `desktop_pad`).
    fn desktop_pad_state(&self, sit: &input::gate::Situation) -> input::gate::DesktopPad {
        let launcher = &self.st.launcher;
        input::gate::DesktopPad {
            enabled: self.st.mode == Mode::Desktop && self.st.config.controller.desktop_navigation,
            launching: launcher.running().is_some() && launcher.started().is_none(),
            settling: just_ended(self.st.game_ended_at, Instant::now(), sit.game_up),
            looked: input::gate::looked_since(launcher.others_looked_at(), self.st.game_ended_at),
            overlay: self.controller.overlay_open(),
        }
    }

    /// The pad's presses (already through the safety rule's filter) for desktop mode, when its
    /// own rule allows; otherwise presses still waiting there are dropped too. Returns whether
    /// the rule allowed.
    fn hand_to_desktop(&mut self, sit: &input::gate::Situation, d: input::gate::DesktopPad, navs: &[Nav]) -> bool {
        let allowed = input::gate::desktop_pad(sit, d);
        if allowed {
            self.desktop.take_pad(navs);
        } else {
            self.desktop.drop_pad();
        }
        allowed
    }

    /// Screenshot runs: the presses `LODESTAR_DESKTOP_KEYS` gave as `pad:…` (the real pad stays
    /// ignored), through the same rules as the pad's, in the situation the hook's `game:`,
    /// `focus:` and `seen:` make up. Returns what the safety rule let through, as the pad's
    /// path does (the in-game menu gets it; Home goes the usual way).
    fn scripted_desktop_pad(&mut self, sit: &input::gate::Situation) -> Vec<Nav> {
        let (navs, made_up) = self.desktop.scripted_pad();
        let sit = input::gate::Situation {
            scripted: false,
            focused: Some(made_up.focused),
            game_up: made_up.game_up,
            others_seen: made_up.seen,
            others_clear: !made_up.seen,
            ..*sit
        };
        // Screenshot runs never look for running games: the hook's `seen:` stands for the look.
        let looked = input::gate::looked_since(Some(made_up.looked_at), self.st.game_ended_at);
        let desktop = input::gate::DesktopPad { looked, ..self.desktop_pad_state(&sit) };
        let pressed = navs.clone();
        let mut navs = input::gate::filter(&sit, navs);
        if just_ended(self.st.game_ended_at, Instant::now(), sit.game_up) {
            navs.retain(|n| *n == Nav::Guide);
        }
        let allowed = self.hand_to_desktop(&sit, desktop, &navs);
        if !pressed.is_empty() {
            let to = if allowed { "the desktop" } else { "nothing (desktop rule)" };
            log::info!("scripted pad {pressed:?}: {navs:?} past the safety rule, to {to}; {desktop:?}");
        }
        navs
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) {
        if self.st.toasts.is_empty() {
            return;
        }
        let controller = self.st.mode == Mode::Controller;
        let pal = if controller { &self.st.controller_theme.file.palette } else { &self.st.desktop_theme.file.palette };
        // Desktop: above the status bar. Controller: bottom centre, above where themes put their
        // help bar and their bottom buttons (PS5-like game pages have Play there). The top
        // centre is where themes put the game's title.
        let (align, offset) = if controller {
            (egui::Align2::CENTER_BOTTOM, [0.0, -(ctx.content_rect().height() * 0.15).round()])
        } else {
            (egui::Align2::RIGHT_BOTTOM, [-16.0, -40.0])
        };
        // One width for the whole stack: the longest text unwrapped, within [MIN, MAX] and the
        // screen. Without it the area keeps the width of the previous (short) toast and a long one
        // wraps into a narrow column, a word per line.
        const SIZE: f32 = 15.0;
        let font = egui::FontId::proportional(SIZE);
        let widest = ctx.fonts_mut(|f| {
            self.st
                .toasts
                .iter()
                .map(|t| f.layout_no_wrap(t.text.clone(), font.clone(), egui::Color32::WHITE).size().x)
                .fold(0.0f32, f32::max)
        });
        let screen_w = ctx.content_rect().width();
        // Buttons on toasts: desktop mode only (controller mode says where to go instead).
        let buttons = !controller && self.st.toasts.iter().any(|t| t.action.is_some());
        // Room for the button next to the text.
        let width = toast_width(widest + if buttons { TOAST_BUTTON_ROOM } else { 0.0 }, screen_w);
        let label = self.st.update.button_label();
        let (accent, accent_text) = (pal.accent.to_egui(), pal.accent_text.to_egui());
        let mut clicked = None;
        egui::Area::new(egui::Id::unique("lodestar_toasts"))
            .anchor(align, offset)
            .order(egui::Order::Foreground)
            .interactable(buttons)
            .show(ctx, |ui| {
                for t in &self.st.toasts {
                    egui::Frame::new()
                        .fill(if t.error { pal.danger.to_egui() } else { pal.surface_alt.to_egui() })
                        .corner_radius(8.0)
                        .inner_margin(egui::Margin::symmetric(TOAST_PAD_X as i8, 10))
                        .show(ui, |ui| {
                            ui.set_width(width);
                            let text = egui::RichText::new(&t.text).color(pal.text.to_egui()).size(SIZE);
                            match t.action.filter(|_| buttons) {
                                Some(a) => {
                                    // The text on the left, the button at the right edge.
                                    ui.horizontal(|ui| {
                                        let b =
                                            egui::Button::new(egui::RichText::new(label).color(accent_text).strong())
                                                .fill(accent);
                                        ui.allocate_ui(
                                            egui::vec2((ui.available_width() - TOAST_BUTTON_ROOM).max(80.0), 0.0),
                                            |ui| ui.add(egui::Label::new(text).wrap()),
                                        );
                                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                            if ui.add(b).clicked() {
                                                clicked = Some(a);
                                            }
                                        });
                                    });
                                }
                                None => {
                                    ui.add(egui::Label::new(text).wrap());
                                }
                            }
                        });
                    ui.add_space(6.0);
                }
            });
        if let Some(a) = clicked {
            self.st.toast_action(a);
        }
    }

    /// `LODESTAR_SCREENSHOT_HIDE=1` (screenshot runs): hide the window the way the tray does once
    /// the UI is up, show it again a moment later, then take the screenshot as usual. Checks that
    /// a hidden window still runs the app's logic and comes back (a run that hangs here fails).
    /// `=quit`: Quit (as the tray's Quit does) while hidden instead; the run ends without a
    /// screenshot, and one that doesn't end means a hidden window can't be quit.
    fn screenshot_hide_test(&mut self, ctx: &egui::Context) {
        if self.opts.screenshot.is_none() || !hide_test_wanted() {
            return;
        }
        let platform = crate::winctl::Platform::detect();
        match self.hide_test {
            HideTest::Waiting if self.frames >= 20 => {
                eprintln!("screenshot run: hiding the window ({platform:?})");
                crate::winctl::hide(ctx, platform);
                self.hide_test = HideTest::Hidden(Instant::now(), 0);
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            HideTest::Hidden(t, passes)
                if t.elapsed() >= Duration::from_millis(1500)
                    && std::env::var_os("LODESTAR_SCREENSHOT_HIDE").is_some_and(|v| v == "quit") =>
            {
                eprintln!("screenshot run: quitting while hidden (logic passes while hidden: {passes})");
                self.st.quit();
                self.hide_test = HideTest::Done;
            }
            HideTest::Hidden(t, passes) if t.elapsed() >= Duration::from_millis(1500) => {
                eprintln!("screenshot run: showing the window again (logic passes while hidden: {passes})");
                crate::winctl::show(ctx, platform);
                self.hide_test = HideTest::Done;
            }
            HideTest::Hidden(t, passes) => {
                self.hide_test = HideTest::Hidden(t, passes + 1);
                ctx.request_repaint_after(Duration::from_millis(50));
            }
            _ => {}
        }
    }

    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(path) = self.opts.screenshot.clone() else { return };
        if self.screenshot_requested {
            return;
        }
        let hide_test_pending = !matches!(self.hide_test, HideTest::Done) && hide_test_wanted();
        let busy = self.st.scan.running
            || !self.opts.inject.is_empty()
            || hide_test_pending
            || self.desktop.debug_keys_pending();
        // A scan that takes very long (a hanging add-on runs up to its timeout) doesn't hold
        // the screenshot past the cap below.
        let capped = self.started.elapsed() > Duration::from_secs(25) && self.frames >= 20 + self.desktop_key_frames;
        if (busy && !capped) || self.frames < 20 + self.desktop_key_frames {
            self.screenshot_settle = 0;
            self.settle_since = None;
            ctx.request_repaint_after(Duration::from_millis(16));
            return;
        }
        self.screenshot_settle += 1;
        let settled_now = self.screenshot_settle >= self.opts.screenshot_frames.max(10)
            && self.settle_since.is_some_and(|t| t.elapsed() >= SCREENSHOT_MIN_SETTLE);
        if settled_now
            && let Some(p) = &mut self.idle_probe
            && p.running(ctx, self.frames)
        {
            return;
        }
        // Keep frames coming until things settle.
        ctx.request_repaint_after(Duration::from_millis(16));
        // Frames run unthrottled while something animates, so N frames can be a fraction of a
        // second: also wait out the UI's animations (carousel scrolls, fades) on the clock.
        let since = *self.settle_since.get_or_insert_with(Instant::now);
        let settled =
            self.screenshot_settle >= self.opts.screenshot_frames.max(10) && since.elapsed() >= SCREENSHOT_MIN_SETTLE;
        let timeout = self.started.elapsed() > Duration::from_secs(25);
        if settled || timeout {
            self.screenshot_requested = true;
            if let Some(p) = &self.latency {
                for line in p.summary() {
                    eprintln!("{line}");
                }
            }
            if let Some(f) = &self.frame_stats {
                eprintln!("{}", f.summary(20));
                let i = self.st.images.stats();
                eprintln!(
                    "image stats: {} textures, {:.1} MB GPU, {} pending, {} failed, {} evicted",
                    i.textures,
                    i.gpu_bytes as f64 / 1048576.0,
                    i.pending,
                    i.failed,
                    i.evicted_total
                );
            }
            let ctx2 = ctx.clone();
            if std::env::var_os("LODESTAR_RESTORE_WINDOW").is_some() {
                let (outer, inner) = ctx.input(|i| (i.viewport().outer_rect, i.content_rect()));
                eprintln!("main window: at {:?}, {:?} inside", outer.map(|r| r.min), inner.size());
            }
            // LODESTAR_OWN_WINDOWS=1: the dialogs' own windows too (`<name>-window<N>.png`).
            let windows = crate::ui::desktop::own_window_rects(ctx);
            ctx.request_screenshot(move |img| {
                let [w, h] = img.size;
                match image::save_buffer(&path, img.as_raw(), w as u32, h as u32, image::ColorType::Rgba8) {
                    Ok(()) => eprintln!("screenshot saved: {} ({w}x{h})", path.display()),
                    Err(e) => eprintln!("screenshot failed: {e}"),
                }
                for (n, r) in windows.into_iter().enumerate() {
                    let p = path.with_file_name(format!(
                        "{}-window{}.png",
                        path.file_stem().and_then(|s| s.to_str()).unwrap_or("shot"),
                        n + 1
                    ));
                    match crate::ui::desktop::capture_own_window(r) {
                        Ok((w, h, rgba)) => match image::save_buffer(&p, &rgba, w, h, image::ColorType::Rgba8) {
                            Ok(()) => eprintln!("window screenshot saved: {} ({w}x{h})", p.display()),
                            Err(e) => eprintln!("window screenshot failed: {e}"),
                        },
                        Err(e) => eprintln!("window screenshot failed: {e:#}"),
                    }
                }
                ctx2.send_viewport_cmd(egui::ViewportCommand::Close);
                ctx2.request_repaint();
            });
        }
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // Testing hooks: scripted pointer events (LODESTAR_DESKTOP_KEYS hover / click) and the
        // window focus the pad's made-up situation gives.
        if self.opts.screenshot.is_none() {
            return;
        }
        if let Some(events) = crate::ui::desktop::take_debug_pointer(ctx) {
            raw_input.events.extend(events);
        }
        if let Some(focused) = self.desktop.scripted_focus() {
            raw_input.focused = focused;
        }
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.st.poll_background();
        self.st.poll_game_options(ctx);
        self.st.run_due_rescan();
        self.st.expire_toasts();
        self.st.poll_shell();
        crate::ui::theme_options::poll(&mut self.st);
        self.apply_pending_mode(ctx);
        let (focused, close) = ctx.input(|i| {
            let others = other_windows(i, &self.desktop).any(|w| w.focused());
            (i.viewport().focused.unwrap_or(true) || others, i.viewport().close_requested())
        });
        self.st.on_focus(focused);
        self.poll_pad(ctx);
        self.track_main_window(ctx);
        self.screenshot_hide_test(ctx);
        // Closing may hide to the tray, minimise or ask instead, per Settings → General.
        let cancel = close && self.st.on_close_request(self.opts.screenshot.is_some());
        if cancel {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        }
        if self.st.quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        if self.st.quit || (close && !cancel) {
            self.desktop.on_quit(&mut self.st);
            if self.main_win.pending() {
                self.main_win = Default::default();
                self.save_window_state();
            }
            self.st.on_closing();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let t_ui = Instant::now();
        if let Some(f) = &mut self.frame_stats {
            f.push_whole(frame.info().cpu_usage);
        }
        let gap = self.latency.as_mut().map_or(0.0, |p| p.frame_start());
        self.frames += 1;
        self.ui_inner(ui);
        if let Some(f) = &mut self.frame_stats {
            f.push(t_ui.elapsed());
        }
        if let Some(p) = &mut self.latency {
            p.frame_end(t_ui.elapsed(), gap, self.st.images.waiting(), &self.controller.probe_where());
        }
    }
}

impl App {
    fn ui_inner(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        if let Some(prev) = &mut self.frame_log {
            let now = Instant::now();
            let events: Vec<String> =
                ctx.input(|i| i.raw.events.iter().map(|e| format!("{e:?}").chars().take(48).collect()).collect());
            log::info!(
                "frame {} +{:.1}ms causes {:?} events {:?}",
                self.frames,
                now.duration_since(*prev).as_secs_f64() * 1000.0,
                ctx.repaint_causes(),
                events
            );
            *prev = now;
        }
        self.st.images.begin_frame();
        if self.frame_stats.is_some() && probe::bump_wanted(self.frames) {
            // LODESTAR_FRAME_STATS=bump: act as if a game changed (what each metadata / art
            // result does) to time the caches that rebuild per library generation.
            self.st.library.generation += 1;
            ctx.request_repaint();
        }
        {
            let (d, c) = (self.st.desktop_theme.clone(), self.st.controller_theme.clone());
            self.st.fonts.ensure(&ctx, &[&d, &c]);
        }
        if self.coverage_gen != self.st.library.generation {
            // Both modes: load the CJK fallback font when a game name needs it (no-op otherwise).
            self.coverage_gen = self.st.library.generation;
            for g in &self.st.library.games {
                self.st.fonts.request_coverage(g.display_name());
            }
        }
        let navs = self.collect_navs(&ctx);
        match self.st.mode {
            Mode::Desktop => {
                self.desktop.show(ui, &mut self.st);
                // The in-game menu / the menu button screens as a centred panel.
                self.controller.desktop_overlay(&ctx, &mut self.st, &navs);
            }
            Mode::Controller => self.controller.show(ui, &mut self.st, &navs),
        }
        // The dialog windows were drawn just now, with their up-to-date focus: a window that
        // lost the focus (to a game) narrows the gate at once, not a pass later.
        self.regate(&ctx);
        self.draw_toasts(&ctx);
        self.handle_screenshot(&ctx);
    }

    /// The gamepad gate again when Lodestar's focus changed since `poll_pad` (see `ui_inner`).
    fn regate(&mut self, ctx: &egui::Context) {
        let sit = self.situation(ctx);
        if sit.focused == self.was_focused {
            return;
        }
        if focus_left_for_game(sit.game_up, self.was_focused, sit.focused) {
            self.controller.focus_left_for_game();
        }
        self.was_focused = sit.focused;
        self.st.input.set_gate(input::gate::gate(&sit));
        ctx.request_repaint();
    }
}

/// Lodestar's windows other than the main one (dialogs in their own window,
/// ui/desktop/own_window.rs) as the main window's pass sees them: eframe's report (gone with
/// the window) and what each window's own latest pass saw.
fn other_windows<'a>(
    i: &'a egui::InputState,
    desktop: &'a crate::ui::desktop::DesktopUi,
) -> impl Iterator<Item = input::gate::OtherWindow> + 'a {
    i.raw.viewports.iter().filter(|(id, _)| **id != egui::ViewportId::ROOT).map(|(id, v)| input::gate::OtherWindow {
        reported: v.focused,
        minimised: v.minimized,
        latest: desktop.own_window_focus(*id),
    })
}

/// Whether winit will run on Wayland: it picks Wayland whenever `WAYLAND_DISPLAY` or
/// `WAYLAND_SOCKET` is set and non-empty (eframe doesn't force a backend).
pub(crate) fn is_wayland(env: impl Fn(&str) -> Option<std::ffi::OsString>) -> bool {
    ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"].iter().any(|k| env(k).is_some_and(|v| !v.is_empty()))
}

const TOAST_PAD_X: f32 = 14.0;
/// Width kept next to a toast's text for its button ("Update", "Restart now").
const TOAST_BUTTON_ROOM: f32 = 110.0;

/// Minimum wall-clock time a screenshot waits after the last injected key / busy state.
const SCREENSHOT_MIN_SETTLE: Duration = Duration::from_millis(700);

/// Lodestar had the focus while a game runs and now hasn't, or can't tell any more. Unknown
/// counts as gone, as it does for the gate (an unknown focus with a game up is unfocused): the
/// menus left open must then stop taking the pad, as for a focus that went to the game.
pub(super) fn focus_left_for_game(game_up: bool, was: Option<bool>, now: Option<bool>) -> bool {
    game_up && was == Some(true) && now != Some(true)
}

/// A game ended a moment ago (and none is up again): only Home acts until it settles.
pub(super) fn just_ended(ended_at: Option<Instant>, now: Instant, game_up: bool) -> bool {
    !game_up && ended_at.is_some_and(|t| now.saturating_duration_since(t) < GAME_END_SETTLE)
}

/// Text width of the toast stack: fits the longest toast unwrapped when it can, at least 240 px
/// (short toasts look the same), at most 460 px (longer ones wrap into a few lines) and never
/// wider than the screen allows.
pub(super) fn toast_width(widest_text: f32, screen_w: f32) -> f32 {
    let room = (screen_w - 48.0 - 2.0 * TOAST_PAD_X).max(120.0);
    widest_text.ceil().clamp(240.0, 460.0).min(room)
}

/// See `App::desktop_key_frames`.
pub(super) fn desktop_key_frames(keys: Option<&str>) -> u64 {
    let n = keys.map_or(0, |k| k.split(',').filter(|k| !k.is_empty()).count() as u64);
    if n == 0 { 0 } else { 10 + 3 * n }
}
