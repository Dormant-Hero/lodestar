//! The AppState side of the RomM server library (romm.rs): Test connection, pairing, syncing
//! the server's list (then a rescan shows it), and downloads with progress / cancel.
//! Every network call runs on its own short-lived worker thread; results come back through a
//! channel that `poll_romm` (once per frame, one `try_recv`) delivers.
//!
//! CONTRACT (UIs):
//!   st.romm_set_up() -> bool             an address is set and the library is switched on
//!   st.romm_test() / st.romm_sync() / st.romm_pair(code)
//!   st.romm.testing / syncing / pairing, st.romm.test_result / sync_result (plain words)
//!   st.romm_can_download(&Game)          not installed RomM game of a console Lodestar emulates
//!   st.romm_download(id)                 desktop / controller Play button (via install_game)
//!   st.romm_progress(id) -> Option<Arc<Progress>>  (set its `cancel` to stop; the thread cleans up)
//!   st.romm.ask: Some((id, suggestion)) → the desktop asks where downloads go;
//!   st.romm_folder_chosen(id, Some(dir) | None)

use crate::app::{AppState, Mode};
use crate::config::RomFolder;
use crate::model::{Game, GameId, Source};
use crate::romm::download::{self, Progress, Target};
use crate::romm::{self, ApiError, Server};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A rescan syncs the server again only when the last sync is older than this.
const RESYNC_AFTER: Duration = Duration::from_secs(120);

enum Msg {
    Tested(Result<romm::ServerInfo, ApiError>),
    /// The summary and how many games the cached list had before.
    Synced(Result<(romm::SyncSummary, usize), ApiError>),
    CoversDone(usize),
    Paired(Result<String, ApiError>),
    /// No folder for this game's console yet.
    Ask(GameId, PathBuf),
    Downloaded {
        id: GameId,
        name: String,
        result: Result<(PathBuf, Option<PathBuf>), ApiError>,
    },
}

pub struct RommState {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    pub testing: bool,
    /// Ok(summary) / Err(message) of the last Test connection.
    pub test_result: Option<Result<String, String>>,
    pub syncing: bool,
    pub sync_result: Option<Result<String, String>>,
    last_sync: Option<Instant>,
    pub pairing: bool,
    pub pair_result: Option<Result<(), String>>,
    pub downloads: HashMap<GameId, Arc<Progress>>,
    /// Ask where downloads go (game, suggested folder).
    pub ask: Option<(GameId, PathBuf)>,
    /// The next rescan is one a sync / download asked for: don't sync again for it.
    skip_sync: bool,
    /// Rescans asked for (tests count them instead of scanning the machine).
    pub(crate) rescans: u32,
}

impl Default for RommState {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        RommState {
            tx,
            rx,
            testing: false,
            test_result: None,
            syncing: false,
            sync_result: None,
            last_sync: None,
            pairing: false,
            pair_result: None,
            downloads: HashMap::new(),
            ask: None,
            skip_sync: false,
            rescans: 0,
        }
    }
}

/// RomM's ROM id of a RomM game.
fn rom_id(g: &Game) -> Option<u64> {
    (g.source == Source::Romm).then(|| g.source_id.parse().ok()).flatten()
}

impl AppState {
    pub fn romm_set_up(&self) -> bool {
        !self.config.romm.url.trim().is_empty() && self.config.source_enabled(Source::Romm)
    }

    fn romm_spawn(&self, name: &str, job: impl FnOnce(&Sender<Msg>) + Send + 'static) {
        let tx = self.romm.tx.clone();
        let ctx = self.ctx().clone();
        let spawned = std::thread::Builder::new().name(format!("romm-{name}")).spawn(move || {
            job(&tx);
            ctx.request_repaint();
        });
        if let Err(e) = spawned {
            log::warn!("romm: couldn't start a thread: {e}");
        }
    }

    /// Settings → Libraries → RomM server → Test connection (a successful test also syncs).
    pub fn romm_test(&mut self) {
        if self.romm.testing {
            return;
        }
        let server = match Server::from_config(&self.config.romm) {
            Ok(s) => s,
            Err(e) => {
                self.romm.test_result = Some(Err(e.to_string()));
                return;
            }
        };
        self.romm.testing = true;
        self.romm.test_result = None;
        self.romm_spawn("test", move |tx| {
            let _ = tx.send(Msg::Tested(romm::test_connection(&server)));
        });
    }

    /// Read the server's list (and its covers) in the background; a changed list rescans.
    pub fn romm_sync(&mut self) {
        if self.romm.syncing || !self.romm_set_up() {
            return;
        }
        let server = match Server::from_config(&self.config.romm) {
            Ok(s) => s,
            Err(e) => {
                self.romm.sync_result = Some(Err(e.to_string()));
                return;
            }
        };
        self.romm.syncing = true;
        self.romm.last_sync = Some(Instant::now());
        let data = self.paths.data_dir.clone();
        self.romm_spawn("sync", move |tx| {
            let before = romm::load_cache(&data).map_or(0, |l| l.games.len());
            match romm::sync(&server, &data, None) {
                Ok((lib, summary)) => {
                    let _ = tx.send(Msg::Synced(Ok((summary, before))));
                    let n = romm::fetch_covers(&server, &data, &lib, None);
                    let _ = tx.send(Msg::CoversDone(n));
                }
                Err(e) => {
                    let _ = tx.send(Msg::Synced(Err(e)));
                }
            }
        });
    }

    /// Called by `rescan`: keep the RomM list fresh (at most every two minutes), and forget the
    /// cached games once the server is no longer set up.
    pub(crate) fn romm_on_rescan(&mut self) {
        if std::mem::take(&mut self.romm.skip_sync) {
            return;
        }
        if self.config.romm.url.trim().is_empty() {
            romm::clear_cache(&self.paths.data_dir);
            return;
        }
        // Screenshot runs only ever talk to a server on this machine (a local mock).
        if crate::art::net::check_allowed(self.config.romm.url.trim()).is_err() {
            return;
        }
        if self.romm.last_sync.is_none_or(|t| t.elapsed() >= RESYNC_AFTER) {
            self.romm_sync();
        }
    }

    /// A rescan for what RomM changed (it doesn't sync again).
    fn romm_rescan(&mut self) {
        self.romm.rescans += 1;
        if cfg!(test) {
            // Tests never scan the machine.
            return;
        }
        self.romm.skip_sync = true;
        self.rescan();
    }

    /// The address or sign-in changed: old results no longer apply.
    pub fn romm_settings_changed(&mut self) {
        self.romm.test_result = None;
        self.romm.sync_result = None;
        self.romm.last_sync = None;
    }

    /// A pairing code from RomM → the token, stored in the settings.
    pub fn romm_pair(&mut self, code: &str) {
        if self.romm.pairing {
            return;
        }
        let server = match Server::from_config(&crate::config::RommConfig {
            token: None,
            username: None,
            password: None,
            ..self.config.romm.clone()
        }) {
            Ok(s) => s,
            Err(e) => {
                self.romm.pair_result = Some(Err(e.to_string()));
                return;
            }
        };
        self.romm.pairing = true;
        self.romm.pair_result = None;
        let code = code.to_string();
        self.romm_spawn("pair", move |tx| {
            let _ = tx.send(Msg::Paired(romm::exchange_code(&server, &code)));
        });
    }

    pub(crate) fn poll_romm(&mut self) {
        let msgs: Vec<Msg> = self.romm.rx.try_iter().collect();
        for m in msgs {
            match m {
                Msg::Tested(r) => {
                    self.romm.testing = false;
                    match r {
                        Ok(info) => {
                            self.romm.test_result = Some(Ok(info.summary()));
                            self.romm.last_sync = None;
                            self.romm_sync();
                        }
                        Err(e) => self.romm.test_result = Some(Err(e.to_string())),
                    }
                }
                Msg::Synced(r) => {
                    self.romm.syncing = false;
                    match r {
                        Ok((s, before)) => {
                            self.romm.sync_result = Some(Ok(format!("{} on the server", romm::count(s.games, "game"))));
                            let narrowed = self.mode == Mode::Desktop
                                && self.config.desktop.preset != crate::config::QuickPreset::All;
                            if let Some(note) = first_sync_note(before, s.games, narrowed) {
                                self.notify_long(note);
                            }
                            if s.changed {
                                self.romm_rescan();
                            }
                        }
                        Err(e) => {
                            log::warn!("romm sync: {e}");
                            self.romm.sync_result = Some(Err(e.to_string()));
                        }
                    }
                }
                Msg::CoversDone(n) => {
                    if n > 0 {
                        self.romm_rescan();
                    }
                }
                Msg::Paired(r) => {
                    self.romm.pairing = false;
                    match r {
                        Ok(token) => {
                            self.config.romm.token = Some(token);
                            self.save_config();
                            self.romm.pair_result = Some(Ok(()));
                            self.romm_settings_changed();
                            self.romm_test();
                        }
                        Err(e) => self.romm.pair_result = Some(Err(e.to_string())),
                    }
                }
                Msg::Ask(id, suggest) => {
                    self.romm.downloads.remove(&id);
                    if self.mode == Mode::Controller {
                        // No folder picker on a TV: the suggestion, said in the toast.
                        self.romm_folder_chosen(id, Some(suggest));
                    } else {
                        self.romm.ask = Some((id, suggest));
                    }
                }
                Msg::Downloaded { id, name, result } => {
                    self.romm.downloads.remove(&id);
                    match result {
                        Ok((path, add_folder)) => {
                            if let Some(dir) = add_folder {
                                self.romm_add_rom_folder(dir);
                            }
                            let short = crate::emulation::paths::host_path(&path);
                            self.notify_long(format!(
                                "{name} is downloaded ({}). It plays with your emulator now.",
                                short.display()
                            ));
                            self.romm_rescan();
                        }
                        Err(ApiError::Cancelled) => self.notify(format!("Stopped downloading {name}")),
                        Err(e) => self.notify_error(format!("Couldn't download {name}: {e}")),
                    }
                }
            }
        }
    }

    /// The download folder as a ROM folder ("sub-folders are consoles"), unless it is one.
    fn romm_add_rom_folder(&mut self, dir: PathBuf) {
        let folders = &mut self.config.emulation.rom_folders;
        if folders.values().any(|f| f.path == dir) {
            return;
        }
        let key = (1..).map(|n| format!("f{n}")).find(|k| !folders.contains_key(k)).unwrap_or_else(|| "f0".into());
        folders.insert(key, RomFolder { path: dir, ..Default::default() });
        self.save_config();
    }

    /// Download is possible: a RomM game not downloaded yet, of a console Lodestar emulates.
    pub fn romm_can_download(&self, g: &Game) -> bool {
        rom_id(g).is_some()
            && !g.installed
            && g.platform.as_deref().and_then(crate::emulation::platforms::by_name).is_some()
    }

    pub fn romm_progress(&self, id: GameId) -> Option<Arc<Progress>> {
        self.romm.downloads.get(&id).cloned()
    }

    /// Stop a download (the download thread sees it at its next read and removes what it wrote).
    pub fn romm_cancel(&self, id: GameId) {
        if let Some(p) = self.romm.downloads.get(&id) {
            p.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Play / Download on a RomM game: download it into its console's ROM folder.
    pub fn romm_download(&mut self, id: GameId) {
        self.romm_download_into(id, None);
    }

    /// The answer to "where should downloads go?" (None = cancelled).
    pub fn romm_folder_chosen(&mut self, id: GameId, dir: Option<PathBuf>) {
        self.romm.ask = None;
        let Some(dir) = dir.filter(|d| d.is_absolute()) else { return };
        let root = download::download_root(&dir);
        self.config.romm.download_dir = Some(root.clone());
        self.save_config();
        self.notify(format!("Games from RomM go to {} (one folder per console)", root.display()));
        self.romm_download_into(id, Some(root));
    }

    fn romm_download_into(&mut self, id: GameId, chosen: Option<PathBuf>) {
        let Some(g) = self.library.get(id) else { return };
        let name = g.display_name().to_string();
        if !self.romm_can_download(g) {
            self.notify(format!(
                "{name} can't be downloaded: Lodestar doesn't know how to play {} games",
                g.platform.as_deref().unwrap_or("these")
            ));
            return;
        }
        let Some(rid) = rom_id(g) else { return };
        if let Some(p) = self.romm.downloads.get(&id) {
            let pct = p.fraction().map(|f| format!(" ({:.0} %)", f * 100.0)).unwrap_or_default();
            self.notify(format!("{name} is downloading{pct}"));
            return;
        }
        if self.no_launch {
            log::warn!("screenshot run: not downloading {name}");
            self.notify(format!("Skipped (screenshot run): download {name}"));
            return;
        }
        let server = match Server::from_config(&self.config.romm) {
            Ok(s) => s,
            Err(e) => {
                self.notify_error(e.to_string());
                return;
            }
        };
        let prog = Arc::new(Progress::default());
        self.romm.downloads.insert(id, prog.clone());
        let data = self.paths.data_dir.clone();
        let mut sctx = crate::scan::ScanContext::from_config(&self.config);
        sctx.home = self.paths.home.clone();
        let download_dir = chosen.or_else(|| self.config.romm.download_dir.clone());
        self.notify(format!("Downloading {name} from RomM…"));
        self.romm_spawn("download", move |tx| {
            let result = (|| -> Result<Option<(PathBuf, Option<PathBuf>)>, ApiError> {
                let lib = romm::load_cache(&data).ok_or(ApiError::BadReply("sync the server first".into()))?;
                // The list came from another server (the address changed): its ids mean other games here.
                if lib.server != server.base {
                    return Err(ApiError::Plain("The RomM server address changed since the last sync. Press Sync now in Settings → Libraries → RomM server, then try again".into()));
                }
                let game = lib
                    .games
                    .iter()
                    .find(|x| x.id == rid)
                    .ok_or(ApiError::BadReply("the game is no longer on the server".into()))?;
                let plat = game.platform_def().ok_or(ApiError::BadReply("unknown console".into()))?;
                let roots = crate::scan::emulation::roots::collect(&sctx, &crate::scan::emulation::roots::Env::real(&sctx));
                match download::pick_target(&roots, plat, download_dir.as_deref(), &sctx.home) {
                    Target::Ask { suggest } => {
                        let _ = tx.send(Msg::Ask(id, suggest));
                        Ok(None)
                    }
                    Target::Dir { dir, add_folder } => {
                        let path = download::download(&server, game, &dir, &prog)?;
                        Ok(Some((path, add_folder)))
                    }
                }
            })();
            match result {
                Ok(None) => {}
                Ok(Some(done)) => {
                    let _ = tx.send(Msg::Downloaded { id, name, result: Ok(done) });
                }
                Err(e) => {
                    let _ = tx.send(Msg::Downloaded { id, name, result: Err(e) });
                }
            }
        });
    }
}

/// The toast after a sync that brought the first games from the server: RomM games aren't
/// downloaded, so a desktop showing only installed (or favourite / played) games hides them —
/// say where they are instead of leaving the user looking for them.
fn first_sync_note(before: usize, after: usize, narrowed: bool) -> Option<String> {
    if before > 0 || after == 0 {
        return None;
    }
    let what = romm::count(after, "game");
    Some(if narrowed {
        format!(
            "{what} added from RomM. They aren't downloaded yet: choose Show: All (or Not installed in the sidebar) to see them."
        )
    } else {
        format!("{what} added from RomM. Press Download on one to play it.")
    })
}

#[cfg(test)]
mod note_tests {
    use super::first_sync_note;

    #[test]
    fn first_sync_says_where_the_games_are() {
        assert_eq!(first_sync_note(5, 12, true), None, "only the first sync");
        assert_eq!(first_sync_note(0, 0, true), None);
        let n = first_sync_note(0, 12, true).unwrap();
        assert!(n.starts_with("12 games added from RomM") && n.contains("Show: All"), "{n}");
        let n = first_sync_note(0, 1, false).unwrap();
        assert!(n.starts_with("1 game added") && !n.contains("Show:"), "{n}");
    }
}
