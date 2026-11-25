//! Testing hooks for scripted screenshots: injected keys, text and pointer events
//! (`LODESTAR_DESKTOP_KEYS`) and the dialogs / pages opened on start (`LODESTAR_DESKTOP_OPEN`).
//!
//! Controller presses in desktop mode: `pad:<name>` (the `--inject` names, `pad:down*5` five
//! times) go through the pad's rule (input/gate.rs `desktop_pad`) in a made-up situation:
//! `game:up` / `game:down` (a game up; down also starts the settle after a game), `focus:off` /
//! `focus:on` (default on; egui is told the window has the focus or not, too), `seen:on` /
//! `seen:off` (a look for running games finished just now and saw a game, or nothing; the first
//! of these tokens stands for a clean look then, and after `game:down` the pad waits for the
//! next `seen:`, as it waits for a new look in a real run). `wait:<ms>` pauses the keys for that
//! long.

use super::*;

/// Testing hook: pointer events from `LODESTAR_DESKTOP_KEYS` (hover / click), one batch per
/// frame, fed to egui as raw input by `App::raw_input_hook` (screenshot runs only).
pub(super) static DEBUG_POINTER: std::sync::Mutex<std::collections::VecDeque<Vec<egui::Event>>> =
    std::sync::Mutex::new(std::collections::VecDeque::new());

/// The next batch of injected pointer events (testing hook), if any.
pub(crate) fn take_debug_pointer(ctx: &egui::Context) -> Option<Vec<egui::Event>> {
    let mut q = DEBUG_POINTER.lock().unwrap_or_else(|e| e.into_inner());
    let batch = q.pop_front();
    if !q.is_empty() {
        ctx.request_repaint();
    }
    batch
}

/// Push a synthetic key / text event (testing hook for scripted screenshots).
pub(super) fn inject_debug_key(ctx: &egui::Context, k: &str) {
    ctx.request_repaint();
    if let Some(t) = k.strip_prefix("text:") {
        // Keys are comma-separated: "%2C" types a comma.
        let t = t.replace("%2C", ",");
        ctx.input_mut(|i| i.events.push(egui::Event::Text(t)));
        return;
    }
    // "drag:x1:y1:x2:y2" (points): press, move in steps, release (one batch per frame).
    if let Some(rest) = k.strip_prefix("drag:") {
        let v: Vec<f32> = rest.split(':').filter_map(|n| n.parse().ok()).collect();
        if let [x1, y1, x2, y2] = v[..] {
            let at = |t: f32| egui::pos2(x1 + (x2 - x1) * t, y1 + (y2 - y1) * t);
            let press = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            let mut q = DEBUG_POINTER.lock().unwrap_or_else(|e| e.into_inner());
            q.push_back(vec![egui::Event::PointerMoved(at(0.0))]);
            q.push_back(vec![press(at(0.0), true)]);
            for i in 1..=10 {
                q.push_back(vec![egui::Event::PointerMoved(at(i as f32 / 10.0))]);
            }
            q.push_back(vec![press(at(1.0), false)]);
        }
        return;
    }
    // "wheel:x:y:dy" (points): the mouse wheel over x, y (negative dy scrolls down).
    if let Some(rest) = k.strip_prefix("wheel:") {
        let v: Vec<f32> = rest.split(':').filter_map(|n| n.parse().ok()).collect();
        if let [x, y, dy] = v[..] {
            let mut q = DEBUG_POINTER.lock().unwrap_or_else(|e| e.into_inner());
            q.push_back(vec![egui::Event::PointerMoved(egui::pos2(x, y))]);
            q.push_back(vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, dy),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers: Modifiers::NONE,
            }]);
        }
        return;
    }
    // "hover:x:y" / "click:x:y" / "dblclick:x:y" / "rclick:x:y" / "ctrlclick:x:y" /
    // "shiftclick:x:y" (points):
    // pointer events. They go in through the next frames' raw input (`take_debug_pointer`):
    // events pushed mid-frame don't move egui's pointer.
    if let Some((kind, xy)) = k.split_once(':')
        && matches!(kind, "hover" | "click" | "dblclick" | "rclick" | "ctrlclick" | "shiftclick")
        && let Some((x, y)) = xy.split_once(':')
        && let (Ok(x), Ok(y)) = (x.parse::<f32>(), y.parse::<f32>())
    {
        let pos = egui::pos2(x, y);
        let primary = kind != "rclick";
        let mods = match kind {
            "ctrlclick" => Modifiers::COMMAND,
            "shiftclick" => Modifiers::SHIFT,
            _ => Modifiers::NONE,
        };
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: if primary { egui::PointerButton::Primary } else { egui::PointerButton::Secondary },
            pressed,
            modifiers: mods,
        };
        let mut q = DEBUG_POINTER.lock().unwrap_or_else(|e| e.into_inner());
        // Held modifiers are input state (ModifiersChanged), set before the press and released
        // after the click.
        let held = !mods.is_none();
        let mut first = vec![egui::Event::PointerMoved(pos)];
        if held {
            first.insert(0, egui::Event::ModifiersChanged(mods));
        }
        q.push_back(first);
        if kind != "hover" {
            q.push_back(vec![button(true)]);
            q.push_back(vec![button(false)]);
        }
        if kind == "dblclick" {
            // The second click in the very next frames (well within egui's double-click time).
            q.push_back(vec![button(true)]);
            q.push_back(vec![button(false)]);
        }
        if held {
            q.push_back(vec![egui::Event::ModifiersChanged(Modifiers::NONE)]);
        }
        return;
    }
    if k == "tab" {
        // Injected events arrive after egui handled this pass's Tab presses: move focus the way
        // a real Tab would instead.
        ctx.memory_mut(|m| m.move_focus(egui::FocusDirection::Next));
        return;
    }
    let (mods, name) = match k.strip_prefix("ctrl+") {
        Some(rest) => (Modifiers::COMMAND, rest),
        None => (Modifiers::NONE, k),
    };
    let key = match name {
        "left" => Key::ArrowLeft,
        "right" => Key::ArrowRight,
        "up" => Key::ArrowUp,
        "down" => Key::ArrowDown,
        "enter" => Key::Enter,
        "esc" => Key::Escape,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "insert" => Key::Insert,
        "delete" => Key::Delete,
        "pagedown" => Key::PageDown,
        "pageup" => Key::PageUp,
        "home" => Key::Home,
        "end" => Key::End,
        "f" => Key::F,
        "a" => Key::A,
        "tab" => Key::Tab,
        _ => return,
    };
    ctx.input_mut(|i| {
        i.events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: mods });
    });
}

impl DesktopUi {
    /// Testing hooks from the environment (read on the first frame).
    pub(super) fn read_debug_env(&mut self) {
        if let Ok(what) = std::env::var("LODESTAR_DESKTOP_OPEN")
            && !what.is_empty()
        {
            self.debug_open = Some(what);
        }
        self.debug_bulk_cancel = std::env::var("LODESTAR_DESKTOP_BULK_CANCEL_AT").ok().and_then(|v| v.parse().ok());
        if let Ok(keys) = std::env::var("LODESTAR_DESKTOP_KEYS") {
            self.debug_keys = keys.split(',').map(str::to_string).filter(|k| !k.is_empty()).collect();
        }
    }

    /// Send the next `LODESTAR_DESKTOP_KEYS` key, one every third frame once the window has
    /// settled.
    pub(super) fn debug_next_key(&mut self, ctx: &egui::Context, st: &mut AppState) {
        if self.debug_keys.is_empty() || self.frame <= 25 || !self.frame.is_multiple_of(3) {
            return;
        }
        // Keys after "open:" are for what it opens.
        if self.debug_open_now || self.pad_script_waits(ctx) {
            return;
        }
        let k = self.debug_keys.remove(0);
        match k.strip_prefix("open:") {
            // "open:<what>": a LODESTAR_DESKTOP_OPEN target opened between keys, so later
            // keys can act inside it.
            Some(what) => {
                self.debug_open = Some(what.to_string());
                self.debug_open_now = true;
            }
            None if self.pad_script_key(st, &k) => ctx.request_repaint(),
            None => inject_debug_key(ctx, &k),
        }
    }

    /// The screenshot waits for the keys (`*N` and `wait:` make more frames than one per key)
    /// and for the pad presses they made.
    pub(crate) fn debug_keys_pending(&self) -> bool {
        !self.debug_keys.is_empty() || self.pad_pending()
    }

    /// Per frame: cancel a bulk download at `LODESTAR_DESKTOP_BULK_CANCEL_AT`, and open the
    /// `LODESTAR_DESKTOP_OPEN` target once the keys before it were sent.
    pub(super) fn debug_tick(&mut self, ctx: &egui::Context, st: &mut AppState) {
        if let Some(n) = self.debug_bulk_cancel
            && st.art_state.bulk.as_ref().is_some_and(|b| b.running && b.done >= n)
        {
            self.debug_bulk_cancel = None;
            st.cancel_bulk_art();
        }
        let debug_ready = (self.debug_keys.is_empty() || self.debug_open_now) && self.frame > 30;
        if self.debug_open.is_some() && !debug_ready {
            // Keep frames coming until it opens (a run without --screenshot, e.g. measuring
            // idle CPU with a dialog open, would otherwise go idle first).
            ctx.request_repaint();
        }
        if let (true, Some(what), Some(id)) = (debug_ready, self.debug_open.clone(), self.selected) {
            self.debug_open = None;
            self.debug_open_now = false;
            self.open_debug_target(ctx, st, what, id);
        }
    }

    /// Open a `LODESTAR_DESKTOP_OPEN` target for the selected game `id` (screenshots):
    /// launch, launch-direct, rename, tags, add, about, remove, menu, menu-controller, appmenu,
    /// viewer[-cover|-hero|-logo|-icon|-banner][-zoom] (the artwork viewer),
    /// art[-cover|-hero|-logo|-icon|-banner][-online][-pick][-reset][-close][:term] (with
    /// LODESTAR_DESKTOP_ART_INPUT=<path or URL> submitted through the field), bulkart[-run], pencil,
    /// settings[-library|-appearance|-controller|-metadata], edit[-<tab>] (Edit Game Details),
    /// playmenu (Play's drop-down), appmenu-add (Add game submenu), add-program:<file>,
    /// add-scan[:<folder>], merge-banner, settings-libraries, settings-hidden[:<search>],
    /// notagame-like (the "…and everything named like this" dialog), bulk-options,
    /// addons[-libraries|-metadata|-generic|-builtin|-themes|-controller-themes|-browse|-updates][:<id>[:enable]],
    /// playnite-import:<path> (Add-ons theme import + report),
    /// addons-install:<folder or .zip>, starting / switch (a launch on its way, the "another
    /// game is running" prompt; faked, see ui/launching.rs), settings-emulation (Settings →
    /// Emulation), settings-emulation-<demo|add|folder|bios:<platform>>[+…] (made-up
    /// emulators / Add emulator… / Add folder… / a console's BIOS panel; emulation_ui.rs
    /// EmuPage::debug), roms-import[-demo][:<folder>] (Add game → Emulated games…, on that
    /// folder without the file dialog; -demo: its library games as the preview), arrange
    /// (Arrange mode on the selected game's page), sortmenu / groupmenu / presetmenu (the
    /// top bar's Sort, Group by and quick preset drop-downs), random (the random game pick),
    /// achievements-all (the page's Achievements with Show all open; made-up progress with
    /// LODESTAR_FAKE_ACHIEVEMENTS), settings-achievements[-checking|-signedin|-error],
    /// theme-creator[:<theme>[:<option>…]] (the theme creator; a built-in theme is copied first),
    /// update (Lodestar's own "What's new in <version>", made up with LODESTAR_FAKE_UPDATE),
    /// update-check (menu → Check for updates…, answered by LODESTAR_FAKE_UPDATE).
    fn open_debug_target(&mut self, ctx: &egui::Context, st: &mut AppState, what: String, id: GameId) {
        if let Some(rest) = what.strip_prefix("theme-creator") {
            // theme-creator[:<theme>[:<option>…]] (theme_creator.rs `CreatorDlg::debug`).
            self.dlg.creator = Some(theme_creator::CreatorDlg::debug(st, rest.trim_start_matches(':')));
        } else if let Some(rest) = what.strip_prefix("stats") {
            // stats[-filter][-hidden][-time]: the Statistics page (on the current filter,
            // with hidden games, bars by play time).
            self.stats = Some(stats::StatsPage::debug(rest));
        } else if let Some(rest) = what.strip_prefix("edit") {
            // edit[-general|-advanced|-media|-links|-installation|-actions|-scripts]
            let tab = edit::Tab::from_key(rest.trim_start_matches('-'));
            self.actions.push(Action::Edit(id, tab));
        } else if self.debug_open_add(ctx, st, &what)
            || self.debug_open_addons(st, &what)
            || self.debug_open_settings(st, &what, id)
            || self.debug_open_artwork(ctx, st, &what, id)
        {
        } else if what == "merge-banner" {
            // The "Same game?" banner for the selected game and the next one in the view
            // (shown only; screenshot runs never click it).
            let other = self.view.iter().map(|&i| st.library.games[i].id).find(|&g| g != id);
            if let Some(other) = other {
                st.merge_suggestions.push(crate::addgame::MergeSuggestion { manual_id: id, scanned_id: other });
            }
        } else if what == "remove-missing" {
            // The confirmation for every missing game (else the selected one).
            let all: Vec<GameId> = st.library.missing().map(|g| g.id).collect();
            self.actions.push(Action::RemoveMissing(if all.is_empty() { vec![id] } else { all }));
        } else if what == "notagame-like" {
            self.actions.push(Action::NotAGameLike(id));
        } else if what == "bulk-options" {
            // Game options for the multi-selection (or the whole view).
            let ids = if self.multi.len() > 1 {
                self.multi_ids(st)
            } else {
                self.view.iter().map(|&i| st.library.games[i].id).collect()
            };
            self.actions.push(Action::BulkOptions(ids));
        } else if what == "credits-all" {
            // Credits with every folded section open.
            self.dlg.credits = Some(credits::CreditsDlg { expand: true });
        } else if what == "whatsnew-menu" {
            self.dlg.whats_new = true;
            self.dlg.debug_export_menu = true;
        } else if what == "whatsnew-saving" {
            self.dlg.whats_new = true;
            self.dlg.debug_export_menu = true;
            self.dlg.debug_export_saving = true;
        } else if let Some(path) = what.strip_prefix("whatsnew-export:") {
            self.dlg.whats_new = true;
            self.dlg.changelog_export_to = Some(std::path::PathBuf::from(path));
        } else if what == "update" {
            // "What's new in <version>" of Lodestar's own update (with LODESTAR_FAKE_UPDATE).
            st.update.show_notes = true;
        } else if what == "update-check" {
            st.check_updates_from_menu();
        } else if what == "appmenu-add" {
            self.open_add_menu = true;
        } else if what == "menu-controller" {
            DEBUG_CONTROLLER_SUBMENU.store(true, std::sync::atomic::Ordering::Relaxed);
            self.debug_menu = Some("menu".into());
        } else if what == "choose-play" {
            emulation_ui::choose::open_at(ctx, st, id, egui::pos2(380.0, 300.0));
        } else if what == "add-as-rom" {
            emulation_ui::addrom::open(ctx, id);
        } else if what == "achievements-all" {
            // The game page's Achievements with "Show all" open.
            ctx.data_mut(|d| d.insert_temp(details::show_all_id(id), true));
        } else if what == "random" {
            self.actions.push(Action::RandomGame);
        } else if matches!(what.as_str(), "menu" | "appmenu" | "playmenu" | "sortmenu" | "groupmenu" | "presetmenu") {
            self.debug_menu = Some(what);
        } else if what == "arrange" {
            if let Some(g) = st.library.get(id) {
                self.arrange = Some(details::Arrange::new(g));
            }
        } else if crate::ui::launching::debug_fake(st, id, &what) {
            // starting / switch: the launch status or the switch prompt for this game.
        } else if what == "launch-direct" {
            if let Some(g) = st.library.get(id) {
                let mut d = dialogs::LaunchDlg::new(g);
                if g.source == Source::Faugus {
                    d.fill_faugus_direct(&st.config);
                } else if g.source == Source::Steam {
                    d.fill_steam_direct(&st.config);
                } else {
                    d.fill_base_command();
                }
                self.dlg.launch = Some(d);
            }
        } else {
            self.actions.push(match what.as_str() {
                "launch" => Action::LaunchOptions(id),
                "rename" => Action::Rename(id),
                "tags" => Action::EditTags(id),
                "add" => Action::AddManual,
                "about" => Action::About,
                "credits" => Action::Credits,
                "whatsnew" => Action::WhatsNew,
                "remove" => Action::Remove(id),
                "settings-library" | "settings-libraries" => Action::Settings(settings::Tab::Library),
                "settings-games" => Action::Settings(settings::Tab::Games),
                "settings-emulation" => Action::Settings(settings::Tab::Emulation),
                "settings-appearance" => Action::Settings(settings::Tab::Appearance),
                "settings-controller" => Action::Settings(settings::Tab::Controller),
                "settings-metadata" => Action::Settings(settings::Tab::Metadata),
                _ => Action::Settings(settings::Tab::General),
            });
        }
    }

    /// The Add game dialogs: add-cloud…, add-program:<file>, roms-import…, add-scan….
    fn debug_open_add(&mut self, ctx: &egui::Context, st: &mut AppState, what: &str) -> bool {
        if let Some(rest) = what.strip_prefix("add-cloud") {
            // add-cloud[-more][-<service key>][:<link> | :name=<name>]: Add game → Cloud game….
            let (flags, arg) = rest.split_once(':').unwrap_or((rest, ""));
            let (link, name) = match arg.strip_prefix("name=") {
                Some(n) => (None, Some(n)),
                None => (Some(arg.to_string()).filter(|a| !a.is_empty()), None),
            };
            let mut d = add_cloud::CloudDlg::open(ctx, link);
            for f in flags.split('-').filter(|f| !f.is_empty()) {
                d = if f == "more" { d.with_more() } else { d.with_service(f) };
            }
            if let Some(n) = name {
                d = d.with_name(n);
            }
            self.dlg.cloud = Some(d);
        } else if let Some(path) = what.strip_prefix("add-program:") {
            // The Pick a program dialog as if `path` had been picked (no file dialog).
            self.dlg.program = Some(add::ProgramDlg::for_path(ctx, std::path::PathBuf::from(path)));
        } else if let Some(rest) = what.strip_prefix("roms-import") {
            // roms-import (Add game → Emulated games…) or roms-import:<folder> (that folder);
            // roms-import-demo:<folder>: its library games shown as if the folder were new.
            let (demo, rest) = rest.strip_prefix("-demo").map_or((false, rest), |r| (true, r));
            let folder = rest.strip_prefix(':').map(std::path::PathBuf::from);
            let mut d = emulation_import::RomImportDlg::open(st, folder);
            if demo {
                d.demo_from_library(st);
            }
            self.dlg.roms = Some(d);
        } else if let Some(rest) = what.strip_prefix("add-scan") {
            // add-scan (Detect installed) or add-scan:<folder> (Scan folder).
            let folder = rest.strip_prefix(':').map(std::path::PathBuf::from);
            self.dlg.import = Some(add::ImportDlg::open(st, folder));
        } else {
            return false;
        }
        true
    }

    /// The Add-ons window: theme imports, installing from a folder, a page / add-on.
    fn debug_open_addons(&mut self, st: &mut AppState, what: &str) -> bool {
        if let Some(path) = what.strip_prefix("playnite-import:") {
            // Add-ons → Desktop themes, importing this Playnite theme as if it had been picked.
            let mut d = addons::AddonsDlg::new(addons::Section::DesktopThemes);
            d.themes.start_import_playnite(st, std::path::PathBuf::from(path));
            self.dlg.addons = Some(d);
        } else if let Some(path) = what.strip_prefix("addons-install:") {
            // Install from a folder / .zip as if it had been picked (no file dialog).
            let mut d = addons::AddonsDlg::new(addons::Section::All);
            d.debug_install = Some(std::path::PathBuf::from(path));
            self.dlg.addons = Some(d);
        } else if let Some(rest) = what.strip_prefix("addons") {
            // addons[-libraries|-metadata|-generic|-builtin|-themes|-controller-themes|-browse|-updates]
            // [:<id>] (that add-on expanded) [:<id>:enable] (its permission prompt).
            let (page, rest) = rest.split_once(':').unwrap_or((rest, ""));
            let mut d = addons::AddonsDlg::new(addons::Section::from_key(page.trim_start_matches('-')));
            let (sel, then) = rest.split_once(':').unwrap_or((rest, ""));
            if !sel.is_empty() {
                d.selected = Some(sel.to_string());
                if then == "enable" {
                    d.confirm_enable = Some(sel.to_string());
                }
            }
            self.dlg.addons = Some(d);
        } else {
            return false;
        }
        true
    }

    /// Settings pages that need more than a tab: settings-emulation-…, settings-hidden…,
    /// romm-…, settings-missing.
    fn debug_open_settings(&mut self, st: &mut AppState, what: &str, id: GameId) -> bool {
        if let Some(rest) = what.strip_prefix("settings-emulation-") {
            let mut d = settings::SettingsDlg::new(settings::Tab::Emulation);
            d.emu = emulation_ui::EmuPage::debug(rest);
            self.dlg.settings = Some(d);
        } else if let Some(q) = what.strip_prefix("settings-hidden") {
            // settings-hidden[:<search>]: Settings → Libraries → Hidden & excluded.
            let mut d = settings::SettingsDlg::new(settings::Tab::Library);
            let mut page = hidden_ui::HiddenPage::default();
            page.search = q.trim_start_matches(':').to_string();
            d.hidden_page = Some(page);
            self.dlg.settings = Some(d);
        } else if let Some(rest) = what.strip_prefix("romm") {
            // romm-settings (Settings → Libraries with RomM server open), romm-ask (the
            // "where should downloads go?" question), romm-progress (a made-up download of
            // the selected game at 40 %; nothing is downloaded).
            romm_ui::debug_open(rest, st, id, &mut self.dlg.settings);
        } else if let Some(rest) = what.strip_prefix("settings-achievements") {
            // settings-achievements[-checking|-signedin|-error]: Settings → Achievements with a
            // made-up account check (nothing is asked online).
            let mut d = settings::SettingsDlg::new(settings::Tab::Achievements);
            let rest = rest.trim_start_matches('-');
            if !rest.is_empty() {
                // Made-up account in the test home's config (screenshot runs use a copy).
                let c = &mut st.config.achievements;
                c.ra_username = "Player1".into();
                c.ra_api_key = Some("screenshot-run-made-up-key".into());
            }
            d.ach = settings::AchPage::debug(rest);
            self.dlg.settings = Some(d);
        } else if what == "settings-missing" {
            // Settings → Libraries → Missing games.
            let mut d = settings::SettingsDlg::new(settings::Tab::Library);
            d.missing_page = Some(Default::default());
            self.dlg.settings = Some(d);
        } else {
            return false;
        }
        true
    }

    /// Artwork: pencil, viewer…, art…, bulkart[-run].
    fn debug_open_artwork(&mut self, ctx: &egui::Context, st: &mut AppState, what: &str, id: GameId) -> bool {
        let slot_in = |what: &str| {
            crate::art::ArtSlot::ALL
                .into_iter()
                .find(|s| what.split('-').any(|w| w == s.key()))
                .unwrap_or(crate::art::ArtSlot::Cover)
        };
        if what == "pencil" {
            self.debug_pencil = true;
        } else if what == "viewer" || what.starts_with("viewer-") {
            // viewer[-<slot key>][-zoom]: the artwork viewer (at 100 % with -zoom).
            if let Some(g) = st.library.get(id) {
                self.dlg.viewer = viewer::ViewerDlg::open(ctx, g, slot_in(what));
                if let Some(v) = &mut self.dlg.viewer {
                    v.debug_zoom = what.split('-').any(|w| w == "zoom");
                }
            }
        } else if what == "art" || what.starts_with("art-") {
            // art[-<slot key>][-online][:<search term>]: the artwork editor (searching
            // right away with -online).
            let (what, query) = what.split_once(':').unwrap_or((what, ""));
            if let Some(g) = st.library.get(id) {
                let mut d = artwork::ArtDlg::new(g, slot_in(what));
                d.online = what.split('-').any(|w| w == "online");
                d.debug_pick = what.split('-').any(|w| w == "pick");
                d.debug_close = what.split('-').any(|w| w == "close");
                d.debug_reset = what.split('-').any(|w| w == "reset");
                d.debug_input = std::env::var("LODESTAR_DESKTOP_ART_INPUT").ok();
                if !query.is_empty() {
                    d.query = query.to_string();
                }
                self.dlg.art = Some(d);
            }
        } else if what == "bulkart" {
            self.actions.push(Action::BulkArt(artwork::BulkScope::View));
        } else if what == "bulkart-run" {
            // Start a bulk download of the view's missing art (status bar progress).
            let ids = self.view.iter().map(|&i| st.library.games[i].id).collect();
            st.bulk_art(ids, crate::art::BulkOptions::default());
        } else {
            return false;
        }
        true
    }
}
