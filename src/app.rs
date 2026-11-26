//! Top-level application state (`AppState`): the library, background scans, mode switching and
//! notifications. The window that draws it frame by frame is `App` (app/window.rs).
//!
//! CONTRACT for UI code (`ui::desktop`, `ui::controller`): read `st.library`, `st.config`,
//! `st.images`, themes; act only through the `AppState` methods below (they keep the DB, the
//! in-memory library and the config file in sync). Never block the UI thread.

use crate::config::{Config, Paths, StartMode};
use crate::db::Db;
use crate::images::ImageCache;
use crate::input::{self, InputHub, Nav};
use crate::launch::{LaunchEvent, Launcher, ResolvedKind};
use crate::library::Library;
use crate::metadata::MetadataFetcher;
use crate::model::*;
use crate::scan::{self, ScanContext, ScanReport};
use crate::theme::fonts::FontRegistry;
use crate::theme::{Theme, ThemeManager};
use crate::ui::{controller::ControllerUi, desktop::DesktopUi};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::launch::play::records_own_playtime;

// The window (eframe's `App`) in its own file; AppState stays here.
mod window;
pub use window::App;
pub(crate) use window::is_wayland;
#[cfg(test)]
use window::{desktop_key_frames, desktop_pad_log, focus_left_for_game, just_ended, toast_width};

/// See `AppState::rescan_at`.
const STEAM_FOLLOWUP_RESCAN: Duration = Duration::from_secs(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Desktop,
    Controller,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: Instant,
    /// A button on the toast (desktop mode).
    pub action: Option<ToastAction>,
}

/// What a toast's button does (`AppState::toast_action`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastAction {
    /// Lodestar's own update: "What's new" / Restart now (update_app.rs).
    Update,
}

#[derive(Clone, Debug, Default)]
pub struct ScanStatus {
    pub running: bool,
    pub last: Option<ScanSummary>,
    /// The built-in libraries are done (their counts are in `last`); library add-ons still run.
    pub addons_pending: bool,
    /// Every game id (shown or excluded) when the running scan started: what is "new" after it
    /// (a partial reload before ScanDone must not hide the built-in libraries' new games).
    pub(crate) known_before: Option<std::collections::HashSet<GameId>>,
    /// The art folder was tidied this session (`art::tidy`, before the first scan writes).
    pub(crate) art_tidied: bool,
}

impl ScanStatus {
    /// Is `source` still being scanned?
    pub fn scanning(&self, source: Source) -> bool {
        self.running && (!self.addons_pending || source == Source::Addon)
    }
}

#[derive(Clone, Debug)]
pub struct ScanSummary {
    pub total: usize,
    pub added: usize,
    pub per_source: Vec<(Source, usize, Option<String>)>,
    pub elapsed: Duration,
}

#[allow(clippy::enum_variant_names)] // all scan messages (the theme-import one moved to Add-ons)
enum BgMsg {
    /// The merge is committed; the UI reloads from its own connection (a snapshot loaded on the
    /// scan thread would undo UI edits made while it was in flight).
    ScanDone {
        report: ScanReport,
        added: usize,
        /// A rescan of some libraries only (the others weren't scanned).
        only: Option<Vec<Source>>,
    },
    /// The built-in scanners' games are merged; library add-ons (which may take up to their
    /// timeout) are still running. The UI reloads so new games show without waiting for them.
    ScanPartial {
        per_source: Vec<(Source, usize, Option<String>)>,
        total: usize,
        elapsed: Duration,
    },
    ScanFailed(String),
}

/// Options from the command line.
#[derive(Clone, Debug, Default)]
pub struct LaunchOptions {
    pub force_mode: Option<Mode>,
    /// Save a PNG of the window after the UI settles, then exit.
    pub screenshot: Option<PathBuf>,
    /// Frames to wait (after images settle) before taking the screenshot.
    pub screenshot_frames: u32,
    /// Keys to inject (controller-mode navigation) before the screenshot, e.g. "down,down,accept".
    pub inject: Vec<Nav>,
    pub no_scan: bool,
    /// Command-line override: never go fullscreen this session (not saved to config).
    pub windowed: bool,
    /// Command-line override: never minimise on launch this session (not saved to config).
    pub no_minimize: bool,
}

pub struct AppState {
    pub paths: Paths,
    pub config: Config,
    pub db: Db,
    pub library: Library,
    pub images: ImageCache,
    pub launcher: Launcher,
    pub input: InputHub,
    pub metadata: MetadataFetcher,
    /// Achievements on game pages (worker + answers so far).
    pub achievements: crate::achievements::Hub,
    /// Screenshot runs: the library generation the made-up counts were made for.
    achievements_fake_for: Option<u64>,
    /// The library's counts were handed to the theme bindings at least once.
    achievements_published: bool,
    /// Artwork search / download service (see art.rs) and the UI-facing state it feeds.
    pub art: crate::art::ArtService,
    pub art_state: crate::art_app::ArtState,
    /// Automatic artwork for games without any after a scan (art_app.rs).
    pub auto_art: crate::art_app::AutoArt,
    /// IGDB-only background lookups of games looked up before IGDB was set up.
    pub igdb_backfill: crate::igdb::Backfill,
    /// Wikipedia descriptions for games looked up before (metadata/wikipedia.rs).
    pub wiki_backfill: crate::metadata::wikipedia::Backfill,
    /// Lookups the user asked for (Download metadata…) and their answers (metadata/lookup_app.rs).
    pub lookups: crate::metadata::Lookups,
    /// "Scan for games" import checklist (addgame.rs).
    pub import_state: crate::addgame::ImportState,
    /// Hand-added games that a scan has since found too (offer to merge).
    pub merge_suggestions: Vec<crate::addgame::MergeSuggestion>,
    pub themes: ThemeManager,
    pub fonts: FontRegistry,
    pub desktop_theme: Arc<Theme>,
    pub controller_theme: Arc<Theme>,
    pub mode: Mode,
    pub scan: ScanStatus,
    pub toasts: Vec<Toast>,
    ctx: egui::Context,
    bg_tx: Sender<BgMsg>,
    bg_rx: Receiver<BgMsg>,
    pending_mode: Option<Mode>,
    quit: bool,
    /// Window minimised because a game is running.
    minimized_for_game: bool,
    /// When the last game ended: for a moment after, only the Home button reaches Lodestar
    /// (presses meant for the game's exit dialogs must not start it again).
    pub game_ended_at: Option<Instant>,
    /// Bumped when the desktop theme changes so the desktop UI re-applies its style.
    pub style_generation: u64,
    /// Finished scans this session (the first one does not toast "new games").
    scan_count: u32,
    /// A rescan was asked for while one was running (e.g. a Steam game exited mid-scan): run
    /// another one when it finishes, so that request's changes (playtime …) aren't lost.
    rescan_queued: bool,
    /// Session-only overrides from the command line.
    pub force_windowed: bool,
    pub no_minimize: bool,
    /// Scripted screenshot run (`--screenshot`): never start a real game (a stray injected
    /// Accept / Enter on a game would otherwise launch it through Steam).
    pub no_launch: bool,
    /// The window can be un-minimised and focused programmatically. False on Wayland, where
    /// winit ignores both (`set_minimized(false)` and `focus_window()` are no-ops): minimising for
    /// a game there would leave Lodestar minimised after the game exits.
    pub window_restorable: bool,
    /// Per launch this session: last played before it (stored, shown) so a launch that fails at
    /// once can be taken back (`LaunchEvent::Failed`).
    launch_undo: std::collections::HashMap<GameId, (Option<i64>, Option<i64>)>,
    /// A follow-up rescan after a Steam game exited, for a Steam client that writes the session's
    /// playtime to `localconfig.vdf` only some seconds later.
    rescan_at: Option<Instant>,
    /// Games whose launch failed and whose `Exited` (sent right after `Failed`, possibly in a
    /// later poll) is still to come.
    failed_launches: std::collections::HashSet<GameId>,
    /// Games running an action that isn't Play (no playtime for those sessions).
    non_play_sessions: std::collections::HashSet<GameId>,
    /// Background work of Edit Game Details (install size …), see edit_app.rs.
    pub(crate) edit_bg: crate::edit_app::EditBg,
    /// External add-ons in `<data_dir>/addons` (addons/mod.rs). `refresh_addons()` re-reads them.
    pub addons: crate::addons::AddonManager,
    /// "<A> is running. Close it and start <B>?" waiting for an answer: (running, new). The UIs
    /// show it as a dialog and answer with `confirm_switch()` / `cancel_switch()` (launch/switch.rs).
    pub switch_prompt: Option<(GameId, GameId)>,
    /// One game at a time: a launch waiting for the running game to close (launch/switch.rs).
    pub(crate) switch: crate::launch::switch::SwitchState,
    /// Game options: detected capabilities and the probe (game_options_app.rs).
    pub game_opts: crate::game_options_app::GameOptsState,
    /// Games running while Lodestar comes and goes: adopting them, closing while one plays
    /// (running_app.rs).
    pub(crate) running: crate::running_app::RunningState,
    /// Steam name / owned-games lookups after scans (steam_names_app.rs).
    pub steam_names: crate::steam_names_app::SteamNamesState,
    /// Games kept out of the library as not games (exclude_app.rs).
    pub excl: crate::exclude_app::ExclState,
    /// The system tray, hiding the window, later starts of the GUI (tray_app.rs).
    pub(crate) shell: crate::tray_app::ShellState,
    /// RomM server: sync, test, downloads (romm_app.rs).
    pub romm: crate::romm_app::RommState,
    /// Lodestar's own updates (update_app.rs).
    pub update: crate::update_app::UpdateState,
    /// The first start's welcome step (welcome_app.rs).
    pub welcome: crate::welcome_app::WelcomeState,
}

/// After a game ends, only the Home button reaches Lodestar for this long.
const GAME_END_SETTLE: std::time::Duration = std::time::Duration::from_millis(1200);

impl AppState {
    // ----- game actions -----

    /// Play: the game's Play action when it has one, else its own launch.
    pub fn launch(&mut self, id: GameId) {
        self.switch.windowed = None;
        self.switch.play_with = None;
        self.switch.play_choice = None;
        self.start_game(id, None);
    }

    /// Play ▾ → Play in a window (emulated games): Play, with the emulator's windowed command.
    pub fn launch_windowed(&mut self, id: GameId) {
        let Some(g) = self.library.get(id) else { return };
        let Some(spec) = g.emu.as_ref().and_then(|e| crate::emulation::registry::spec_windowed(g, e)) else {
            let why = crate::emulation::registry::missing_reason(g).unwrap_or_else(|| "Not an emulated game".into());
            self.notify_error(why);
            return;
        };
        // Remembered in case another game has to close first (launch/switch.rs starts it later).
        self.switch.windowed = Some(id);
        self.switch.play_with = None;
        self.switch.play_choice = None;
        self.start_game_as(id, None, Some(spec));
    }

    /// Start a game (`action` None = Play) or one of its actions (`edits.actions[i]`). Play
    /// launches (also an action marked as the Play action) count as plays: play count, last
    /// played, playtime, scripts, completion → Playing (`config.auto_playing`). Other actions
    /// (a settings tool, a benchmark …) only run, tracked per their tracking mode.
    pub(crate) fn start_game(&mut self, id: GameId, action: Option<usize>) {
        // A Play in a window that waited for another game to close stays in a window.
        let windowed = (action.is_none() && self.switch.windowed == Some(id))
            .then(|| self.library.get(id))
            .flatten()
            .and_then(|g| g.emu.as_ref().and_then(|e| crate::emulation::registry::spec_windowed(g, e)));
        self.start_game_as(id, action, windowed);
    }

    /// `start_game` with the game's command replaced by `spec` (Play in a window).
    fn start_game_as(&mut self, id: GameId, action: Option<usize>, spec: Option<LaunchSpec>) {
        let Some(mut game) = self.library.get(id).cloned() else { return };
        // Play ▾ → Play with ▸ <emulator>: this Play uses that emulator and profile.
        let play_with = self.switch.play_with.clone().filter(|(g, _, _)| *g == id && action.is_none());
        if let Some((_, emu, profile)) = &play_with {
            crate::emulation::options::with_choice(&mut game, emu, profile);
        }
        // "Choose what to play": the line picked, else the one remembered ("Always use this").
        if action.is_none() {
            crate::emu_app::apply_play_choice(
                &mut game,
                self.switch.play_choice.as_ref().filter(|(g, _)| *g == id).map(|(_, c)| c),
            );
        }
        if let Some(spec) = spec {
            // The given command, not the one the emulation registry would build.
            game.launch = Some(spec);
            game.emu = None;
        }
        let act = match action {
            Some(i) => match game.edits.actions.get(i) {
                Some(a) => Some(a),
                None => return,
            },
            None => game.play_action(),
        };
        let play = act.is_none_or(|a| a.play);
        let what = match act.filter(|_| !play) {
            Some(a) if !a.name.trim().is_empty() => format!("{} ({})", game.display_name(), a.name.trim()),
            _ => game.display_name().to_string(),
        };
        if self.no_launch {
            log::warn!("screenshot run: not launching {what}");
            self.notify(format!("Launch skipped (screenshot run): {what}"));
            return;
        }
        // Another game Lodestar launched still runs: ask / close it first / allow several.
        if !self.one_game_gate(id, action) {
            return;
        }
        self.switch.windowed = None;
        self.switch.play_with = None;
        self.switch.play_choice = None;
        if self.launcher.active_games().contains(&id) {
            // Same game: once it is up, a second Play (double click, A pressed twice) must not
            // start a second instance. While a Steam launch is still pending, asking Steam again
            // is harmless and can nudge a stuck client.
            if self.launcher.is_started(id) {
                self.notify(format!("{} is already running", game.display_name()));
                return;
            }
        }
        // Right after startup the capability probe may still run: wait for it (bounded), or
        // Gamescope / MangoHud / GameMode would be dropped as "not installed".
        self.ensure_game_options_probed();
        let resolved = match action {
            Some(i) => crate::launch::resolve_action(&game, i),
            None => crate::launch::resolve(&game),
        };
        let mut plan = crate::launch::LaunchPlan::for_game(&game, act, &self.config.scripts, play);
        // Cloud games opening in a browser tab / an app or browser already open: not followed.
        let cloud_note = if play { crate::cloud::launch_note(&game, act, &mut plan) } else { None };
        match resolved.and_then(|r| self.launcher.launch_planned(id, game.display_name(), &r, &plan)) {
            Ok(()) => {
                if !play {
                    self.non_play_sessions.insert(id);
                    self.launch_undo.remove(&id);
                    self.notify(format!("Starting {what}"));
                    return;
                }
                self.non_play_sessions.remove(&id);
                let now = now_unix();
                match self.db.record_launch(id, now) {
                    Ok(prev) => {
                        self.launch_undo.insert(id, (prev, game.last_played));
                        self.failed_launches.remove(&id);
                    }
                    Err(e) => log::warn!("record_launch: {e:#}"),
                }
                self.library.update(id, |g| {
                    g.play_count += 1;
                    g.last_played = Some(now);
                });
                // From the library's game: `game` may carry Play with's one-time choice.
                let auto = self.library.get(id).and_then(|g| crate::launch::play::auto_playing_edits(g, &self.config));
                if let Some(e) = auto {
                    self.set_edits(id, e);
                }
                self.notify(cloud_note.unwrap_or_else(|| format!("Starting {what}")));
            }
            Err(e) => self.notify_error(format!("Couldn't start {what}: {e:#}")),
        }
    }

    pub fn running_game(&self) -> Option<GameId> {
        self.launcher.running()
    }

    pub fn toggle_favorite(&mut self, id: GameId) {
        let Some(v) = self.library.get(id).map(|g| !g.favorite) else { return };
        if self.db.set_favorite(id, v).is_ok() {
            self.library.update(id, |g| g.favorite = v);
        }
    }

    pub fn set_hidden(&mut self, id: GameId, hidden: bool) {
        if self.db.set_hidden(id, hidden).is_ok() {
            self.library.update(id, |g| g.hidden = hidden);
        }
    }

    pub fn rename(&mut self, id: GameId, name: Option<String>) {
        let name = name.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        if self.db.set_user_name(id, name.as_deref()).is_ok() {
            self.library.update(id, |g| {
                g.sort_name = sort_key(name.as_deref().unwrap_or(&g.name));
                g.user_name = name;
            });
        }
    }

    pub fn set_tags(&mut self, id: GameId, tags: Vec<String>) {
        // Trimmed, no empties, no duplicates that differ only in case ("RPG", "rpg").
        let tags = crate::library::normalize_tags(tags);
        if self.db.set_tags(id, &tags).is_ok() {
            self.library.update(id, |g| g.tags = tags);
        }
    }

    /// Set or clear (None = follow Steam / the source) the user's controller-support setting.
    pub fn set_controller_override(&mut self, id: GameId, v: Option<ControllerSupport>) {
        match self.db.set_controller_override(id, v) {
            Ok(()) => self.library.update(id, |g| g.controller_override = v),
            Err(e) => self.notify_error(format!("Couldn't save controller support: {e:#}")),
        }
    }

    /// Set or clear the user's launch override for a game.
    pub fn set_launch_override(&mut self, id: GameId, o: Option<LaunchOverride>) {
        let o = o.filter(|o| !o.is_empty());
        match self.db.set_launch_override(id, o.as_ref()) {
            Ok(()) => self.library.update(id, |g| g.launch_override = o),
            Err(e) => self.notify_error(format!("Couldn't save launch options: {e:#}")),
        }
    }

    #[cfg(test)]
    pub fn add_manual_game(&mut self, name: &str, launch: LaunchSpec, art: Art) {
        match self.db.add_manual(name, launch, art) {
            Ok(_) => self.reload_library(),
            Err(e) => self.notify_error(format!("Couldn't add game: {e:#}")),
        }
    }

    /// Remove a game from the library. A game added by hand is deleted. A scanned game would
    /// come straight back with the next scan, so it is hidden instead: scans never unhide a game,
    /// and its favourite/tags/playtime are kept. It reappears under "Show hidden", where
    /// `restore_game` (or `set_hidden(id, false)`) brings it back.
    pub fn remove_game(&mut self, id: GameId) {
        let Some(g) = self.library.get(id) else { return };
        let name = g.display_name().to_string();
        if g.source == Source::Manual {
            crate::library::user::replace_game(&self.db, id, None);
            match self.db.delete(id) {
                Ok(()) => self.reload_library(),
                Err(e) => self.notify_error(format!("Couldn't remove {name}: {e:#}")),
            }
            return;
        }
        match self.db.set_hidden(id, true) {
            Ok(()) => {
                self.library.update(id, |g| g.hidden = true);
                self.notify(format!("Removed {name} (it stays out on rescans; Show hidden games to restore it)"));
            }
            Err(e) => self.notify_error(format!("Couldn't remove {name}: {e:#}")),
        }
    }

    /// Undo `remove_game` for a scanned game (it was hidden).
    pub fn restore_game(&mut self, id: GameId) {
        self.set_hidden(id, false);
    }

    // ----- library -----

    pub fn rescan(&mut self) {
        self.rescan_only(None);
    }

    /// Rescan the libraries on this computer, without RomM and add-ons (they go online or run
    /// other programs): the Steam name lookup found names. All of them, not just Steam: a
    /// shortcut (Steam's or a menu entry) into Faugus, Heroic or Lutris is only recognised as
    /// that game while that library is scanned too; without it the shortcut became a second copy.
    pub(crate) fn rescan_local_libraries(&mut self) {
        let local = Source::ALL.iter().copied().filter(|s| !matches!(s, Source::Romm | Source::Addon)).collect();
        self.rescan_only(Some(local));
    }

    fn rescan_only(&mut self, only: Option<Vec<Source>>) {
        if only.is_none() {
            self.romm_on_rescan();
        }
        if self.scan.running {
            self.rescan_queued = true;
            return;
        }
        self.scan.running = true;
        self.scan.known_before = Some(self.library.games.iter().chain(&self.excl.games).map(|g| g.id).collect());
        let tx = self.bg_tx.clone();
        let ctx = self.ctx.clone();
        let db_path = self.paths.db_file();
        let tidy_art = (!std::mem::replace(&mut self.scan.art_tidied, true)).then(|| self.paths.art_dir());
        let mut scan_ctx = ScanContext::from_config(&self.config);
        if let Some(keep) = &only {
            for s in Source::ALL.iter().filter(|s| !keep.contains(s)) {
                if !scan_ctx.config.disabled_sources.iter().any(|k| k == s.key()) {
                    scan_ctx.config.disabled_sources.push(s.key().to_string());
                }
            }
        }
        std::thread::Builder::new()
            .name("scan".into())
            .spawn(move || {
                let msg = (|| -> anyhow::Result<BgMsg> {
                    let mut db = Db::open(&db_path)?;
                    if let Some(art_dir) = &tidy_art {
                        match crate::art::tidy::run(&db, art_dir) {
                            Ok(t) if t != Default::default() => log::info!(
                                "art: removed {} folders of games that are gone, fetching art again for {} games",
                                t.folders_removed,
                                t.games_refetched
                            ),
                            Ok(_) => {}
                            Err(e) => log::warn!("art clean-up: {e:#}"),
                        }
                    }
                    // Library add-ons run last, in a pass of their own: a slow or hanging one
                    // (up to its timeout) must not hold back the built-in libraries' games.
                    let addon_key = Source::Addon.key();
                    let addons_on = !scan_ctx.addons.enabled.is_empty()
                        && !scan_ctx.config.disabled_sources.iter().any(|k| k == addon_key);
                    let mut first_ctx = scan_ctx.clone();
                    if addons_on {
                        first_ctx.config.disabled_sources.push(addon_key.to_string());
                    }
                    let mut report = scan::run_all(&first_ctx);
                    let mut stats =
                        db.merge_scan_with_shortcuts(&report.games, &report.ok_sources, &report.dropped_shortcuts)?;
                    if addons_on {
                        let _ = tx.send(BgMsg::ScanPartial {
                            per_source: report.per_source.iter().map(|(s, r)| (*s, r.found, r.error.clone())).collect(),
                            total: report.games.len(),
                            elapsed: report.elapsed,
                        });
                        ctx.request_repaint();
                        let mut addon_ctx = scan_ctx.clone();
                        addon_ctx.config.disabled_sources =
                            Source::ALL.iter().filter(|s| **s != Source::Addon).map(|s| s.key().to_string()).collect();
                        let more = scan::run_all(&addon_ctx);
                        let st2 = db.merge_scan(&more.games, &more.ok_sources)?;
                        stats.added += st2.added;
                        stats.updated += st2.updated;
                        stats.removed += st2.removed;
                        stats.uninstalled += st2.uninstalled;
                        stats.missing += st2.missing;
                        report.games.extend(more.games);
                        report.ok_sources.extend(more.ok_sources);
                        report.per_source.extend(more.per_source);
                        report.duplicates_dropped += more.duplicates_dropped;
                        report.elapsed += more.elapsed;
                    }
                    log::info!(
                        "scan: {} games from {} sources in {:?} ({stats:?}, {} duplicates dropped)",
                        report.games.len(),
                        report.ok_sources.len(),
                        report.elapsed,
                        report.duplicates_dropped
                    );
                    Ok(BgMsg::ScanDone { report, added: stats.added, only })
                })()
                .unwrap_or_else(|e| BgMsg::ScanFailed(format!("{e:#}")));
                let _ = tx.send(msg);
                ctx.request_repaint();
            })
            .ok();
    }

    pub(crate) fn reload_library(&mut self) {
        match self.db.load_all() {
            Ok(games) => self.set_games(games),
            Err(e) => self.notify_error(format!("Couldn't read library: {e:#}")),
        }
    }

    // ----- modes, themes, config -----

    pub fn switch_mode(&mut self, mode: Mode) {
        self.pending_mode = Some(mode);
        self.ctx.request_repaint();
    }

    /// Quit Lodestar (the menu's Quit: always quits, also while a game runs).
    pub fn quit(&mut self) {
        self.quit = true;
        self.ctx.request_repaint();
    }

    /// `quit()` was asked for.
    pub(crate) fn quitting(&self) -> bool {
        self.quit
    }

    pub fn save_config(&mut self) {
        self.apply_launcher_settings();
        self.apply_tray_setting();
        self.publish_game_option_defaults();
        self.input.set_config(&self.config.controller);
        self.metadata.set_config(&self.config.metadata);
        self.metadata.set_addons(&self.addons, &self.config.addons);
        self.art.set_config(&self.config.metadata);
        self.queue_igdb_backfill();
        self.queue_wikipedia_backfill();
        if !self.config.art.auto_fill {
            self.stop_auto_art();
        }
        if let Err(e) = self.config.save(&self.paths.config_file()) {
            self.notify_error(format!("Couldn't save settings: {e:#}"));
        }
    }

    /// The controller theme `key` with the colour scheme the user picked for it.
    pub fn load_controller_theme(&self, key: &str) -> (Arc<Theme>, Option<String>) {
        self.themes.load_with_scheme(key, self.config.controller.color_scheme.as_deref())
    }

    pub fn set_desktop_theme(&mut self, key: &str) {
        let (theme, problem) = self.themes.load_with_scheme(key, self.config.desktop_color_scheme.as_deref());
        self.desktop_theme = theme;
        if let Some(p) = problem {
            self.notify_error(p);
        }
        self.config.desktop_theme = self.desktop_theme.key.clone();
        self.style_generation += 1;
        self.save_config();
    }

    pub fn set_controller_theme(&mut self, key: &str) {
        let (theme, problem) = self.load_controller_theme(key);
        self.controller_theme = theme;
        if let Some(p) = problem {
            self.notify_error(p);
        }
        self.config.controller.theme = self.controller_theme.key.clone();
        self.save_config();
    }

    pub fn notify(&mut self, text: impl Into<String>) {
        self.push_toast(text.into(), false);
    }

    pub fn notify_error(&mut self, text: impl Into<String>) {
        let t = text.into();
        log::warn!("{t}");
        self.push_toast(t, true);
    }

    /// A notice that stays up longer (10 s), for one the user may only see a little later.
    pub(crate) fn notify_long(&mut self, text: impl Into<String>) {
        self.push_toast_for(text.into(), false, Duration::from_secs(10));
    }

    fn push_toast(&mut self, text: String, error: bool) {
        self.push_toast_for(text, error, Duration::from_secs(if error { 6 } else { 3 }));
    }

    fn push_toast_for(&mut self, text: String, error: bool, shown: Duration) {
        self.toasts.push(Toast { text, error, until: Instant::now() + shown, action: None });
        self.trim_toasts();
    }

    /// A toast with a button (`action`), shown for `shown`.
    pub(crate) fn push_toast_with(&mut self, text: String, action: Option<ToastAction>, shown: Duration) {
        self.toasts.push(Toast { text, error: false, until: Instant::now() + shown, action });
        self.trim_toasts();
    }

    fn trim_toasts(&mut self) {
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
        self.ctx.request_repaint();
    }

    pub fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    /// Launcher hygiene from the settings; never in screenshot runs (nothing is minimised or
    /// closed there).
    fn apply_launcher_settings(&mut self) {
        let cfg = if self.no_launch {
            crate::launch::hygiene::HygieneConfig::default()
        } else {
            crate::launch::hygiene::HygieneConfig::from_settings(&self.config.launchers)
        };
        self.launcher.set_hygiene(cfg);
    }

    /// Controller mode should be fullscreen (config, unless `--windowed`/screenshot this session).
    pub fn wants_fullscreen(&self) -> bool {
        self.config.controller.fullscreen && !self.force_windowed
    }

    /// Button prompts to draw: the manual choice, else the style of the pad used last
    /// (`LODESTAR_FAKE_PAD` stands in for it; screenshot runs never look at real pads).
    pub fn prompts(&self) -> input::Prompts {
        let c = &self.config.controller;
        input::Prompts::new(c.button_prompts.resolve(|| self.detected_button_style()), c.swap_ab)
    }

    /// Style of the pad in use, as Automatic would pick it (None: no pad, or a screenshot run).
    pub fn detected_button_style(&self) -> Option<input::ButtonStyle> {
        static FAKE: std::sync::OnceLock<Option<input::ButtonStyle>> = std::sync::OnceLock::new();
        let fake = *FAKE
            .get_or_init(|| std::env::var("LODESTAR_FAKE_PAD").ok().and_then(|v| input::style::fake_pad_style(&v)));
        fake.or_else(|| (!self.no_launch).then(|| self.input.pad_style()).flatten())
    }

    // ----- per-frame plumbing -----

    fn poll_background(&mut self) {
        self.achievements.poll();
        self.sync_achievement_counts();
        self.poll_art();
        self.poll_addgame();
        self.poll_store();
        self.poll_romm();
        self.poll_emulator_dirs();
        self.poll_steam_names();
        self.poll_updates();
        while let Ok(msg) = self.bg_rx.try_recv() {
            match msg {
                BgMsg::ScanDone { report, added, only } => self.scan_done(&report, added, only),
                BgMsg::ScanPartial { per_source, total, elapsed } => {
                    self.reload_library();
                    self.scan.addons_pending = true;
                    self.scan.last = Some(ScanSummary { total, added: 0, per_source, elapsed });
                }
                BgMsg::ScanFailed(e) => {
                    self.scan.running = false;
                    self.scan.known_before = None;
                    self.scan.addons_pending = false;
                    self.notify_error(format!("Scan failed: {e}"));
                    self.run_queued_rescan();
                }
            }
        }
        self.poll_running_scan();
        for ev in self.launcher.poll() {
            match ev {
                LaunchEvent::Started { id } => self.game_started(id),
                LaunchEvent::Exited { id, secs } => self.game_exited(id, secs),
                LaunchEvent::Failed { id, error } => self.launch_failed(id, &error),
            }
        }
        self.advance_switch();
        for n in self.launcher.script_notices() {
            if n.should_report() {
                let text = n.text();
                if n.outcome.ok() {
                    self.notify(text);
                } else {
                    self.notify_error(text);
                }
            }
        }
        self.poll_edit();
        self.poll_metadata();
        // After the metadata results above: it waits for the fetcher to be idle.
        self.run_auto_art();
    }

    /// The library's achievement counts follow the library and the settings; controller themes
    /// get them through the `{game.achievements…}` bindings.
    fn sync_achievement_counts(&mut self) {
        if let Some(kind) = crate::ui::achievements::fake_kind()
            && self.achievements_fake_for != Some(self.library.generation)
        {
            self.achievements_fake_for = Some(self.library.generation);
            let fake = crate::ui::achievements::fake_counts(&self.library.games, &kind);
            self.achievements.set_fake_counts(fake);
        }
        let before = self.achievements.counts_generation();
        self.achievements.sync(&self.library.games, self.library.generation, &self.config.achievements);
        if self.achievements.counts_generation() != before || !self.achievements_published {
            self.achievements_published = true;
            crate::theme::render::bind::set_library_achievements(self.achievements.counts().clone());
            self.library.set_achievement_counts(self.achievements.counts().clone());
        }
    }

    /// A library scan finished: show the new library and start what follows a scan.
    fn scan_done(&mut self, report: &ScanReport, added: usize, only: Option<Vec<Source>>) {
        self.scan.running = false;
        self.scan.addons_pending = false;
        let known = self
            .scan
            .known_before
            .take()
            .unwrap_or_else(|| self.library.games.iter().chain(&self.excl.games).map(|g| g.id).collect());
        // Every UI write goes to the DB at once, so the DB (after the scan's commit)
        // has both the scan's and the user's changes. ~1 ms for a few hundred games.
        self.reload_library();
        self.refresh_steam_owned();
        // New rows that a "Not a game" rule keeps out don't count as new games.
        let added = if added == 0 { 0 } else { self.library.games.iter().filter(|g| !known.contains(&g.id)).count() };
        let mut summary = ScanSummary {
            total: report.games.len(),
            added,
            per_source: report.per_source.iter().map(|(s, r)| (*s, r.found, r.error.clone())).collect(),
            elapsed: report.elapsed,
        };
        // One library rescanned: the other libraries' rows of the last full scan stay.
        if only.is_some()
            && let Some(old) = &self.scan.last
        {
            let mut rows = old.per_source.clone();
            let mut total = old.total;
            for row in &summary.per_source {
                match rows.iter_mut().find(|r| r.0 == row.0) {
                    Some(r) => {
                        total = (total + row.1).saturating_sub(r.1);
                        *r = row.clone();
                    }
                    None => {
                        total += row.1;
                        rows.push(row.clone());
                    }
                }
            }
            summary.total = total;
            summary.per_source = rows;
            summary.elapsed = old.elapsed;
            summary.added += old.added;
        }
        self.scan.last = Some(summary);
        if added > 0 && self.scan_count > 0 {
            self.notify(format!("{added} new game{} found", if added == 1 { "" } else { "s" }));
        }
        self.scan_count += 1;
        // Emulators installed since the last scan count for Play right away. Started
        // here (not in the scan thread) so it uses the settings as they are now: a
        // scan started before a Settings → Emulation change must not publish the old ones.
        self.refresh_emulation(true);
        // Art files that appeared since startup (e.g. Steam's library cache) get another chance.
        self.images.forget_failed();
        self.queue_metadata();
        self.auto_art_after_scan();
        self.steam_names_after_scan(&report.steam_needs);
        self.run_queued_rescan();
    }

    fn game_started(&mut self, id: GameId) {
        log::info!("game {id} started");
        if self.config.minimize_on_launch && !self.no_minimize && self.window_restorable && self.mode == Mode::Desktop {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            self.minimized_for_game = true;
        }
    }

    /// A game ended: count its playtime (when Lodestar is the one counting) and bring the window back.
    fn game_exited(&mut self, id: GameId, secs: u64) {
        log::info!("game {id} exited after {secs}s");
        // New unlocks show when the player comes back to the game's page.
        if let Some(g) = self.library.get(id) {
            let g = g.clone();
            self.achievements.game_exited(&g, &self.config.achievements);
        }
        self.launch_undo.remove(&id);
        // Who counts the playtime depends on how the game was started, not on its
        // source: the Steam client counts what it ran (picked up by a rescan); a
        // Steam game started with a custom command never reaches Steam's counter.
        let kind = self.launcher.launch_kind(id);
        let via_steam_client = matches!(kind, Some(ResolvedKind::Steam { .. }));
        // A launch that failed (`Failed` comes just before its `Exited`) isn't play.
        let failed = self.failed_launches.remove(&id);
        let play = !self.non_play_sessions.remove(&id);
        // Picked up from another Lodestar that still runs: that one records it.
        let foreign = self.launcher.is_foreign(id);
        if play
            && !foreign
            && records_own_playtime(kind)
            && !failed
            && secs > 0
            && self.db.add_playtime(id, secs).is_ok()
        {
            self.library.update(id, |g| g.playtime_secs += secs);
        }
        // Hidden to the tray meanwhile: it stays there.
        if std::mem::take(&mut self.minimized_for_game) && !self.shell.hidden {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        }
        // Closed (minimised) by the user while the game ran: it stays down. A game
        // only picked up (started from Steam, or before this run) doesn't pull
        // Lodestar to the front either.
        self.game_ended_at = Some(Instant::now());
        // Another game still running ("Allow several"): Lodestar stays behind it, or
        // presses meant for that game would land on the Playing overlay.
        if !self.running.minimised_for_close
            && self.launcher.adopted_start(id).is_none()
            && self.launcher.started().is_none()
        {
            self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        if via_steam_client {
            // Steam writes playtime to localconfig.vdf; pick it up now, and once
            // more a little later in case Steam hadn't flushed it yet.
            self.rescan();
            self.rescan_at = Some(Instant::now() + STEAM_FOLLOWUP_RESCAN);
            self.ctx.request_repaint_after(STEAM_FOLLOWUP_RESCAN);
        }
    }

    fn launch_failed(&mut self, id: GameId, error: &str) {
        self.failed_launches.insert(id);
        // Failed at once (bad command, crash on start): not a play.
        if let Some((prev_db, prev_mem)) = self.launch_undo.remove(&id)
            && self.db.undo_launch(id, prev_db).is_ok()
        {
            self.library.update(id, |g| {
                g.play_count = g.play_count.saturating_sub(1);
                g.last_played = prev_mem;
            });
        }
        let name = self.library.get(id).map(|g| g.display_name().to_string()).unwrap_or_default();
        self.notify_error(format!("{name}: {error}"));
    }

    /// Fetched metadata: stored, then the library row refreshed. A lookup the user asked for goes
    /// back to Edit Game Details instead (stored on Save).
    fn poll_metadata(&mut self) {
        for r in self.metadata.poll() {
            if r.lookup.is_some() {
                self.lookups.answered(r);
                continue;
            }
            if r.backfill {
                self.igdb_backfill_done(r.id);
            }
            if r.wiki_backfill {
                self.wikipedia_backfill_done(r.id);
            }
            if self.db.set_fetched(r.id, &r.art, &r.meta).is_ok()
                && let Ok(Some(g)) = self.db.get(r.id)
            {
                self.library.update(r.id, |old| *old = g);
            }
        }
    }

    fn run_due_rescan(&mut self) {
        let Some(at) = self.rescan_at else { return };
        let now = Instant::now();
        if now >= at {
            self.rescan_at = None;
            self.rescan();
        } else {
            self.ctx.request_repaint_after(at - now);
        }
    }

    fn run_queued_rescan(&mut self) {
        if std::mem::take(&mut self.rescan_queued) {
            self.rescan();
        }
    }

    /// After a scan: ask the fetcher for games never looked up that lack art or a description.
    pub(crate) fn queue_metadata(&mut self) {
        if self.lookups_wait_for_welcome() {
            return;
        }
        self.queue_protondb();
        let want_steam = self.config.metadata.steam_store;
        let want_sgdb = self.config.metadata.steamgriddb_key.as_deref().is_some_and(|k| !k.is_empty());
        let want_addons = self.metadata.has_addon_sources();
        let want_igdb = self.config.metadata.igdb_metadata && self.config.metadata.igdb_credentials().is_some();
        let want_wiki = self.config.metadata.wikipedia && crate::metadata::wikipedia::allowed();
        if !want_steam && !want_sgdb && !want_addons && !want_igdb && !want_wiki {
            return;
        }
        for g in &self.library.games {
            let needs = g.art.cover.is_none() || g.art.hero.is_none() || g.meta.description.is_none();
            let eligible =
                (g.source == Source::Steam && want_steam) || want_sgdb || want_addons || want_igdb || want_wiki;
            if needs && eligible && matches!(self.db.fetched_at(g.id), Ok(None)) {
                self.metadata.request(g);
            }
        }
        self.queue_igdb_backfill();
        self.queue_wikipedia_backfill();
    }

    /// IGDB was set up (or its credentials changed) after games were looked up: give every game
    /// that misses something IGDB could fill one IGDB-only background lookup (only gaps are
    /// filled; once per game and Twitch app; never-looked-up games go through `queue_metadata`).
    fn queue_igdb_backfill(&mut self) {
        let m = &self.config.metadata;
        if !self.igdb_backfill.enabled || !m.igdb_metadata {
            return;
        }
        let Some((id, secret)) = m.igdb_credentials() else {
            return;
        };
        let key = crate::igdb::cache_key(id, secret);
        // (Not before the library is loaded: the games are queued once per session.)
        if self.igdb_backfill.queued_for == Some(key)
            || crate::igdb::is_rejected(id, secret)
            || self.library.games.is_empty()
        {
            return;
        }
        self.igdb_backfill.queued_for = Some(key);
        let db = &self.db;
        let st = self.igdb_backfill.state.get_or_insert_with(|| {
            db.kv_get(crate::igdb::BACKFILL_KV).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
        });
        if st.key != key {
            *st = crate::igdb::BackfillDone { key, done: Default::default() };
        }
        for g in &self.library.games {
            if !g.hidden && !st.done.contains(&g.id) && matches!(self.db.fetched_at(g.id), Ok(Some(_))) {
                self.metadata.request_igdb_backfill(g);
            }
        }
    }

    fn igdb_backfill_done(&mut self, id: GameId) {
        let Some(st) = self.igdb_backfill.state.as_mut() else {
            return;
        };
        if st.done.insert(id) {
            match serde_json::to_string(&*st) {
                Ok(s) => {
                    if let Err(e) = self.db.kv_set(crate::igdb::BACKFILL_KV, &s) {
                        log::warn!("IGDB: saving lookups: {e:#}");
                    }
                }
                Err(e) => log::warn!("IGDB: saving lookups: {e}"),
            }
        }
    }

    fn expire_toasts(&mut self) {
        let now = Instant::now();
        self.toasts.retain(|t| t.until > now);
        if let Some(next) = self.toasts.iter().map(|t| t.until).min() {
            self.ctx.request_repaint_after(next.saturating_duration_since(now));
        }
    }
}

#[cfg(test)]
impl AppState {
    /// A full state on a temporary root (no window, no scan). Spawns the usual worker threads.
    pub(crate) fn for_test(root: &std::path::Path) -> AppState {
        let ctx = egui::Context::default();
        let paths = Paths {
            home: root.join("home"),
            config_dir: root.join("config"),
            data_dir: root.join("data"),
            cache_dir: root.join("cache"),
        };
        let mut config = Config::default();
        config.metadata.steam_store = false;
        config.metadata.steamgriddb_key = None;
        let db = Db::open(&paths.db_file()).unwrap();
        let games = db.load_all().unwrap();
        let themes = ThemeManager::new(paths.themes_dir());
        let (desktop_theme, _) = themes.load_checked(&config.desktop_theme);
        let (controller_theme, _) = themes.load_checked(&config.controller.theme);
        let (bg_tx, bg_rx) = unbounded();
        let mut addons = crate::addons::AddonManager::new(paths.data_dir.join("addons"));
        addons.refresh();
        let mut st = AppState {
            addons,
            images: ImageCache::new(&ctx, paths.thumbs_dir()),
            launcher: Launcher::with_log_dir(&ctx, paths.cache_dir.join("logs")),
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
            auto_art: Default::default(),
            igdb_backfill: Default::default(),
            wiki_backfill: Default::default(),
            lookups: Default::default(),
            import_state: Default::default(),
            merge_suggestions: vec![],
            library: Library::default(),
            excl: Default::default(),
            fonts: FontRegistry::new(),
            themes,
            desktop_theme,
            controller_theme,
            mode: Mode::Desktop,
            scan: ScanStatus::default(),
            toasts: vec![],
            ctx,
            bg_tx,
            bg_rx,
            pending_mode: None,
            quit: false,
            minimized_for_game: false,
            game_ended_at: None,
            style_generation: 1,
            scan_count: 0,
            rescan_queued: false,
            force_windowed: true,
            no_minimize: true,
            no_launch: false,
            window_restorable: false,
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
        st.set_games(games);
        st
    }

    pub(crate) fn pending_mode_for_test(&self) -> Option<Mode> {
        self.pending_mode
    }

    /// Poll until `done` or the timeout; returns whether `done` became true.
    pub(crate) fn poll_until(&mut self, timeout: Duration, mut done: impl FnMut(&AppState) -> bool) -> bool {
        let t = Instant::now();
        while t.elapsed() < timeout {
            self.poll_background();
            if done(self) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        false
    }
}

mod probe;

#[cfg(test)]
mod state_tests;

#[cfg(test)]
mod tests;
