//! Cloud gaming entries (Add game → Cloud game…): GeForce NOW, Xbox Cloud Gaming, Amazon Luna,
//! Boosteroid or any other web service, opened through the service's own app when one is
//! installed, else a web browser (its own window, full screen) or a tab in the default browser.
//!
//! A cloud game is a hand-added game (`Source::Manual`, so it is removed / edited like one) whose
//! `source_id` says which service it is on and how it opens:
//! `cloud:<service>:<open>:<fs|win>:<nonce>` (see [`Entry`]). Its stored launch is
//! `LaunchSpec::Url(<game link or the service's home page>)`; the command that really runs is
//! rebuilt at launch time from what is installed now ([`launch_for`], like emulated games), so a
//! GeForce NOW app or a browser installed later is picked up and an uninstalled one never
//! leaves the game unplayable. Platform "cloud"; artwork / metadata come by name like any game.
//!
//! Tracking: the started app / browser window is followed like any game. A browser (or app) that
//! was already running takes the page over and the started process exits at once, so then the
//! session isn't followed and the user is told why ([`launch_note`]); a tab in the default
//! browser (xdg-open) is never followed.
//!
//! Findings (Nov 2026) behind the presets: NVIDIA's native Linux GeForce NOW app is a Flatpak
//! from NVIDIA's own remote (`com.nvidia.geforcenow`, `flatpak run com.nvidia.geforcenow`); no
//! documented way to start one game from its command line or a URL, so it opens at its library.
//! In browsers GeForce NOW streams in Chrome / Edge (Chromium-based), not in Firefox. Xbox Cloud
//! Gaming: www.xbox.com/play (game pages `…/play/games/<slug>/<product id>`), officially Edge and
//! Chrome, Firefox mostly works; community app Greenlight (`io.github.unknownskl.greenlight`,
//! `--fullscreen`; `--connect=xcloud_<titleId>` needs an id only its signed-in catalogue knows)
//! and XStreaming (`io.github.Geocld.XStreamingDesktop`). Amazon Luna: browser only
//! (luna.amazon.com, Chrome / Edge). Boosteroid: cloud.boosteroid.com or its native client
//! (`.deb`: `/opt/BoosteroidGamesS.R.L./bin/Boosteroid`, AUR: `/usr/bin/boosteroid`).
//! No service offers a documented local (no-login) library list, so there is no import.

use crate::model::*;
use std::path::{Path, PathBuf};
use std::sync::Arc;

mod app;
pub use app::NewCloudGame;
pub mod gfn;
#[cfg(test)]
mod tests;

/// The platform of cloud games.
pub const PLATFORM: &str = "cloud";

// ------------------------------------------------------------------------------- services

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Service {
    GeForceNow,
    Xbox,
    Luna,
    Boosteroid,
    /// Any other web service (the user gives the link).
    Other,
}

/// A service's own desktop app.
#[derive(Debug)]
pub struct AppDef {
    pub name: &'static str,
    pub flatpak: Option<&'static str>,
    /// Programs on PATH (plain names) or absolute paths.
    pub programs: &'static [&'static str],
    /// Its full-screen switch, when it has a documented one.
    pub fullscreen_arg: Option<&'static str>,
    /// Process names (argv[0] file names) of the native program while it runs.
    pub procs: &'static [&'static str],
}

const GFN_APPS: &[AppDef] = &[AppDef {
    name: "GeForce NOW app",
    flatpak: Some("com.nvidia.geforcenow"),
    programs: &[],
    fullscreen_arg: None,
    procs: &[],
}];

const XBOX_APPS: &[AppDef] = &[
    AppDef {
        name: "Greenlight",
        flatpak: Some("io.github.unknownskl.greenlight"),
        programs: &["greenlight"],
        fullscreen_arg: Some("--fullscreen"),
        procs: &["greenlight"],
    },
    AppDef {
        name: "XStreaming",
        flatpak: Some("io.github.Geocld.XStreamingDesktop"),
        programs: &[],
        fullscreen_arg: None,
        procs: &[],
    },
];

const BOOSTEROID_APPS: &[AppDef] = &[AppDef {
    name: "Boosteroid app",
    flatpak: None,
    programs: &["boosteroid", "/opt/BoosteroidGamesS.R.L./bin/Boosteroid"],
    fullscreen_arg: None,
    procs: &["Boosteroid", "boosteroid.bin"],
}];

impl Service {
    pub const ALL: [Service; 5] =
        [Service::GeForceNow, Service::Xbox, Service::Luna, Service::Boosteroid, Service::Other];

    /// Stable key (in `source_id`). Never change these strings.
    pub fn key(self) -> &'static str {
        match self {
            Service::GeForceNow => "gfn",
            Service::Xbox => "xbox",
            Service::Luna => "luna",
            Service::Boosteroid => "boosteroid",
            Service::Other => "web",
        }
    }

    pub fn from_key(k: &str) -> Option<Service> {
        Service::ALL.into_iter().find(|s| s.key() == k)
    }

    /// Its games' [`Game::source_group_key`] ("cloud-gfn"): how lists that go by source (Group by
    /// Source, the desktop's Sources entries) tell the services apart.
    pub fn group_key(self) -> &'static str {
        match self {
            Service::GeForceNow => "cloud-gfn",
            Service::Xbox => "cloud-xbox",
            Service::Luna => "cloud-luna",
            Service::Boosteroid => "cloud-boosteroid",
            Service::Other => "cloud-web",
        }
    }

    /// The name shown as the game's source.
    pub fn name(self) -> &'static str {
        match self {
            Service::GeForceNow => "GeForce NOW",
            Service::Xbox => "Xbox Cloud Gaming",
            Service::Luna => "Amazon Luna",
            Service::Boosteroid => "Boosteroid",
            Service::Other => "Cloud gaming",
        }
    }

    /// The picker's label.
    pub fn label(self) -> &'static str {
        match self {
            Service::Other => "Other",
            Service::Xbox => "Xbox Cloud",
            s => s.name(),
        }
    }

    /// The service's web player.
    pub fn home(self) -> Option<&'static str> {
        match self {
            Service::GeForceNow => Some("https://play.geforcenow.com"),
            Service::Xbox => Some("https://www.xbox.com/play"),
            Service::Luna => Some("https://luna.amazon.com"),
            Service::Boosteroid => Some("https://cloud.boosteroid.com"),
            Service::Other => None,
        }
    }

    /// The service's colour (the picker's tile).
    pub fn color(self) -> [u8; 3] {
        match self {
            Service::GeForceNow => [0x76, 0xb9, 0x00],
            Service::Xbox => [0x10, 0x7c, 0x10],
            Service::Luna => [0x7b, 0x3f, 0xe4],
            Service::Boosteroid => [0x2b, 0x4c, 0xd6],
            Service::Other => [0x60, 0x6a, 0x78],
        }
    }

    /// Letters on the picker's tile.
    pub fn badge(self) -> &'static str {
        match self {
            Service::GeForceNow => "GFN",
            Service::Xbox => "XBOX",
            Service::Luna => "LUNA",
            Service::Boosteroid => "B",
            Service::Other => "WEB",
        }
    }

    pub fn apps(self) -> &'static [AppDef] {
        match self {
            Service::GeForceNow => GFN_APPS,
            Service::Xbox => XBOX_APPS,
            Service::Boosteroid => BOOSTEROID_APPS,
            Service::Luna | Service::Other => &[],
        }
    }

    /// Which service a link belongs to (None: some other site).
    pub fn for_url(url: &str) -> Option<Service> {
        let u = Url::parse(url)?;
        let h = u.host.as_str();
        let on = |d: &str| h == d || h.ends_with(&format!(".{d}"));
        if on("geforcenow.com") || (on("nvidia.com") && u.path.contains("geforce-now")) {
            Some(Service::GeForceNow)
        } else if on("xbox.com") {
            Some(Service::Xbox)
        } else if h.starts_with("luna.amazon.") || h.contains(".luna.amazon.") {
            Some(Service::Luna)
        } else if on("boosteroid.com") {
            Some(Service::Boosteroid)
        } else {
            None
        }
    }

    /// Why a browser of `family` may not work for this service (None: it does).
    pub fn browser_warning(self, family: Family) -> Option<&'static str> {
        match (self, family) {
            (Service::GeForceNow, Family::Firefox) => Some(
                "GeForce NOW doesn't stream in Firefox. Install the GeForce NOW app from nvidia.com, \
                 or a Chrome-based browser (Chrome, Chromium, Edge, Brave), and Lodestar uses it.",
            ),
            (Service::Luna, Family::Firefox) => {
                Some("Amazon Luna supports Chrome and Edge; it may not start in Firefox.")
            }
            (Service::Xbox, Family::Firefox) => {
                Some("Xbox Cloud Gaming officially supports Chrome and Edge; Firefox mostly works.")
            }
            _ => None,
        }
    }
}

// ------------------------------------------------------------------------------- browsers

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    /// `--app=URL` (own window without tabs), `--start-fullscreen`.
    Chromium,
    /// `--new-window URL`, `--kiosk URL` (full screen).
    Firefox,
}

#[derive(Debug)]
pub struct BrowserDef {
    /// Stable key (in `source_id`).
    pub key: &'static str,
    pub name: &'static str,
    pub family: Family,
    pub flatpaks: &'static [&'static str],
    pub programs: &'static [&'static str],
    /// argv[0] file names of the native browser's processes.
    pub procs: &'static [&'static str],
    /// Desktop file ids of the native browser (default-browser lookup).
    pub desktop_ids: &'static [&'static str],
}

/// Chromium-based first (the services support them best), in the order auto picks them.
pub const BROWSERS: &[BrowserDef] = &[
    BrowserDef {
        key: "chrome",
        name: "Google Chrome",
        family: Family::Chromium,
        flatpaks: &["com.google.Chrome"],
        programs: &["google-chrome-stable", "google-chrome"],
        procs: &["chrome"],
        desktop_ids: &["google-chrome.desktop", "google-chrome-stable.desktop"],
    },
    BrowserDef {
        key: "edge",
        name: "Microsoft Edge",
        family: Family::Chromium,
        flatpaks: &["com.microsoft.Edge"],
        programs: &["microsoft-edge-stable", "microsoft-edge"],
        procs: &["msedge"],
        desktop_ids: &["microsoft-edge.desktop", "microsoft-edge-stable.desktop"],
    },
    BrowserDef {
        key: "chromium",
        name: "Chromium",
        family: Family::Chromium,
        flatpaks: &["org.chromium.Chromium", "io.github.ungoogled_software.ungoogled_chromium"],
        programs: &["chromium", "chromium-browser"],
        procs: &["chromium", "chromium-browser"],
        desktop_ids: &["chromium.desktop", "chromium-browser.desktop"],
    },
    BrowserDef {
        key: "brave",
        name: "Brave",
        family: Family::Chromium,
        flatpaks: &["com.brave.Browser"],
        programs: &["brave", "brave-browser"],
        procs: &["brave"],
        desktop_ids: &["brave-browser.desktop", "brave.desktop"],
    },
    BrowserDef {
        key: "vivaldi",
        name: "Vivaldi",
        family: Family::Chromium,
        flatpaks: &["com.vivaldi.Vivaldi"],
        programs: &["vivaldi-stable", "vivaldi"],
        procs: &["vivaldi-bin"],
        desktop_ids: &["vivaldi-stable.desktop", "vivaldi.desktop"],
    },
    BrowserDef {
        key: "firefox",
        name: "Firefox",
        family: Family::Firefox,
        flatpaks: &["org.mozilla.firefox"],
        programs: &["firefox"],
        procs: &["firefox", "firefox-bin"],
        desktop_ids: &["firefox.desktop", "firefox-esr.desktop"],
    },
    BrowserDef {
        key: "librewolf",
        name: "LibreWolf",
        family: Family::Firefox,
        flatpaks: &["io.gitlab.librewolf-community"],
        programs: &["librewolf"],
        procs: &["librewolf"],
        desktop_ids: &["librewolf.desktop"],
    },
    BrowserDef {
        key: "zen",
        name: "Zen Browser",
        family: Family::Firefox,
        flatpaks: &["app.zen_browser.zen"],
        programs: &["zen-browser", "zen"],
        procs: &["zen", "zen-bin"],
        desktop_ids: &["zen.desktop", "zen-browser.desktop"],
    },
];

// ------------------------------------------------------------------------------- detection

/// How something installed is started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    Flatpak(String),
    Native(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundApp {
    pub service: Service,
    pub name: &'static str,
    pub install: Install,
    pub fullscreen_arg: Option<&'static str>,
    pub procs: &'static [&'static str],
    /// The app's icon (Flatpak exports), for the picker.
    pub icon: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FoundBrowser {
    /// `firefox` (native) / `firefox@flatpak` (in `source_id`).
    pub key: String,
    /// "Firefox" / "Firefox (Flatpak)".
    pub name: String,
    pub family: Family,
    pub install: Install,
    pub procs: &'static [&'static str],
}

/// What is installed: service apps, browsers and which one is the default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    pub apps: Vec<FoundApp>,
    pub browsers: Vec<FoundBrowser>,
    /// Key of the default browser among `browsers`.
    pub default_browser: Option<String>,
}

impl Found {
    pub fn browser(&self, key: &str) -> Option<&FoundBrowser> {
        self.browsers.iter().find(|b| b.key == key)
    }
    pub fn apps_for(&self, s: Service) -> impl Iterator<Item = &FoundApp> {
        self.apps.iter().filter(move |a| a.service == s)
    }
    /// The default browser's name, when it is known.
    pub fn default_name(&self) -> Option<&str> {
        self.default_browser.as_deref().and_then(|k| self.browser(k)).map(|b| b.name.as_str())
    }
}

/// Where to look (the real machine, or a fixture in tests).
#[derive(Clone, Debug)]
pub struct Machine {
    pub system_flatpak: PathBuf,
    pub user_flatpak: PathBuf,
    pub path: Option<std::ffi::OsString>,
    /// Prefix for absolute program paths (`/` on the real machine).
    pub root: PathBuf,
    /// `mimeapps.list` files, most important first.
    pub mimeapps: Vec<PathBuf>,
}

impl Machine {
    pub fn system() -> Machine {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        Machine {
            // Test hook: the dev container sees the host's Flatpaks under /run/host.
            system_flatpak: std::env::var_os("LODESTAR_FLATPAK_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/var/lib/flatpak")),
            user_flatpak: home.join(".local/share/flatpak"),
            path: std::env::var_os("PATH"),
            root: PathBuf::from("/"),
            mimeapps: vec![
                config.join("mimeapps.list"),
                home.join(".local/share/applications/mimeapps.list"),
                PathBuf::from("/etc/xdg/mimeapps.list"),
                PathBuf::from("/usr/share/applications/mimeapps.list"),
            ],
        }
    }

    /// A fixture machine under `root`: `root/var/lib/flatpak`, `root/home/.local/share/flatpak`,
    /// `root/usr/bin` as PATH, `root/home/.config/mimeapps.list`.
    #[cfg(test)]
    pub fn under(root: &Path) -> Machine {
        Machine {
            system_flatpak: root.join("var/lib/flatpak"),
            user_flatpak: root.join("home/.local/share/flatpak"),
            path: Some(root.join("usr/bin").into_os_string()),
            root: root.to_path_buf(),
            mimeapps: vec![root.join("home/.config/mimeapps.list")],
        }
    }

    /// The Flatpak installation that has `id`, if any (user first).
    fn flatpak(&self, id: &str) -> Option<PathBuf> {
        [&self.user_flatpak, &self.system_flatpak]
            .into_iter()
            .find(|r| r.join("app").join(id).join("current").exists() || r.join("app").join(id).is_dir())
            .cloned()
    }

    fn program(&self, p: &str) -> Option<PathBuf> {
        if p.starts_with('/') {
            let full = self.root.join(p.trim_start_matches('/'));
            is_exe(&full).then(|| PathBuf::from(p))
        } else {
            crate::launch::find_in_path_with(p, self.path.as_deref())
        }
    }

    fn flatpak_icon(&self, root: &Path, id: &str) -> Option<PathBuf> {
        let base = root.join("exports/share/icons/hicolor");
        for size in ["128x128", "256x256", "512x512", "96x96", "64x64", "scalable"] {
            for ext in ["png", "svg"] {
                let p = base.join(size).join("apps").join(format!("{id}.{ext}"));
                if p.is_file() {
                    return Some(p);
                }
            }
        }
        None
    }

    /// The desktop id of the default web browser (`x-scheme-handler/https`, else `http`).
    fn default_browser_id(&self) -> Option<String> {
        for key in ["x-scheme-handler/https", "x-scheme-handler/http", "text/html"] {
            for f in &self.mimeapps {
                if let Some(id) = std::fs::read_to_string(f).ok().and_then(|s| mime_default(&s, key)) {
                    return Some(id);
                }
            }
        }
        None
    }
}

fn is_exe(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `[Default Applications] key=first.desktop;other.desktop;` → `first.desktop`.
fn mime_default(text: &str, key: &str) -> Option<String> {
    let mut in_defaults = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_defaults = l == "[Default Applications]";
            continue;
        }
        if !in_defaults {
            continue;
        }
        if let Some((k, v)) = l.split_once('=')
            && k.trim() == key
        {
            return v.split(';').map(str::trim).find(|s| !s.is_empty()).map(str::to_string);
        }
    }
    None
}

/// Look for the service apps and browsers on `m`. Cheap: a few dozen `stat`s.
pub fn detect(m: &Machine) -> Found {
    let mut f = Found::default();
    for s in Service::ALL {
        for a in s.apps() {
            let fp = a.flatpak.and_then(|id| m.flatpak(id).map(|root| (id, root)));
            let (install, icon) = if let Some((id, root)) = fp {
                (Install::Flatpak(id.to_string()), m.flatpak_icon(&root, id))
            } else if let Some(p) = a.programs.iter().find_map(|p| m.program(p)) {
                (Install::Native(p), None)
            } else {
                continue;
            };
            f.apps.push(FoundApp {
                service: s,
                name: a.name,
                install,
                fullscreen_arg: a.fullscreen_arg,
                procs: a.procs,
                icon,
            });
        }
    }
    let default_id = m.default_browser_id();
    for b in BROWSERS {
        if let Some(p) = b.programs.iter().find_map(|p| m.program(p)) {
            if default_id.as_deref().is_some_and(|d| b.desktop_ids.contains(&d)) {
                f.default_browser = Some(b.key.to_string());
            }
            f.browsers.push(FoundBrowser {
                key: b.key.to_string(),
                name: b.name.to_string(),
                family: b.family,
                install: Install::Native(p),
                procs: b.procs,
            });
        }
        if let Some(id) = b.flatpaks.iter().find(|id| m.flatpak(id).is_some()) {
            let key = format!("{}@flatpak", b.key);
            if default_id.as_deref() == Some(format!("{id}.desktop").as_str()) {
                f.default_browser = Some(key.clone());
            }
            let native_too = f.browsers.iter().any(|x| x.key == b.key);
            f.browsers.push(FoundBrowser {
                key,
                name: if native_too { format!("{} (Flatpak)", b.name) } else { b.name.to_string() },
                family: b.family,
                install: Install::Flatpak(id.to_string()),
                procs: &[],
            });
        }
    }
    f
}

// The published detection (the launch path reads it; it is detected again when it is a minute
// old, so an app or browser installed while Lodestar runs is used at the next Play). `refresh`
// runs on a helper thread when the Add dialog opens.
static FOUND: parking_lot::RwLock<Option<(std::time::Instant, Arc<Found>)>> = parking_lot::RwLock::new(None);

/// How long a detection is used before it is made again (on the next cloud launch).
const FOUND_FRESH: std::time::Duration = std::time::Duration::from_secs(60);

/// Detect on the real machine and publish the result.
pub fn refresh() -> Arc<Found> {
    let f = Arc::new(detect(&Machine::system()));
    *FOUND.write() = Some((std::time::Instant::now(), f.clone()));
    f
}

/// Publish a fixture detection (tests; never goes stale).
#[cfg(test)]
pub fn publish(f: Found) {
    let far = std::time::Instant::now() + std::time::Duration::from_secs(365 * 24 * 3600);
    *FOUND.write() = Some((far, Arc::new(f)));
}

/// What was detected last (detected now when nothing was published yet or it is a minute old;
/// a few dozen `stat`s).
pub fn published() -> Arc<Found> {
    if let Some((at, f)) = FOUND.read().clone()
        && at.elapsed() < FOUND_FRESH
    {
        return f;
    }
    refresh()
}

// ------------------------------------------------------------------------------- entries

/// How a cloud game opens.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Open {
    /// The best way installed right now (see [`plan`]).
    Auto,
    /// The service's app.
    App,
    /// This browser (a [`FoundBrowser::key`]) in its own window.
    Browser(String),
    /// A tab in the default browser (xdg-open).
    Tab,
}

impl Open {
    fn token(&self) -> String {
        match self {
            Open::Auto => "auto".into(),
            Open::App => "app".into(),
            Open::Tab => "tab".into(),
            Open::Browser(k) => format!("b-{k}"),
        }
    }
    fn from_token(t: &str) -> Option<Open> {
        Some(match t {
            "auto" => Open::Auto,
            "app" => Open::App,
            "tab" => Open::Tab,
            t => {
                let k = t.strip_prefix("b-")?;
                if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || c == '@' || c == '-' || c == '_') {
                    return None;
                }
                Open::Browser(k.to_string())
            }
        })
    }
}

/// A cloud game's settings, kept in its `source_id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub service: Service,
    pub open: Open,
    /// Browser windows / apps that can: full screen.
    pub fullscreen: bool,
}

pub const SOURCE_PREFIX: &str = "cloud:";

impl Entry {
    /// `cloud:<service>:<open>:<fs|win>:<nonce>`.
    pub fn source_id(&self, nonce: &str) -> String {
        format!(
            "{SOURCE_PREFIX}{}:{}:{}:{nonce}",
            self.service.key(),
            self.open.token(),
            if self.fullscreen { "fs" } else { "win" }
        )
    }

    pub fn parse(source_id: &str) -> Option<Entry> {
        let rest = source_id.strip_prefix(SOURCE_PREFIX)?;
        let mut it = rest.splitn(4, ':');
        let service = Service::from_key(it.next()?)?;
        let open = Open::from_token(it.next()?)?;
        let fullscreen = match it.next()? {
            "fs" => true,
            "win" => false,
            _ => return None,
        };
        Some(Entry { service, open, fullscreen })
    }

    /// The part of a cloud game's `source_id` that keeps it unique (kept when its settings
    /// change).
    pub fn nonce(source_id: &str) -> Option<&str> {
        source_id.strip_prefix(SOURCE_PREFIX)?.splitn(4, ':').nth(3)
    }

    /// The entry of a game, when it is a cloud game.
    pub fn of(g: &Game) -> Option<Entry> {
        if g.source != Source::Manual {
            return None;
        }
        Entry::parse(&g.source_id)
    }
}

impl Game {
    /// The name shown as where the game comes from: the cloud service for cloud games
    /// ("GeForce NOW"), else the source's name.
    pub fn source_name(&self) -> &'static str {
        match Entry::of(self) {
            Some(e) => e.service.name(),
            None => self.source.display(),
        }
    }

    /// A stable grouping key for `source_name` ("cloud-gfn" / the source key).
    pub fn source_group_key(&self) -> &'static str {
        match Entry::of(self) {
            Some(e) => e.service.group_key(),
            None => self.source.key(),
        }
    }

    /// Is this a cloud game (Add game → Cloud game…)?
    pub fn is_cloud(&self) -> bool {
        Entry::of(self).is_some()
    }
}

/// The page a cloud game opens: its stored link, else the service's home page.
pub fn game_url(g: &Game, e: &Entry) -> Option<String> {
    match &g.launch {
        Some(LaunchSpec::Url(u)) if is_web(u) => Some(u.trim().to_string()),
        _ => e.service.home().map(str::to_string),
    }
}

/// Is `url` the game's own page (not just the service's home page)?
pub fn is_game_link(service: Service, url: &str) -> bool {
    let norm = |s: &str| s.trim().trim_end_matches('/').to_ascii_lowercase();
    service.home().is_none_or(|h| norm(h) != norm(url))
}

pub fn is_web(u: &str) -> bool {
    let l = u.trim().to_ascii_lowercase();
    (l.starts_with("https://") || l.starts_with("http://")) && l.len() > 8 && !u.chars().any(char::is_control)
}

// ------------------------------------------------------------------------------- launching

/// How a plan opens the game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum How {
    App { name: &'static str, install: Install, procs: &'static [&'static str] },
    Window { name: String, family: Family, install: Install, procs: &'static [&'static str] },
    Tab,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub spec: LaunchSpec,
    pub how: How,
}

/// The command for `entry` opening `url` with what `found` has. `has_link`: `url` is the game's
/// own page (an app that can't open pages is then only used when chosen explicitly).
pub fn plan(entry: &Entry, url: &str, has_link: bool, found: &Found) -> Plan {
    let s = entry.service;
    let app = || found.apps_for(s).next();
    match &entry.open {
        Open::App => {
            if let Some(a) = app() {
                return app_plan(a, entry.fullscreen);
            }
        }
        Open::Browser(k) => {
            if let Some(b) = found.browser(k) {
                return window_plan(b, url, entry.fullscreen);
            }
        }
        Open::Tab => return tab_plan(url),
        Open::Auto => {}
    }
    // Automatic (or the chosen way isn't installed any more).
    if !has_link && let Some(a) = app() {
        return app_plan(a, entry.fullscreen);
    }
    match auto_browser(s, found) {
        Some(b) => window_plan(b, url, entry.fullscreen),
        None => tab_plan(url),
    }
}

/// The browser automatic picks: the default one when it is Chromium-based (or any browser for a
/// service that is fine with Firefox), else the first Chromium-based one, else any.
pub fn auto_browser(s: Service, found: &Found) -> Option<&FoundBrowser> {
    let ok = |b: &&FoundBrowser| s.browser_warning(b.family).is_none();
    let default = found.default_browser.as_deref().and_then(|k| found.browser(k));
    default
        .filter(ok)
        .or_else(|| found.browsers.iter().find(|b| b.family == Family::Chromium))
        .or_else(|| found.browsers.iter().find(ok))
        .or(default)
        .or_else(|| found.browsers.first())
}

fn cmd(install: &Install, mut args: Vec<String>) -> LaunchSpec {
    match install {
        Install::Flatpak(id) => {
            let mut a = vec!["run".to_string(), id.clone()];
            a.append(&mut args);
            LaunchSpec::Command { program: "flatpak".into(), args: a, cwd: None, env: vec![] }
        }
        Install::Native(p) => LaunchSpec::Command { program: p.display().to_string(), args, cwd: None, env: vec![] },
    }
}

fn app_plan(a: &FoundApp, fullscreen: bool) -> Plan {
    let args = a.fullscreen_arg.filter(|_| fullscreen).map(str::to_string).into_iter().collect();
    Plan { spec: cmd(&a.install, args), how: How::App { name: a.name, install: a.install.clone(), procs: a.procs } }
}

/// Browser arguments for a window of its own on `url`.
pub fn browser_args(family: Family, url: &str, fullscreen: bool) -> Vec<String> {
    match (family, fullscreen) {
        (Family::Chromium, true) => vec!["--start-fullscreen".into(), format!("--app={url}")],
        (Family::Chromium, false) => vec![format!("--app={url}")],
        (Family::Firefox, true) => vec!["--kiosk".into(), url.into()],
        (Family::Firefox, false) => vec!["--new-window".into(), url.into()],
    }
}

fn window_plan(b: &FoundBrowser, url: &str, fullscreen: bool) -> Plan {
    Plan {
        spec: cmd(&b.install, browser_args(b.family, url, fullscreen)),
        how: How::Window { name: b.name.clone(), family: b.family, install: b.install.clone(), procs: b.procs },
    }
}

fn tab_plan(url: &str) -> Plan {
    Plan { spec: LaunchSpec::Url(url.to_string()), how: How::Tab }
}

/// The plan for a cloud game with what is published (None: not a cloud game / no page).
pub fn plan_for(g: &Game) -> Option<(Entry, Plan)> {
    let e = Entry::of(g)?;
    let url = game_url(g, &e)?;
    let p = plan(&e, &url, is_game_link(e.service, &url), &published());
    Some((e, p))
}

/// The launch the launcher runs for a cloud game (None: not one).
pub fn launch_for(g: &Game) -> Option<LaunchSpec> {
    plan_for(g).map(|(_, p)| p.spec)
}

/// One line for the Add dialog: what opening the game does.
pub fn describe(service: Service, p: &Plan, has_link: bool) -> String {
    match &p.how {
        How::App { name, .. } if has_link => {
            format!("Starts the {name}; it can't open a game page directly, so pick the game there.")
        }
        How::App { name, .. } => format!("Starts the {name}; pick the game there."),
        How::Window { name, .. } if has_link => format!("Opens the game's page in a {name} window."),
        How::Window { name, .. } => {
            format!("Opens {} in a {name} window; pick the game there.", host_of(service))
        }
        How::Tab => "Opens the page in a new tab of your web browser (play time isn't counted for tabs).".into(),
    }
}

fn host_of(s: Service) -> String {
    s.home()
        .and_then(Url::parse)
        .map(|u| u.host.trim_start_matches("www.").to_string())
        .unwrap_or_else(|| s.name().to_string())
}

/// A warning for the Add dialog about the plan (a browser the service doesn't support).
pub fn warning(service: Service, p: &Plan, found: &Found) -> Option<&'static str> {
    let family = match &p.how {
        How::Window { family, .. } => Some(*family),
        How::Tab => found.default_browser.as_deref().and_then(|k| found.browser(k)).map(|b| b.family),
        How::App { .. } => None,
    }?;
    service.browser_warning(family)
}

/// Where processes and Flatpak instances are listed (the real machine, or fixtures).
pub struct RunEnv {
    pub proc_root: PathBuf,
    /// `$XDG_RUNTIME_DIR/.flatpak` (one folder per running Flatpak instance).
    pub flatpak_run: Option<PathBuf>,
}

impl RunEnv {
    pub fn system() -> RunEnv {
        RunEnv {
            proc_root: PathBuf::from("/proc"),
            flatpak_run: std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join(".flatpak")),
        }
    }
}

/// Is the app / browser of `how` running already (so a launch hands the page to it and ends)?
pub fn already_running(how: &How, env: &RunEnv) -> bool {
    let (install, procs) = match how {
        How::App { install, procs, .. } | How::Window { install, procs, .. } => (install, *procs),
        How::Tab => return false,
    };
    match install {
        Install::Flatpak(id) => flatpak_running(id, env),
        Install::Native(_) => native_running(procs, &env.proc_root),
    }
}

fn flatpak_running(id: &str, env: &RunEnv) -> bool {
    let Some(dir) = &env.flatpak_run else { return false };
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    rd.flatten().any(|e| {
        let d = e.path();
        let named =
            std::fs::read_to_string(d.join("info")).is_ok_and(|s| s.lines().any(|l| l.trim() == format!("name={id}")));
        let pid = std::fs::read_to_string(d.join("pid")).ok().and_then(|s| s.trim().parse::<i32>().ok());
        named && pid.is_some_and(|p| env.proc_root.join(p.to_string()).is_dir())
    })
}

fn native_running(procs: &[&str], proc_root: &Path) -> bool {
    if procs.is_empty() {
        return false;
    }
    crate::procfs::read_procs_in(proc_root, true).into_iter().any(|p| {
        let Some(a0) = p.cmdline.as_ref().and_then(|c| c.first()) else { return false };
        // Flatpak processes (`/app/…`) belong to the Flatpak build, not the native one.
        let name = Path::new(a0).file_name().and_then(|s| s.to_str()).unwrap_or("");
        !p.zombie && !a0.starts_with("/app/") && procs.contains(&name)
    })
}

/// Before a cloud game's Play launch: when the page goes to a browser tab, or to an app /
/// browser that already runs (it takes the page over and the started process ends at once),
/// the session isn't followed (`plan.tracking` → None). Returns what to tell the user then.
/// None for other games, custom commands and the user's own Play actions.
pub fn launch_note(g: &Game, action: Option<&GameAction>, lp: &mut crate::launch::LaunchPlan) -> Option<String> {
    launch_note_with(g, action, lp, &RunEnv::system())
}

pub fn launch_note_with(
    g: &Game,
    action: Option<&GameAction>,
    lp: &mut crate::launch::LaunchPlan,
    env: &RunEnv,
) -> Option<String> {
    let custom = g.launch_override.as_ref().and_then(|o| o.command.as_deref()).is_some_and(|c| !c.trim().is_empty());
    if custom || action.is_some_and(|a| !a.path.trim().is_empty() || !a.play) {
        return None;
    }
    if lp.tracking != TrackingMode::Default {
        return None;
    }
    let (_, p) = plan_for(g)?;
    let name = g.display_name();
    match &p.how {
        How::Tab => {
            lp.tracking = TrackingMode::None;
            Some(format!(
                "Opened {name} in your web browser. Lodestar can't see when you stop playing in a browser tab, so this session's play time isn't counted."
            ))
        }
        How::App { name: app, .. } if already_running(&p.how, env) => {
            lp.tracking = TrackingMode::None;
            Some(format!(
                "Opened {name}: the {app} was already open, so Lodestar can't tell when you stop playing and this session's play time isn't counted. Close the app before playing to have it counted."
            ))
        }
        How::Window { name: b, .. } if already_running(&p.how, env) => {
            lp.tracking = TrackingMode::None;
            Some(format!(
                "Opened {name} in {b}, which was already open, so Lodestar can't tell when you stop playing and this session's play time isn't counted. Close {b} before playing to have it counted."
            ))
        }
        _ => None,
    }
}

/// The ways the Add dialog offers (Automatic first), with their labels.
pub fn choices(service: Service, found: &Found) -> Vec<(Open, String)> {
    let mut v = vec![(Open::Auto, "Automatic".to_string())];
    if let Some(a) = found.apps_for(service).next() {
        v.push((Open::App, a.name.to_string()));
    }
    for b in &found.browsers {
        v.push((Open::Browser(b.key.clone()), format!("{} window", b.name)));
    }
    v.push((
        Open::Tab,
        match found.default_name() {
            Some(n) => format!("Default browser tab ({n})"),
            None => "Default browser tab".into(),
        },
    ));
    v
}

// ------------------------------------------------------------------------------- links → names

/// The bits of a web link Lodestar looks at.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Url {
    host: String,
    path: String,
    query: String,
    fragment: String,
}

impl Url {
    fn parse(u: &str) -> Option<Url> {
        let u = u.trim();
        let lower = u.to_ascii_lowercase();
        let rest = if lower.starts_with("https://") {
            &u[8..]
        } else if lower.starts_with("http://") {
            &u[7..]
        } else {
            return None;
        };
        let (rest, fragment) = rest.split_once('#').unwrap_or((rest, ""));
        let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
        let (host, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        let host = host.rsplit('@').next().unwrap_or(host);
        let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
        if host.is_empty() {
            return None;
        }
        Some(Url { host, path: path.to_string(), query: query.to_string(), fragment: fragment.to_string() })
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A path segment that is an id, not a name: digits, a 12-character product id (Xbox
/// `9NKX70BBCDRN`), an Amazon ASIN, a UUID, a hex hash.
fn is_id(seg: &str) -> bool {
    let s = seg.trim();
    if s.is_empty() || s.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    let alnum = s.chars().all(|c| c.is_ascii_alphanumeric());
    let has_digit = s.chars().any(|c| c.is_ascii_digit());
    let upper = s.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if alnum && upper && has_digit && (9..=16).contains(&s.len()) {
        return true;
    }
    let hexish = s.chars().all(|c| c.is_ascii_hexdigit() || c == '-');
    hexish && has_digit && s.len() >= 16
}

/// Words of path segments that never name a game.
const NOT_NAMES: &[&str] = &[
    "play",
    "games",
    "game",
    "launch",
    "store",
    "app",
    "apps",
    "mall",
    "en-us",
    "en-gb",
    "details",
    "product",
    "title",
    "titles",
    "library",
    "home",
    "deeplink",
    "application",
    "p",
    "dp",
];

fn is_locale(seg: &str) -> bool {
    let s = seg.to_ascii_lowercase();
    let b = s.as_bytes();
    (b.len() == 5
        && b[2] == b'-'
        && s[..2].chars().all(|c| c.is_ascii_lowercase())
        && s[3..].chars().all(|c| c.is_ascii_lowercase()))
        || (b.len() == 2 && s.chars().all(|c| c.is_ascii_lowercase()))
}

/// The game's name from a store / play link, where the link carries it:
/// `xbox.com/en-US/play/games/forza-horizon-5/9NKX70BBCDRN` → "Forza Horizon 5",
/// `store.steampowered.com/app/1091500/Cyberpunk_2077/` → "Cyberpunk 2077",
/// `luna.amazon.com/game/fortnite/B0…` → "Fortnite"; `?title=` / `name=` / `shortName=` too.
pub fn name_from_url(url: &str) -> Option<String> {
    let u = Url::parse(url)?;
    // Query (and hash-route query) parameters that carry a name.
    for q in [u.query.as_str(), u.fragment.split_once('?').map_or("", |(_, q)| q)] {
        for kv in q.split('&') {
            if let Some((k, v)) = kv.split_once('=')
                && matches!(k.to_ascii_lowercase().as_str(), "title" | "name" | "shortname" | "game-name" | "gamename")
            {
                let n = tidy_slug(&percent_decode(v));
                if !n.is_empty() && !is_id(v) {
                    return Some(n);
                }
            }
        }
    }
    let segs: Vec<String> = u.path.split('/').filter(|s| !s.is_empty()).map(percent_decode).collect();
    // A slug right after a marker word ("games", "launch", "game", "app/<id>" …).
    let mut best: Option<&String> = None;
    for (i, s) in segs.iter().enumerate() {
        let lower = s.to_ascii_lowercase();
        if NOT_NAMES.contains(&lower.as_str()) || is_locale(s) || is_id(s) {
            continue;
        }
        let after_marker = i > 0 && {
            let prev = segs[i - 1].to_ascii_lowercase();
            NOT_NAMES.contains(&prev.as_str()) || is_id(&segs[i - 1])
        };
        if after_marker || best.is_none() {
            best = Some(s);
            if after_marker {
                break;
            }
        }
    }
    let n = tidy_slug(best?);
    (!n.is_empty() && n.len() <= 80).then_some(n)
}

/// `forza-horizon-5` / `Cyberpunk_2077` / `the%20witcher` → title-cased words.
pub fn tidy_slug(s: &str) -> String {
    let s = s.trim().trim_end_matches(".html").trim_end_matches(".htm");
    let already_cased = s.chars().any(|c| c.is_ascii_uppercase());
    let words: Vec<&str> = s.split(['-', '_', '+', ' ']).filter(|w| !w.is_empty()).collect();
    const SMALL: &[&str] = &["of", "the", "and", "a", "an", "in", "on", "to", "for", "at", "by", "or", "vs"];
    words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let lower = w.to_ascii_lowercase();
            if already_cased {
                return w.to_string();
            }
            if is_roman(&lower) {
                return lower.to_ascii_uppercase();
            }
            if i > 0 && SMALL.contains(&lower.as_str()) {
                return lower;
            }
            let mut c = lower.chars();
            match c.next() {
                Some(f) => f.to_uppercase().chain(c).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_roman(w: &str) -> bool {
    matches!(w, "ii" | "iii" | "iv" | "vi" | "vii" | "viii" | "ix" | "xi" | "xii" | "xiii" | "xiv" | "xv" | "xvi")
}
