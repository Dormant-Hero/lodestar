//! Lodestar's own updates in the app: the daily check, the download and the restart, as the
//! UIs see them (`st.update`) and ask for them (the `AppState` methods below). update.rs does the
//! work, on background threads; nothing here blocks the UI thread.
//!
//! Screenshot runs never check (they stay offline). `LODESTAR_FAKE_UPDATE` makes one up for
//! them instead: `<version>[:other|:readonly|:noasset][:downloading|:ready|:failed]`.

use crate::app::{AppState, Mode, ToastAction};
use crate::art::net::{self, Net};
use crate::update::{self, CheckErr, CheckRecord, Install, Release};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The first automatic check waits this long after start-up (the library and its artwork come
/// first), and so does removing what an update left behind.
const STARTUP_DELAY: Duration = Duration::from_secs(8);
/// After a check that couldn't reach GitHub, the next one comes this much sooner than a day.
const OFFLINE_RETRY_SECS: i64 = 3600;

/// Where an update is.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Idle,
    Downloading {
        got: u64,
        total: u64,
    },
    /// The new AppImage is in place; this session still runs the old one.
    Ready,
    /// The last download failed (why, for the user).
    Failed(String),
}

/// Who asked for a check: answers differ (only the user's own checks say "up to date" or why it
/// failed, and only they show a version the user chose to skip).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Check {
    Auto,
    /// Settings → General → Check now: the answer is a line under it.
    Settings,
    /// Menu → Check for updates…: the answer is a toast, or "What's new" for a newer version.
    Menu,
}

/// Whether a check by `by` that found `found` tells the user about it.
fn announce(found: &Release, skipped: Option<&str>, by: Check) -> bool {
    by != Check::Auto || skipped != Some(found.version.as_str())
}

enum Msg {
    Checked { by: Check, at: i64, result: Result<Release, CheckErr> },
    Progress(u64, u64),
    Installed,
    InstallFailed(String),
}

pub struct UpdateState {
    pub install: Install,
    /// The newest release when it is newer than this Lodestar.
    pub latest: Option<Release>,
    pub checked_at: Option<i64>,
    pub checking: bool,
    /// What Settings → Check now found ("Lodestar is up to date (…)." / why not).
    pub note: Option<String>,
    pub phase: Phase,
    /// A UI asked to show "What's new in …" (the desktop opens its dialog and clears it).
    pub show_notes: bool,
    /// Bumped on every change (the controller's menu refreshes on it).
    pub generation: u64,
    /// Automatic checks may run (a real GUI run that may go online).
    auto: bool,
    /// Made up for a screenshot run (`LODESTAR_FAKE_UPDATE`): nothing is downloaded or run.
    fake: bool,
    /// What a check finds in such a run.
    fake_release: Option<Release>,
    started: Option<Instant>,
    cleaned: bool,
    toasted: bool,
    record: PathBuf,
    net: Arc<dyn Net>,
    cancel: Arc<AtomicBool>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Default for UpdateState {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        UpdateState {
            install: Install::Other,
            latest: None,
            checked_at: None,
            checking: false,
            note: None,
            phase: Phase::Idle,
            show_notes: false,
            generation: 0,
            auto: false,
            fake: false,
            fake_release: None,
            started: None,
            cleaned: false,
            toasted: false,
            record: PathBuf::new(),
            net: Arc::new(net::UreqNet::new()),
            cancel: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
        }
    }
}

impl UpdateState {
    /// Lodestar can download and install `latest` itself.
    pub fn can_install(&self) -> bool {
        self.install.can_update() && self.latest.as_ref().is_some_and(|r| r.asset.is_some())
    }

    pub fn downloading(&self) -> Option<(u64, u64)> {
        match self.phase {
            Phase::Downloading { got, total } => Some((got, total)),
            _ => None,
        }
    }

    pub fn ready(&self) -> bool {
        self.phase == Phase::Ready
    }

    /// The release page to open (the newest release's, else the latest one).
    pub fn page(&self) -> String {
        self.latest.as_ref().map_or_else(|| update::RELEASES_PAGE.to_string(), |r| r.page.clone())
    }

    /// The label of the one button that moves an update along (toast, About): "Update" /
    /// "Restart now" / "Details".
    pub fn button_label(&self) -> &'static str {
        if self.ready() {
            "Restart now"
        } else if self.can_install() {
            "Update"
        } else {
            "Details"
        }
    }

    /// "42 %" of a download.
    pub fn percent(&self) -> Option<u32> {
        self.downloading().map(|(g, t)| g.saturating_mul(100).checked_div(t).unwrap_or(0).min(100) as u32)
    }

    /// Why Lodestar doesn't install `latest` itself (None when it does).
    pub fn why_not_installable(&self) -> Option<String> {
        if self.can_install() {
            return None;
        }
        Some(match &self.install {
            Install::AppImage { path, writable: false } => format!(
                "Lodestar can't replace its own file in {} (no permission to write there). Download the new \
                 AppImage from the release page.",
                path.parent().map_or_else(|| path.display().to_string(), |p| p.display().to_string())
            ),
            Install::AppImage { .. } => {
                "This release has no AppImage for this computer yet. It may still be uploading: try again later, \
                 or have a look at the release page."
                    .into()
            }
            Install::Other => "Download the new version from the release page, or update Lodestar the same way you \
                               installed it."
                .into(),
        })
    }

    fn bump(&mut self) {
        self.generation += 1;
    }
}

/// `LODESTAR_FAKE_UPDATE` (screenshot runs): the state it describes.
fn fake_state(spec: &str) -> Option<(Release, Install, Phase)> {
    let mut parts = spec.split(':');
    let version = parts.next().filter(|v| update::Version::parse(v).is_some())?.to_string();
    let mut install =
        Install::AppImage { path: PathBuf::from("/home/you/Applications/Lodestar.AppImage"), writable: true };
    let mut phase = Phase::Idle;
    let mut asset = true;
    for p in parts {
        match p {
            "other" => install = Install::Other,
            "readonly" => {
                install = Install::AppImage { path: PathBuf::from("/opt/lodestar/Lodestar.AppImage"), writable: false }
            }
            "noasset" => asset = false,
            "downloading" => phase = Phase::Downloading { got: 9_400_000, total: 21_700_000 },
            "ready" => phase = Phase::Ready,
            "failed" => phase = Phase::Failed("The download stopped. Check the connection and try again.".into()),
            _ => {}
        }
    }
    let name = update::asset_name(&version);
    let release = Release {
        tag: format!("v{version}"),
        page: format!("https://github.com/{}/{}/releases/tag/v{version}", update::OWNER, update::REPO),
        notes: "### Library\n- **Faster start-up** with big libraries: the first screen is up in half the time.\n\
                - Fixed covers that sometimes showed the wrong game after a rescan.\n\n### Controller mode\n\
                - The on-screen keyboard remembers where you left it.\n- Smoother scrolling through long lists."
            .into(),
        asset: asset.then(|| update::Asset {
            url: format!("https://github.com/{}/{}/releases/download/v{version}/{name}", update::OWNER, update::REPO),
            name,
            size: 21_700_000,
            sha256: None,
        }),
        version,
    };
    Some((release, install, phase))
}

impl AppState {
    /// At start-up. `real`: the GUI as the user runs it (not a screenshot run).
    pub(crate) fn init_updates(&mut self, real: bool) {
        let record = self.paths.cache_dir.join("update-check.json");
        let skipped = self.config.skipped_update.clone();
        let current = update::current();
        let shown = |r: &Release| r.is_newer_than(&current) && announce(r, skipped.as_deref(), Check::Auto);
        let u = &mut self.update;
        u.record = record;
        if !real {
            let spec = std::env::var("LODESTAR_FAKE_UPDATE").ok();
            if let Some((release, install, phase)) = spec.as_deref().and_then(fake_state) {
                u.latest = Some(release.clone()).filter(shown);
                u.fake_release = Some(release);
                u.install = install;
                u.phase = phase;
                u.fake = true;
                u.checked_at = Some(crate::model::now_unix());
                self.update_found_toast();
            }
            return;
        }
        u.install = Install::from_env();
        u.auto = !net::local_only();
        let rec = CheckRecord::load(&u.record);
        u.checked_at = rec.checked_at;
        u.latest = rec.latest.filter(shown);
        u.started = Some(Instant::now());
        log::info!("update: running as {:?}", u.install);
        self.ctx().request_repaint_after(STARTUP_DELAY);
    }

    /// Per frame: answers from the update threads, the automatic check when it is due.
    pub(crate) fn poll_updates(&mut self) {
        while let Ok(msg) = self.update.rx.try_recv() {
            match msg {
                Msg::Checked { by, at, result } => self.update_checked(by, at, result),
                Msg::Progress(got, total) => {
                    if matches!(self.update.phase, Phase::Downloading { .. }) {
                        self.update.phase = Phase::Downloading { got, total };
                        self.update.bump();
                    }
                }
                Msg::Installed => {
                    self.update.phase = Phase::Ready;
                    self.update.bump();
                    let v = self.update.latest.as_ref().map(|r| r.version.clone()).unwrap_or_default();
                    self.push_action_toast(format!("Lodestar {v} is installed. Restart Lodestar to use it."));
                }
                Msg::InstallFailed(e) => {
                    self.update.bump();
                    if e == "Cancelled." {
                        self.update.phase = Phase::Idle;
                    } else {
                        self.update.phase = Phase::Failed(e.clone());
                        self.notify_error(format!("The update didn't install: {e}"));
                    }
                }
            }
        }
        let Some(started) = self.update.started else { return };
        if started.elapsed() < STARTUP_DELAY {
            return;
        }
        if !self.update.cleaned {
            self.update.cleaned = true;
            if let Some(path) = self.update.install.appimage().map(PathBuf::from) {
                // This start worked: what an earlier update left behind can go.
                let _ = std::thread::Builder::new().name("update-clean".into()).spawn(move || {
                    let n = update::clean_leftovers(&path);
                    if n > 0 {
                        log::info!("update: removed {n} file(s) an earlier update left");
                    }
                });
            }
        }
        let u = &self.update;
        if !u.auto || u.checking || u.phase != Phase::Idle || !self.config.check_updates {
            return;
        }
        let now = crate::model::now_unix();
        if update::auto_check_wanted(self.config.check_updates, net::local_only(), u.checked_at, now) {
            self.start_update_check(Check::Auto);
        } else if let Some(t) = u.checked_at {
            // Running for days: the next check when it is due.
            let wait = (t + update::CHECK_EVERY_SECS - now).clamp(60, update::CHECK_EVERY_SECS) as u64;
            self.ctx().request_repaint_after(Duration::from_secs(wait));
        }
    }

    /// Settings → General → Check now (also when automatic checks are off).
    pub fn check_updates_now(&mut self) {
        self.start_update_check(Check::Settings);
    }

    /// Menu → Check for updates… (also when automatic checks are off).
    pub fn check_updates_from_menu(&mut self) {
        self.start_update_check(Check::Menu);
    }

    fn start_update_check(&mut self, by: Check) {
        if self.update.checking {
            return;
        }
        if by != Check::Auto {
            self.update.note = None;
        }
        if self.update.fake {
            let result = self.update.fake_release.clone().ok_or(CheckErr::NoRelease);
            self.update_checked(by, crate::model::now_unix(), result);
            return;
        }
        self.update.checking = true;
        self.update.bump();
        let (tx, net, ctx) = (self.update.tx.clone(), self.update.net.clone(), self.ctx().clone());
        let spawned = std::thread::Builder::new().name("update-check".into()).spawn(move || {
            let result = update::fetch_latest(&*net);
            let _ = tx.send(Msg::Checked { by, at: crate::model::now_unix(), result });
            ctx.request_repaint();
        });
        if spawned.is_err() {
            self.update.checking = false;
        }
    }

    fn update_checked(&mut self, by: Check, at: i64, result: Result<Release, CheckErr>) {
        let skipped = self.config.skipped_update.clone();
        let u = &mut self.update;
        u.checking = false;
        u.bump();
        let current = update::current();
        let mut told = None;
        match result {
            Ok(r) if r.is_newer_than(&current) => {
                log::info!("update: Lodestar {} is available (this is {current})", r.version);
                u.checked_at = Some(at);
                if announce(&r, skipped.as_deref(), by) {
                    u.latest = Some(r);
                    u.show_notes |= by == Check::Menu;
                } else {
                    log::info!("update: {} was skipped", r.version);
                    u.latest = None;
                }
            }
            Ok(r) => {
                log::info!("update: up to date ({current}, newest release {})", r.version);
                u.checked_at = Some(at);
                u.latest = None;
                told = Some((format!("Lodestar is up to date ({current})."), false));
            }
            Err(e) => {
                log::info!("update: no answer ({e:?})");
                // Offline: try again in an hour rather than tomorrow.
                u.checked_at = Some(if matches!(e, CheckErr::Offline(_)) {
                    at - update::CHECK_EVERY_SECS + OFFLINE_RETRY_SECS
                } else {
                    at
                });
                told = Some((e.to_string(), true));
            }
        }
        if !u.fake {
            CheckRecord { checked_at: u.checked_at, latest: u.latest.clone() }.save(&u.record);
        }
        match (by, told) {
            (Check::Settings, Some((text, _))) => self.update.note = Some(text),
            (Check::Menu, Some((text, false))) => self.notify(text),
            (Check::Menu, Some((text, true))) => self.notify_error(text),
            (Check::Auto, _) if self.update.latest.is_some() => self.update_found_toast(),
            _ => {}
        }
    }

    /// The update on show is the version the user chose to skip (found by a check they asked
    /// for).
    pub fn update_skipped(&self) -> bool {
        let skipped = self.config.skipped_update.as_deref();
        self.update.latest.as_ref().is_some_and(|r| skipped == Some(r.version.as_str()))
    }

    /// "Skip this version": automatic checks stay quiet about it from now on.
    pub fn skip_update(&mut self) {
        let Some(r) = self.update.latest.take() else { return };
        log::info!("update: skipping {}", r.version);
        self.config.skipped_update = Some(r.version);
        self.save_config();
        self.update.note = None;
        self.update.bump();
    }

    /// "Lodestar 0.17.1 is available", once a session.
    fn update_found_toast(&mut self) {
        if std::mem::replace(&mut self.update.toasted, true) {
            return;
        }
        let Some(v) = self.update.latest.as_ref().map(|r| r.version.clone()) else { return };
        let text = if self.mode == Mode::Controller {
            format!("Lodestar {v} is available: open the menu → Update Lodestar")
        } else {
            format!("Lodestar {v} is available")
        };
        self.push_action_toast(text);
    }

    fn push_action_toast(&mut self, text: String) {
        self.push_toast_with(text, Some(ToastAction::Update), Duration::from_secs(12));
    }

    /// Download and install `latest` (the AppImage only).
    pub fn start_update(&mut self) {
        if !self.update.can_install() || matches!(self.update.phase, Phase::Downloading { .. } | Phase::Ready) {
            return;
        }
        let (Some(asset), Some(target)) = (
            self.update.latest.as_ref().and_then(|r| r.asset.clone()),
            self.update.install.appimage().map(PathBuf::from),
        ) else {
            return;
        };
        self.update.phase = Phase::Downloading { got: 0, total: asset.size };
        self.update.bump();
        if self.update.fake {
            self.update.phase = Phase::Downloading { got: asset.size * 2 / 5, total: asset.size };
            return;
        }
        self.update.cancel = Arc::new(AtomicBool::new(false));
        let (tx, net, ctx, cancel) =
            (self.update.tx.clone(), self.update.net.clone(), self.ctx().clone(), self.update.cancel.clone());
        let spawned = std::thread::Builder::new().name("update-download".into()).spawn(move || {
            let mut last = Instant::now();
            let r = update::download_and_replace(&*net, &asset, &target, &cancel, &mut |got, total| {
                // A few updates a second are plenty for a progress bar.
                if got == total || last.elapsed() >= Duration::from_millis(150) {
                    last = Instant::now();
                    let _ = tx.send(Msg::Progress(got, total));
                    ctx.request_repaint();
                }
            });
            let _ = tx.send(match r {
                Ok(()) => Msg::Installed,
                Err(e) => Msg::InstallFailed(e),
            });
            ctx.request_repaint();
        });
        if spawned.is_err() {
            self.update.phase = Phase::Failed("Couldn't start the download.".into());
        }
    }

    pub fn cancel_update(&mut self) {
        self.update.cancel.store(true, Ordering::Relaxed);
        if self.update.fake {
            self.update.phase = Phase::Idle;
            self.update.bump();
        }
    }

    /// "Restart now": quit, then start the new AppImage with the same arguments (main.rs).
    pub fn restart_after_update(&mut self) {
        if !self.update.ready() {
            return;
        }
        let Some(path) = self.update.install.appimage().map(PathBuf::from) else { return };
        if self.no_launch {
            self.notify("Restart skipped (screenshot run)");
            return;
        }
        update::request_restart(&path);
        self.quit();
    }

    /// A toast's button was clicked.
    pub(crate) fn toast_action(&mut self, a: ToastAction) {
        match a {
            ToastAction::Update => {
                if self.update.ready() {
                    self.restart_after_update();
                } else {
                    self.update.show_notes = true;
                }
            }
        }
        self.ctx().request_repaint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test state that may "go online" through `net`.
    fn state(root: &std::path::Path, net: Arc<dyn Net>) -> AppState {
        let mut st = AppState::for_test(root);
        st.update.record = root.join("update-check.json");
        st.update.net = net;
        st.update.auto = true;
        st.update.started = Some(Instant::now() - STARTUP_DELAY);
        st
    }

    /// Answers every check with `.0` and counts the checks in `.1`.
    struct Answer(Vec<u8>, Arc<parking_lot::Mutex<u32>>);
    impl Net for Answer {
        fn get(&self, _: &str, _: Option<&str>, _: u64) -> Result<net::HttpResponse, String> {
            Err("no".into())
        }
        fn head(&self, _: &str) -> Result<u16, String> {
            Err("no".into())
        }
        fn get_with(&self, _: &str, _: &[(&str, String)], _: u64) -> Result<net::HttpResponse, String> {
            *self.1.lock() += 1;
            Ok(net::HttpResponse { status: 200, body: self.0.clone(), headers: vec![] })
        }
    }

    /// No connection.
    struct Offline;
    impl Net for Offline {
        fn get(&self, _: &str, _: Option<&str>, _: u64) -> Result<net::HttpResponse, String> {
            Err("offline".into())
        }
        fn head(&self, _: &str) -> Result<u16, String> {
            Err("offline".into())
        }
        fn get_with(&self, _: &str, _: &[(&str, String)], _: u64) -> Result<net::HttpResponse, String> {
            Err("offline".into())
        }
    }

    fn newer_release() -> Vec<u8> {
        serde_json::json!({
            "tag_name": "v99.0.0",
            "html_url": "https://github.com/Dormant-Hero/lodestar/releases/tag/v99.0.0",
            "body": "- New",
            "assets": []
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn a_due_check_finds_a_newer_version_toasts_once_and_remembers_it() {
        let dir = tempfile::tempdir().unwrap();
        let asked = Arc::new(parking_lot::Mutex::new(0));
        let mut st = state(dir.path(), Arc::new(Answer(newer_release(), asked.clone())));
        assert!(st.poll_until(Duration::from_secs(5), |s| s.update.latest.is_some()));
        assert_eq!(*asked.lock(), 1);
        assert_eq!(st.update.latest.as_ref().unwrap().version, "99.0.0");
        let toasts: Vec<_> = st.toasts.iter().filter(|t| t.action.is_some()).map(|t| t.text.clone()).collect();
        assert_eq!(toasts, ["Lodestar 99.0.0 is available"]);
        assert_eq!(st.update.button_label(), "Details", "not an AppImage: notes and the release page");
        // Checked just now: polling again asks nobody.
        st.poll_until(Duration::from_millis(300), |_| false);
        assert_eq!(*asked.lock(), 1);
        let rec = CheckRecord::load(&dir.path().join("update-check.json"));
        assert_eq!(rec.latest.map(|r| r.version).as_deref(), Some("99.0.0"));
    }

    #[test]
    fn with_check_for_updates_off_nothing_is_asked() {
        let dir = tempfile::tempdir().unwrap();
        let asked = Arc::new(parking_lot::Mutex::new(0));
        let mut st = state(dir.path(), Arc::new(Answer(newer_release(), asked.clone())));
        st.config.check_updates = false;
        st.poll_until(Duration::from_millis(400), |_| false);
        assert_eq!(*asked.lock(), 0);
        assert!(st.update.latest.is_none());
        // Check now still asks.
        st.check_updates_now();
        assert!(st.poll_until(Duration::from_secs(5), |s| s.update.latest.is_some()));
        assert_eq!(*asked.lock(), 1);
    }

    #[test]
    fn without_automatic_checks_nothing_is_asked() {
        let dir = tempfile::tempdir().unwrap();
        let asked = Arc::new(parking_lot::Mutex::new(0));
        let mut st = state(dir.path(), Arc::new(Answer(newer_release(), asked.clone())));
        st.update.auto = false; // what init_updates leaves for screenshot (local-only) runs
        st.poll_until(Duration::from_millis(400), |_| false);
        assert_eq!(*asked.lock(), 0);
        // A plain test state never even starts the clock.
        let mut st = AppState::for_test(&dir.path().join("b"));
        st.poll_until(Duration::from_millis(200), |_| false);
        assert!(!st.update.checking && st.update.latest.is_none());
    }

    #[test]
    fn an_older_or_equal_release_is_up_to_date() {
        let dir = tempfile::tempdir().unwrap();
        let body = format!("{{\"tag_name\": \"v{}\"}}", env!("CARGO_PKG_VERSION")).into_bytes();
        let mut st = state(dir.path(), Arc::new(Answer(body, Default::default())));
        st.check_updates_now();
        assert!(st.poll_until(Duration::from_secs(5), |s| !s.update.checking && s.update.note.is_some()));
        assert!(st.update.latest.is_none());
        assert!(st.update.note.as_deref().unwrap().starts_with("Lodestar is up to date"));
        assert!(st.toasts.iter().all(|t| t.action.is_none()));
    }

    fn texts(st: &AppState) -> Vec<String> {
        st.toasts.iter().map(|t| t.text.clone()).collect()
    }

    #[test]
    fn the_menu_check_answers_with_a_toast_or_the_release_notes() {
        let dir = tempfile::tempdir().unwrap();
        let current = env!("CARGO_PKG_VERSION");
        let body = format!("{{\"tag_name\": \"v{current}\"}}").into_bytes();
        let mut st = state(dir.path(), Arc::new(Answer(body, Default::default())));
        st.config.check_updates = false;
        st.check_updates_from_menu();
        assert!(st.poll_until(Duration::from_secs(5), |s| !s.update.checking));
        assert_eq!(texts(&st), [format!("Lodestar is up to date ({current}).")]);
        assert!(st.update.note.is_none(), "the menu's answer is the toast, not Settings' line");

        let mut st = state(&dir.path().join("offline"), Arc::new(Offline));
        st.config.check_updates = false;
        st.check_updates_from_menu();
        assert!(st.poll_until(Duration::from_secs(5), |s| !s.update.checking));
        assert_eq!(texts(&st), ["Couldn't reach GitHub. Try again later."]);
        assert!(st.toasts[0].error);

        let mut st = state(&dir.path().join("newer"), Arc::new(Answer(newer_release(), Default::default())));
        st.config.check_updates = false;
        st.check_updates_from_menu();
        assert!(st.poll_until(Duration::from_secs(5), |s| s.update.latest.is_some()));
        assert!(st.update.show_notes, "a newer version opens What's new");
    }

    #[test]
    fn a_skipped_version_is_announced_only_when_the_user_asks() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = state(dir.path(), Arc::new(Answer(newer_release(), Default::default())));
        st.config.skipped_update = Some("99.0.0".into());
        // The automatic check finds it but stays quiet.
        assert!(st.poll_until(Duration::from_secs(5), |s| s.update.checked_at.is_some() && !s.update.checking));
        assert!(st.update.latest.is_none());
        assert!(st.toasts.is_empty());
        // A check the user asks for shows it, marked as skipped.
        st.check_updates_now();
        assert!(st.poll_until(Duration::from_secs(5), |s| s.update.latest.is_some()));
        assert!(st.update_skipped());
        // A newer one than the skipped version is announced again.
        let r = st.update.latest.clone().unwrap();
        let newer = Release { version: "99.1.0".into(), ..r.clone() };
        assert!(announce(&newer, Some("99.0.0"), Check::Auto));
        assert!(!announce(&r, Some("99.0.0"), Check::Auto));
        assert!(announce(&r, Some("99.0.0"), Check::Menu) && announce(&r, None, Check::Auto));
    }

    #[test]
    fn skip_this_version_is_remembered_in_the_config() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = state(dir.path(), Arc::new(Answer(newer_release(), Default::default())));
        assert!(st.poll_until(Duration::from_secs(5), |s| s.update.latest.is_some()));
        assert!(!st.update_skipped());
        let found = st.update.latest.clone().unwrap();
        st.skip_update();
        assert!(st.update.latest.is_none());
        assert_eq!(st.config.skipped_update.as_deref(), Some("99.0.0"));
        // The next start doesn't bring it back from the last check's record either (a newer
        // release in it would come back).
        let record = CheckRecord { checked_at: Some(1), latest: Some(found) };
        for (skipped, shown) in [(Some("99.0.0"), false), (None, true)] {
            let mut again = AppState::for_test(&dir.path().join(format!("again-{shown}")));
            std::fs::create_dir_all(&again.paths.cache_dir).unwrap();
            record.save(&again.paths.cache_dir.join("update-check.json"));
            again.config.skipped_update = skipped.map(str::to_string);
            again.init_updates(true);
            assert_eq!(again.update.latest.is_some(), shown);
        }
        // Old config files (no such key) load with nothing skipped.
        let old: crate::config::Config = toml::from_str("check_updates = true\n").unwrap();
        assert_eq!(old.skipped_update, None);
    }

    #[test]
    fn the_fake_update_describes_each_state() {
        let (r, i, p) = fake_state("0.18.0:downloading").unwrap();
        assert_eq!(r.version, "0.18.0");
        assert!(i.can_update() && r.asset.is_some());
        assert!(matches!(p, Phase::Downloading { .. }));
        let (_, i, p) = fake_state("0.18.0:other:ready").unwrap();
        assert_eq!((i, p), (Install::Other, Phase::Ready));
        assert!(fake_state("soon").is_none());
    }
}
