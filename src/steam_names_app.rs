//! The AppState side of the Steam name lookups (steam_names.rs): after a scan that left Steam
//! apps without a name, or when the owned-games list is due / the Steam Web API key changed, a
//! short-lived worker thread asks Steam, saves the caches and, when something changed, the
//! Steam library alone is scanned again.
//!
//! CONTRACT (UIs):
//!   st.steam_names.running              a lookup is running
//!   st.steam_names.status               plain words about the last lookup (None = nothing yet)

use crate::app::AppState;
use crate::model::Source;
use crate::scan::steam::names::{self, Needs};
use crate::steam_names::{self as sn, Job, Outcome};
use crossbeam_channel::{Receiver, bounded};
use std::time::{Duration, Instant};

/// After a failed lookup, scans don't start another for this long.
const RETRY_AFTER: Duration = Duration::from_secs(30 * 60);

#[derive(Default)]
pub struct SteamNamesState {
    rx: Option<Receiver<Outcome>>,
    pub running: bool,
    last_failed: Option<Instant>,
    /// The Steam Web API key as it was at the last lookup decision (None: no scan yet).
    key_seen: Option<String>,
    /// Plain words about the last lookup.
    pub status: Option<String>,
}

impl AppState {
    fn steam_key(&self) -> String {
        crate::config::secret(&self.config.achievements.steam_web_api_key).unwrap_or("").to_string()
    }

    fn steam_names_allowed(&self) -> bool {
        !crate::art::net::local_only()
            && self.config.scan.include_uninstalled
            && !self.config.scan.disabled_sources.iter().any(|k| k == Source::Steam.key())
    }

    /// A scan finished: look up what it couldn't name, and the owned-games list when due.
    pub(crate) fn steam_names_after_scan(&mut self, needs: &Needs) {
        let key = self.steam_key();
        let changed_key = self.steam_names.key_seen.as_ref().is_some_and(|k| *k != key);
        self.steam_names.key_seen = Some(key);
        if !self.steam_names_allowed()
            || self.steam_names.running
            || self.steam_names.last_failed.is_some_and(|t| t.elapsed() < RETRY_AFTER)
        {
            return;
        }
        let names: Vec<u32> =
            if self.config.metadata.steam_store { needs.names.iter().copied().collect() } else { Vec::new() };
        self.steam_names_start(names, changed_key);
    }

    /// Start a lookup (owned games only when a key is set and the list is due or `force_owned`).
    fn steam_names_start(&mut self, names: Vec<u32>, force_owned: bool) {
        let key = Some(self.steam_key()).filter(|k| !k.is_empty());
        if names.is_empty() && key.is_none() {
            return;
        }
        let cache_dir = self.paths.cache_dir.clone();
        let scan_ctx = crate::scan::ScanContext::from_config(&self.config);
        let secrets = self.config.achievements.secrets();
        let (tx, rx) = bounded(1);
        let ctx = self.ctx().clone();
        let spawned = std::thread::Builder::new().name("steam-names".into()).spawn(move || {
            let owned = if key.is_some() {
                sn::owned_due(&cache_dir, &sn::scanned_accounts(&scan_ctx), force_owned, names::now())
            } else {
                Vec::new()
            };
            let out = if names.is_empty() && owned.is_empty() {
                Outcome::default()
            } else {
                let job = Job { cache_dir, names, owned, key, secrets, local_only: crate::art::net::local_only() };
                sn::run(&crate::art::net::UreqNet::new(), &crate::art::net::Throttle::polite(), &job)
            };
            let _ = tx.send(out);
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.steam_names.rx = Some(rx);
            self.steam_names.running = true;
        }
    }

    /// Once per frame: a finished lookup, and a Steam Web API key that was set or changed.
    pub(crate) fn poll_steam_names(&mut self) {
        if let Some(out) = self.steam_names.rx.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.steam_names.rx = None;
            self.steam_names.running = false;
            match &out.error {
                Some(e) => {
                    self.steam_names.last_failed = Some(Instant::now());
                    self.steam_names.status = Some(e.clone());
                }
                None => {
                    if let Some(n) = out.owned_count {
                        self.steam_names.status =
                            Some(format!("Steam lists {n} game{} you own", if n == 1 { "" } else { "s" }));
                    }
                }
            }
            if out.library_changed() {
                self.rescan_local_libraries();
            }
        }
        // The key changed in Settings (after the first scan; only once it looks complete).
        let Some(seen) = self.steam_names.key_seen.clone() else { return };
        let key = self.steam_key();
        if key == seen || self.steam_names.running || self.scan.running {
            return;
        }
        if key.is_empty() {
            // Key removed: forget the owned-games list it fetched.
            self.steam_names.key_seen = Some(key);
            self.steam_names.status = None;
            let path = self.paths.cache_dir.join(names::OWNED_FILE);
            if path.is_file() && std::fs::remove_file(&path).is_ok() {
                self.rescan_local_libraries();
            }
        } else if sn::key_looks_complete(&key) && self.steam_names_allowed() {
            self.steam_names.key_seen = Some(key);
            self.steam_names.last_failed = None;
            self.steam_names_start(Vec::new(), true);
        }
    }
}
