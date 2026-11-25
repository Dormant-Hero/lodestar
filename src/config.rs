//! User configuration (`~/.config/lodestar/config.toml`) and standard paths.
//!
//! CONTRACT: add fields with `#[serde(default)]` so old config files keep loading.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// Settings of one area each, in sibling files; their items are re-exported here.
mod controller;
mod desktop;
mod emulation;
mod menu_button;
pub use controller::*;
pub use desktop::*;
pub use emulation::*;
pub use menu_button::{MenuButton, MenuCombo, PadButton, SteamHome};

/// Settings → Audio: navigation sounds / music.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioMode {
    /// On when the controller theme brings its own, else off.
    #[default]
    Auto,
    On,
    Off,
}

impl AudioMode {
    /// Plays, given whether the theme has its own sounds / music.
    pub fn enabled(self, theme_has: bool) -> bool {
        match self {
            AudioMode::Auto => theme_has,
            AudioMode::On => true,
            AudioMode::Off => false,
        }
    }

    /// Settings: Automatic → On → Off → Automatic.
    pub fn next(self) -> AudioMode {
        match self {
            AudioMode::Auto => AudioMode::On,
            AudioMode::On => AudioMode::Off,
            AudioMode::Off => AudioMode::Auto,
        }
    }

    /// "Automatic (on)" / "On" / "Off".
    pub fn label(self, theme_has: bool) -> &'static str {
        match (self, theme_has) {
            (AudioMode::Auto, true) => "Automatic (on)",
            (AudioMode::Auto, false) => "Automatic (off)",
            (AudioMode::On, _) => "On",
            (AudioMode::Off, _) => "Off",
        }
    }
}

// 0.13.0 saved these as booleans, `false` by default: that becomes Automatic (nobody chose
// "off" on purpose then — it was the only default), `true` stays On.
impl<'de> Deserialize<'de> for AudioMode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Bool(bool),
            Text(String),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Bool(true) => AudioMode::On,
            Raw::Bool(false) => AudioMode::Auto,
            Raw::Text(t) => match t.to_ascii_lowercase().as_str() {
                "on" => AudioMode::On,
                "off" => AudioMode::Off,
                _ => AudioMode::Auto,
            },
        })
    }
}

pub const APP_ID: &str = "lodestar";

/// Standard locations. Override the root with `LODESTAR_HOME` (used by tests/screenshots):
/// then config/data/cache all live under that directory.
#[derive(Clone, Debug)]
pub struct Paths {
    pub home: PathBuf,
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl Paths {
    pub fn detect() -> Paths {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
        if let Some(root) = std::env::var_os("LODESTAR_HOME") {
            let root = PathBuf::from(root);
            return Paths {
                home,
                config_dir: root.join("config"),
                data_dir: root.join("data"),
                cache_dir: root.join("cache"),
            };
        }
        let config = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
        let data = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));
        let cache = dirs::cache_dir().unwrap_or_else(|| home.join(".cache"));
        Paths { config_dir: config.join(APP_ID), data_dir: data.join(APP_ID), cache_dir: cache.join(APP_ID), home }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }
    pub fn db_file(&self) -> PathBuf {
        self.data_dir.join("library.db")
    }
    /// User-installed and converted themes: `<themes_dir>/<name>/theme.toml`.
    pub fn themes_dir(&self) -> PathBuf {
        self.data_dir.join("themes")
    }
    /// Downloaded artwork (metadata fetcher).
    pub fn art_dir(&self) -> PathBuf {
        self.data_dir.join("art")
    }
    /// Decoded/downscaled thumbnails.
    pub fn thumbs_dir(&self) -> PathBuf {
        self.cache_dir.join("thumbs")
    }

    pub fn ensure(&self) {
        for d in
            [&self.config_dir, &self.data_dir, &self.cache_dir, &self.themes_dir(), &self.art_dir(), &self.thumbs_dir()]
        {
            let _ = std::fs::create_dir_all(d);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum StartMode {
    #[default]
    Desktop,
    Controller,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScanConfig {
    /// Scan automatically on startup.
    pub on_startup: bool,
    /// Sources to skip entirely (by `Source::key`).
    pub disabled_sources: Vec<String>,
    /// Extra Steam root directories to look in (besides the auto-detected native/Flatpak ones).
    pub extra_steam_roots: Vec<PathBuf>,
    /// Extra directories with `.desktop` files.
    pub extra_desktop_dirs: Vec<PathBuf>,
    /// Show Steam games that are owned but not installed (when the scanner can find them).
    pub include_uninstalled: bool,
    /// Desktop-entry names (case-insensitive substrings) to ignore, e.g. launchers themselves.
    pub desktop_ignore: Vec<String>,
    /// Extra Wine prefixes / folders holding prefixes to look in for store launchers and Windows games.
    pub extra_prefix_dirs: Vec<PathBuf>,
}

impl Default for ScanConfig {
    fn default() -> Self {
        ScanConfig {
            on_startup: true,
            disabled_sources: vec![],
            extra_steam_roots: vec![],
            extra_desktop_dirs: vec![],
            include_uninstalled: true,
            extra_prefix_dirs: vec![],
            desktop_ignore: vec![
                "settings".into(),
                "uninstall".into(),
                "launcher".into(),
                "steam".into(),
                "lutris".into(),
                "heroic".into(),
                "bottles".into(),
                "faugus".into(),
                "protonplus".into(),
                "protonup".into(),
                "winetricks".into(),
                "lsfg".into(),
                "lodestar".into(),
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetadataConfig {
    /// Download missing covers/descriptions from the Steam store for Steam games (no key needed).
    pub steam_store: bool,
    /// Optional SteamGridDB API key (https://www.steamgriddb.com/profile/preferences/api)
    /// for art of non-Steam games.
    pub steamgriddb_key: Option<String>,
    /// Artwork sources in priority order (names as `art::SourceKind::name`). Sources missing
    /// from the list follow in their built-in order; unknown names are dropped on load.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub source_order: Vec<String>,
    /// Artwork sources switched off in Settings → Metadata.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sources_off: Vec<String>,
    /// The artwork source order for emulated / console games (Settings → Metadata → Emulated
    /// games), like `source_order`; empty = the built-in order for consoles.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub emu_source_order: Vec<String>,
    /// Artwork sources switched off for emulated / console games. None = the built-in choice
    /// (the PC-only sources Steam, GOG, PCGamingWiki and Lutris.net off).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emu_sources_off: Option<Vec<String>>,
    /// IGDB (https://api-docs.igdb.com): the user's own Twitch application client id + secret;
    /// Lodestar trades them for an app access token (kept in memory until it expires).
    pub igdb_client_id: Option<String>,
    pub igdb_client_secret: Option<String>,
    /// Fill missing descriptions, genres, dates, companies, scores and series from IGDB
    /// (when the client id + secret are set). Never replaces what a game already has.
    pub igdb_metadata: bool,
    /// Fill missing descriptions from the English Wikipedia (no key; after the Steam store and
    /// IGDB). The game page credits the article (CC BY-SA).
    pub wikipedia: bool,
    /// Descriptions: Wikipedia's lead section first, even when the Steam store / IGDB has one.
    pub prefer_wikipedia: bool,
    /// TheGamesDB API key (https://forums.thegamesdb.net → API key request).
    pub thegamesdb_key: Option<String>,
}

impl Default for MetadataConfig {
    fn default() -> Self {
        MetadataConfig {
            steam_store: true,
            steamgriddb_key: None,
            source_order: vec![],
            sources_off: vec![],
            emu_source_order: vec![],
            emu_sources_off: None,
            igdb_client_id: None,
            igdb_client_secret: None,
            igdb_metadata: true,
            wikipedia: true,
            prefer_wikipedia: false,
            thegamesdb_key: None,
        }
    }
}

/// A trimmed, non-empty secret / id from the config.
pub fn secret(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

impl MetadataConfig {
    /// IGDB client id + secret, when both are set.
    pub fn igdb_credentials(&self) -> Option<(&str, &str)> {
        Some((secret(&self.igdb_client_id)?, secret(&self.igdb_client_secret)?))
    }
    /// Every secret value set (to scrub them from error texts and logs).
    pub fn secrets(&self) -> Vec<String> {
        [&self.steamgriddb_key, &self.igdb_client_secret, &self.thegamesdb_key]
            .into_iter()
            .filter_map(|v| secret(v).filter(|s| s.len() >= 4).map(str::to_string))
            .collect()
    }
    /// Drop artwork source names Lodestar doesn't have (a source that was removed, hand edits)
    /// from the order and on / off lists, so they aren't saved again.
    fn drop_unknown_sources(&mut self) {
        let known = |n: &String| crate::art::SourceKind::from_name(n).is_some();
        self.source_order.retain(known);
        self.sources_off.retain(known);
        self.emu_source_order.retain(known);
        if let Some(off) = &mut self.emu_sources_off {
            off.retain(known);
        }
    }
}

/// `text` with every occurrence of each secret replaced by `***`.
pub fn redact(text: &str, secrets: &[String]) -> String {
    let mut s = text.to_string();
    for k in secrets.iter().filter(|k| !k.is_empty()) {
        if s.contains(k.as_str()) {
            s = s.replace(k.as_str(), "***");
        }
    }
    s
}

/// External add-ons: which are enabled and their settings (values as strings; secrets too — the
/// config file is private to the user).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AddonsConfig {
    pub enabled: Vec<String>,
    pub settings: std::collections::BTreeMap<String, std::collections::BTreeMap<String, String>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub start_mode: StartMode,
    /// Theme name used in desktop mode (palette + fonts).
    pub desktop_theme: String,
    /// The desktop theme's colour scheme (None: its own colours).
    pub desktop_color_scheme: Option<String>,
    /// UI scale multiplier on top of the display scale.
    pub ui_scale: f32,
    /// Minimise the window while a game runs, restore when it exits.
    pub minimize_on_launch: bool,
    /// What closing the window does (the menu's Quit, Ctrl+Q and the tray's Quit always quit).
    /// None = automatic: hide to the tray while a game runs when there is a system tray,
    /// else minimise while a game runs (`WhenClosing::effective`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when_closing: Option<WhenClosing>,
    /// Before 0.4: what closing did while a game ran. Read once and turned into `when_closing`
    /// (`sanitized`), never written back.
    #[serde(skip_serializing)]
    pub close_while_playing: Option<CloseWhilePlaying>,
    /// Show Lodestar's icon in the system tray (StatusNotifierItem) when the desktop has one.
    pub tray_icon: bool,
    /// "Lodestar is in the system tray" was shown once already.
    pub tray_hint_shown: bool,
    /// Set a game's completion status to Playing when it is launched and was unset / not played /
    /// plan to play (Playnite does the same).
    pub auto_playing: bool,
    /// Global scripts run around every game session (per-game scripts can opt out).
    pub scripts: crate::model::GameScripts,
    pub scan: ScanConfig,
    pub desktop: DesktopConfig,
    pub controller: ControllerConfig,
    pub metadata: MetadataConfig,
    pub addons: AddonsConfig,
    pub art: ArtConfig,
    /// Store clients (Heroic, Battle.net, EA app …) around games, and one game at a time.
    pub launchers: LaunchersConfig,
    /// "Not a game" rules (Settings → Libraries → Hidden & excluded).
    pub exclude: ExcludeConfig,
    /// Global game options: the defaults every game's options fall back to.
    /// Lenient: one invalid option drops only itself (reported by [`Config::parse_lenient`]).
    #[serde(deserialize_with = "crate::game_options::deserialize_lenient")]
    pub game_options: crate::game_options::GameOptions,
    /// Per library (`Source::key()`): options over the global ones (Settings → Games → Applies to).
    /// Lenient per library and per option, like `game_options`.
    #[serde(
        default,
        skip_serializing_if = "std::collections::BTreeMap::is_empty",
        deserialize_with = "crate::game_options::deserialize_lenient_map"
    )]
    pub game_options_by_source: std::collections::BTreeMap<String, crate::game_options::GameOptions>,
    /// Emulation (Settings → Emulation).
    pub emulation: EmulationConfig,
    /// The user's RomM server (Settings → Libraries → RomM server).
    pub romm: RommConfig,
    /// Achievements on game pages (Settings → Achievements).
    pub achievements: AchievementsConfig,
    /// Ask GitHub once a day whether a newer Lodestar is out (Settings → General → Updates).
    pub check_updates: bool,
    /// The release the user chose to skip ("Skip this version"): automatic checks don't announce
    /// it; a newer one is announced again.
    pub skipped_update: Option<String>,
}

/// Achievements (achievements/mod.rs). Keys are kept like the other secrets (the config file is
/// private to the user); there is no RetroAchievements password anywhere. `Debug` never shows
/// the keys (a request carrying this config may be logged).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AchievementsConfig {
    /// Show the Achievements section on game pages (off: no achievements anywhere).
    pub show: bool,
    /// Show counts in the library too: on covers, as a List column and in controller themes'
    /// selected-game line ("31 / 54").
    pub library_counts: bool,
    /// Steam games' achievements (from the Steam client's files).
    pub steam: bool,
    /// Optional Steam Web API key (steamcommunity.com/dev/apikey) for games the Steam client has
    /// no files for yet.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steam_web_api_key: Option<String>,
    /// Emulated games' achievements from RetroAchievements.
    pub retroachievements: bool,
    /// RetroAchievements username ("" = not set up).
    pub ra_username: String,
    /// RetroAchievements Web API key (retroachievements.org → Settings → Keys).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ra_api_key: Option<String>,
    /// The account's permanent id (usernames can change), learned when the key is checked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ra_ulid: Option<String>,
}

impl Default for AchievementsConfig {
    fn default() -> Self {
        AchievementsConfig {
            show: true,
            library_counts: true,
            steam: true,
            steam_web_api_key: None,
            retroachievements: true,
            ra_username: String::new(),
            ra_api_key: None,
            ra_ulid: None,
        }
    }
}

impl AchievementsConfig {
    /// The RetroAchievements account progress is asked for: its permanent id once checked,
    /// else the username ("" = not set up).
    pub fn ra_user(&self) -> &str {
        self.ra_ulid.as_deref().map(str::trim).filter(|u| !u.is_empty()).unwrap_or(self.ra_username.trim())
    }

    /// RetroAchievements can be asked: switched on, with a username and a Web API key.
    pub fn ra_ready(&self) -> bool {
        self.show
            && self.retroachievements
            && !self.ra_username.trim().is_empty()
            && self.ra_api_key.as_deref().is_some_and(|k| !k.trim().is_empty())
    }

    /// Every secret value set (to scrub them from error texts and logs), also as it appears
    /// percent-encoded in a request URL.
    pub fn secrets(&self) -> Vec<String> {
        let mut out = Vec::new();
        for v in [&self.steam_web_api_key, &self.ra_api_key] {
            let Some(k) = v.as_deref().map(str::trim).filter(|s| s.len() >= 4) else { continue };
            let encoded = crate::art::net::encode(k);
            if encoded != k {
                out.push(encoded);
            }
            out.push(k.to_string());
        }
        out
    }
}

impl std::fmt::Debug for AchievementsConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let set = |v: &Option<String>| if secret(v).is_some() { "<set>" } else { "<none>" };
        f.debug_struct("AchievementsConfig")
            .field("show", &self.show)
            .field("library_counts", &self.library_counts)
            .field("steam", &self.steam)
            .field("steam_web_api_key", &set(&self.steam_web_api_key))
            .field("retroachievements", &self.retroachievements)
            .field("ra_username", &self.ra_username)
            .field("ra_api_key", &set(&self.ra_api_key))
            .field("ra_ulid", &self.ra_ulid)
            .finish()
    }
}

/// A RomM server (romm.rs): its address and how Lodestar signs in. Secrets are kept like the
/// other keys (the config file is private to the user).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RommConfig {
    /// "http://192.168.1.10:8080" ("" = not set up).
    pub url: String,
    /// Client API token ("rmm_…", RomM → Administration → Client API Tokens).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Older servers without client tokens: user name and password (HTTP Basic).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Where downloads go for consoles that have no ROM folder yet (asked once).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_dir: Option<PathBuf>,
    /// Accept a self-signed HTTPS certificate (home servers).
    pub accept_invalid_certs: bool,
}

/// Before 0.4: what closing the window did while a game Lodestar followed ran. Only read from
/// old config files (`Config::close_while_playing`); `WhenClosing` replaces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseWhilePlaying {
    Minimise,
    Quit,
    Ask,
}

/// What closing the window does (Settings → General → When closing Lodestar).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WhenClosing {
    /// Hide to the system tray while a game Lodestar follows runs (its session is recorded);
    /// quit otherwise.
    TrayWhilePlaying,
    /// Always hide to the tray; only Quit (menu, Ctrl+Q, the tray's menu) quits.
    TrayAlways,
    /// Minimise to the taskbar while a game runs; quit otherwise.
    MinimiseWhilePlaying,
    /// Quit; a running game keeps running and Lodestar picks it up again when it next starts.
    Quit,
    /// Ask while a game runs; quit otherwise.
    Ask,
}

impl WhenClosing {
    pub const ALL: [WhenClosing; 5] = [
        WhenClosing::TrayWhilePlaying,
        WhenClosing::TrayAlways,
        WhenClosing::MinimiseWhilePlaying,
        WhenClosing::Quit,
        WhenClosing::Ask,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WhenClosing::TrayWhilePlaying => "Hide to the tray while a game runs",
            WhenClosing::TrayAlways => "Always hide to the tray",
            WhenClosing::MinimiseWhilePlaying => "Minimise while a game runs",
            WhenClosing::Quit => "Quit",
            WhenClosing::Ask => "Ask",
        }
    }

    /// A short label (controller mode's settings list).
    pub fn short_label(self) -> &'static str {
        match self {
            WhenClosing::TrayWhilePlaying => "Tray while playing",
            WhenClosing::TrayAlways => "Always to the tray",
            WhenClosing::MinimiseWhilePlaying => "Minimise while playing",
            WhenClosing::Quit => "Quit",
            WhenClosing::Ask => "Ask",
        }
    }

    /// The setting in effect: the chosen one, else the tray while a game runs when there is a
    /// system tray (`tray`), else minimising while a game runs.
    pub fn effective(chosen: Option<WhenClosing>, tray: bool) -> WhenClosing {
        chosen.unwrap_or(if tray { WhenClosing::TrayWhilePlaying } else { WhenClosing::MinimiseWhilePlaying })
    }

    /// Uses the system tray.
    pub fn uses_tray(self) -> bool {
        matches!(self, WhenClosing::TrayWhilePlaying | WhenClosing::TrayAlways)
    }
}

/// Name rules that keep things that are not games out of the library (see exclude.rs).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExcludeConfig {
    /// The user's own rules, in the order they were added.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rules: Vec<ExcludeRule>,
    /// Built-in rules (`exclude::BUILTIN` ids) the user switched off. Built-ins are on unless
    /// listed here, so rules added in later versions reach existing configs.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub builtin_off: Vec<String>,
}

/// One user rule: a whole-name pattern with `*` wildcards (case-insensitive).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExcludeRule {
    pub pattern: String,
    /// Only games of this library (`Source::key`); None = every library.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub enabled: bool,
}

impl Default for ExcludeRule {
    fn default() -> Self {
        ExcludeRule { pattern: String::new(), source: None, enabled: true }
    }
}

/// What to do when a game is started while another game Lodestar launched still runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OneGameAtATime {
    /// Ask: "<A> is running. Close it and start <B>?"
    #[default]
    Ask,
    /// Close the running game (politely, then by force after a grace period) and start the new one.
    CloseRunning,
    /// Let several games run at once.
    AllowSeveral,
}

impl OneGameAtATime {
    pub const ALL: [OneGameAtATime; 3] =
        [OneGameAtATime::Ask, OneGameAtATime::CloseRunning, OneGameAtATime::AllowSeveral];
    pub fn label(self) -> &'static str {
        match self {
            OneGameAtATime::Ask => "Ask",
            OneGameAtATime::CloseRunning => "Close it automatically",
            OneGameAtATime::AllowSeveral => "Allow several",
        }
    }
}

/// Launcher hygiene (Settings → General → Launchers).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchersConfig {
    /// Minimise a store client's windows once the game it started is running (the game showing
    /// up means any login / update screen is done).
    pub hide_launchers: bool,
    /// Close store clients Lodestar itself started for a game when that game exits (never one
    /// that was already open).
    pub close_launchers_after: bool,
    /// Minimise the Steam client's windows while a Steam game runs.
    pub hide_steam: bool,
    /// Close Steam when its game ends, only a Steam Lodestar started (cold start, `steam -silent`)
    /// and only once it is idle (cloud sync done), through `steam -shutdown`. Off by default:
    /// Steam is slow to start again and many keep it running for Steam Input, the overlay, friends.
    pub close_steam_after: bool,
    /// Keep Steam running for controller support (Steam Input layouts need it): never closed,
    /// only minimised. Turning it on turns `close_steam_after` off and the other way round.
    pub keep_steam_for_controller: bool,
    pub one_game_at_a_time: OneGameAtATime,
}

impl LaunchersConfig {
    /// "Close Steam when the game ends" (on turns "keep Steam running" off).
    pub fn set_close_steam(&mut self, on: bool) {
        self.close_steam_after = on;
        if on {
            self.keep_steam_for_controller = false;
        }
    }

    /// "Keep Steam running for controller support" (on turns closing Steam off).
    pub fn set_keep_steam(&mut self, on: bool) {
        self.keep_steam_for_controller = on;
        if on {
            self.close_steam_after = false;
        }
    }
}

impl Default for LaunchersConfig {
    fn default() -> Self {
        LaunchersConfig {
            hide_launchers: true,
            close_launchers_after: true,
            hide_steam: true,
            close_steam_after: false,
            keep_steam_for_controller: false,
            one_game_at_a_time: OneGameAtATime::Ask,
        }
    }
}

/// Automatic artwork (Settings → Metadata).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArtConfig {
    /// After a scan, look up missing artwork of games that have never been looked up (Steam
    /// store name search; SteamGridDB too with a key). Confident matches only, once per game,
    /// never replacing art a game already has.
    pub auto_fill: bool,
}

impl Default for ArtConfig {
    fn default() -> Self {
        ArtConfig { auto_fill: true }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            start_mode: StartMode::Desktop,
            desktop_theme: "Lodestar".into(),
            desktop_color_scheme: None,
            ui_scale: 1.0,
            minimize_on_launch: true,
            when_closing: None,
            close_while_playing: None,
            tray_icon: true,
            tray_hint_shown: false,
            auto_playing: true,
            scripts: Default::default(),
            scan: ScanConfig::default(),
            desktop: DesktopConfig::default(),
            controller: ControllerConfig::default(),
            metadata: MetadataConfig::default(),
            addons: AddonsConfig::default(),
            art: ArtConfig::default(),
            launchers: LaunchersConfig::default(),
            exclude: ExcludeConfig::default(),
            game_options: Default::default(),
            game_options_by_source: Default::default(),
            emulation: EmulationConfig::default(),
            romm: RommConfig::default(),
            achievements: AchievementsConfig::default(),
            check_updates: true,
            skipped_update: None,
        }
    }
}

impl Config {
    /// Load, falling back to defaults on any error (a broken config must never stop the app).
    ///
    /// Loading is lenient per setting: a value with the wrong type or an unknown enum variant
    /// (hand edits, a config from a newer version) resets only that setting, not the whole file.
    /// When anything had to be dropped, the original file is copied to `config.toml.bak` first,
    /// because the next save would otherwise overwrite the user's settings for good.
    pub fn load(path: &Path) -> Config {
        let text = match std::fs::read(path) {
            Ok(b) => String::from_utf8_lossy(&b).into_owned(),
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("config {}: {e}; using defaults", path.display());
                    return Config::default();
                }
                // A first start: nothing from before to keep (see `emulator_dirs_checked`).
                let mut c = Config::default();
                c.emulation.emulator_dirs_checked = true;
                return c;
            }
        };
        let (cfg, problems) = Config::parse_lenient(&text);
        if !problems.is_empty() {
            let bak = path.with_extension("toml.bak");
            log::warn!(
                "config {}: {}; kept the rest, original saved as {}",
                path.display(),
                problems.join("; "),
                bak.display()
            );
            if let Err(e) = std::fs::copy(path, &bak) {
                log::warn!("could not back up {}: {e}", path.display());
            }
        }
        cfg
    }

    /// Parse config text, keeping every setting that is valid. Returns the config and a list of
    /// what was ignored (empty when the text was fully valid).
    pub fn parse_lenient(text: &str) -> (Config, Vec<String>) {
        let (cfg, mut problems) = Self::parse_settings(text);
        // The game options load leniently by themselves (one bad option drops only itself, see
        // `game_options::deserialize_lenient`); list what they dropped so the .bak is kept.
        problems.extend(game_option_problems(text));
        (cfg, problems)
    }

    /// [`Self::parse_lenient`] without the game-option report.
    fn parse_settings(text: &str) -> (Config, Vec<String>) {
        if let Ok(c) = toml::from_str::<Config>(text) {
            return (c.sanitized(), vec![]);
        }
        let user: toml::Table = match toml::from_str(text) {
            Ok(t) => t,
            Err(e) => {
                let msg = e.to_string();
                let first = msg.lines().next().unwrap_or("invalid TOML").to_string();
                return (Config::default(), vec![format!("not valid TOML ({first})")]);
            }
        };
        let mut base = match toml::Table::try_from(Config::default()) {
            Ok(t) => t,
            Err(_) => return (Config::default(), vec!["internal error".into()]),
        };
        let mut problems = vec![];
        // `base` is a valid config at every step; each user value is tried on a copy and kept
        // only if the whole config still deserializes.
        fn merge(root: &mut toml::Table, path: &mut Vec<String>, user: &toml::Table, problems: &mut Vec<String>) {
            for (k, v) in user {
                path.push(k.clone());
                let mut base_is_table = table_at(root, path).is_some();
                // A table the defaults leave out (`game_options_by_source`, `addons.settings.<id>`,
                // empty maps aren't serialized): start it empty when the config accepts that, then
                // merge into it per leaf, so one bad leaf doesn't drop its siblings.
                if let toml::Value::Table(_) = v
                    && !base_is_table
                    && !root_has(root, path)
                {
                    let mut trial = root.clone();
                    set_at(&mut trial, path, toml::Value::Table(toml::Table::new()));
                    if table_at(&mut trial, path).is_some() && trial.clone().try_into::<Config>().is_ok() {
                        *root = trial;
                        base_is_table = true;
                    }
                }
                match v {
                    toml::Value::Table(t) if base_is_table => merge(root, path, t, problems),
                    _ => {
                        let mut trial = root.clone();
                        set_at(&mut trial, path, v.clone());
                        if trial.clone().try_into::<Config>().is_ok() {
                            *root = trial;
                        } else {
                            problems.push(format!("ignored invalid `{}`", path.join(".")));
                        }
                    }
                }
                path.pop();
            }
        }
        fn table_at<'a>(root: &'a mut toml::Table, path: &[String]) -> Option<&'a mut toml::Table> {
            let mut t = root;
            for k in path {
                t = t.get_mut(k)?.as_table_mut()?;
            }
            Some(t)
        }
        /// Something (not necessarily a table) is at `path`.
        fn root_has(root: &mut toml::Table, path: &[String]) -> bool {
            let Some((last, parents)) = path.split_last() else {
                return true;
            };
            table_at(root, parents).is_some_and(|t| t.contains_key(last))
        }
        fn set_at(root: &mut toml::Table, path: &[String], v: toml::Value) {
            let Some((last, parents)) = path.split_last() else {
                return;
            };
            if let Some(t) = table_at(root, parents) {
                t.insert(last.clone(), v);
            }
        }
        merge(&mut base, &mut vec![], &user, &mut problems);
        let cfg = base.try_into::<Config>().unwrap_or_default();
        (cfg.sanitized(), problems)
    }

    /// Replace values that would break the UI (NaN / infinite / non-positive sizes) by defaults.
    /// Ranges are clamped where the values are used.
    fn sanitized(mut self) -> Config {
        let d = Config::default();
        // Before 0.4 closing while a game ran minimised by default (and every saved config has
        // that written out): that becomes "automatic" (the tray when there is one). A choice of
        // Quit or Ask is kept.
        if let Some(old) = self.close_while_playing.take()
            && self.when_closing.is_none()
        {
            self.when_closing = match old {
                CloseWhilePlaying::Minimise => None,
                CloseWhilePlaying::Quit => Some(WhenClosing::Quit),
                CloseWhilePlaying::Ask => Some(WhenClosing::Ask),
            };
        }
        self.controller.sanitize_menu_button();
        self.metadata.drop_unknown_sources();
        if !self.ui_scale.is_finite() || self.ui_scale <= 0.0 {
            self.ui_scale = d.ui_scale;
        }
        if !self.desktop.cover_width.is_finite() || self.desktop.cover_width <= 0.0 {
            self.desktop.cover_width = d.desktop.cover_width;
        }
        // Before 0.9 "Installed" was a toggle of its own; it is one of the quick presets now.
        if self.desktop.installed_only && self.desktop.preset == QuickPreset::All {
            self.desktop.preset = QuickPreset::Installed;
        }
        self.desktop.installed_only = self.desktop.preset == QuickPreset::Installed;
        self
    }

    /// Atomic save (write temp + fsync + rename), so a crash or full disk never leaves a
    /// truncated config behind.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let text = toml::to_string_pretty(self)?;
        crate::fsutil::write_atomic(path, text.as_bytes())?;
        // The rename itself reaches the disk only with the folder.
        if let Some(dir) = path.parent()
            && let Ok(d) = std::fs::File::open(dir)
        {
            let _ = d.sync_all();
        }
        Ok(())
    }

    pub fn source_enabled(&self, s: crate::model::Source) -> bool {
        !self.scan.disabled_sources.iter().any(|k| k == s.key())
    }
}

/// The game options `text` has that the lenient loader drops (`game_options` and every
/// `game_options_by_source.<library>`), as `parse_lenient` problems. Empty for invalid TOML
/// (reported elsewhere).
fn game_option_problems(text: &str) -> Vec<String> {
    use crate::game_options::{GameOptions, lenient_from_value};
    let Ok(user) = toml::from_str::<toml::Table>(text) else {
        return vec![];
    };
    let mut sections: Vec<(String, &toml::Value)> = vec![];
    if let Some(v) = user.get("game_options") {
        sections.push(("game_options".into(), v));
    }
    if let Some(toml::Value::Table(by)) = user.get("game_options_by_source") {
        sections.extend(by.iter().map(|(k, v)| (format!("game_options_by_source.{k}"), v)));
    }
    sections
        .into_iter()
        .filter_map(|(prefix, v)| Some((prefix, serde_json::to_value(v).ok()?)))
        .flat_map(|(prefix, v)| lenient_from_value::<GameOptions>(v, &prefix).1)
        .map(|d| format!("ignored invalid `{d}`"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_credentials_and_redaction() {
        let mut m = MetadataConfig::default();
        assert!(m.igdb_metadata && m.igdb_credentials().is_none());
        m.igdb_client_id = Some(" cid ".into());
        m.igdb_client_secret = Some("  ".into());
        assert!(m.igdb_credentials().is_none(), "blank secret");
        m.igdb_client_secret = Some("s3cr3t-value".into());
        assert_eq!(m.igdb_credentials(), Some(("cid", "s3cr3t-value")));
        m.thegamesdb_key = Some("pw-12345".into());
        let secrets = m.secrets();
        assert_eq!(redact("GET x?client_secret=s3cr3t-value&p=pw-12345", &secrets), "GET x?client_secret=***&p=***");
        // Round trip: empty lists aren't written, the rest is.
        let text = toml::to_string_pretty(&Config { metadata: m.clone(), ..Default::default() }).unwrap();
        assert!(!text.contains("source_order"));
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(back.metadata, m);
    }

    #[test]
    fn config_of_a_removed_source_still_loads() {
        // A config from 0.16.2 and before: ScreenScraper's credentials and its place in the
        // artwork source lists. Everything else is kept, the old source silently goes.
        let old = r#"
[metadata]
thegamesdb_key = "tgdb-key"
screenscraper_dev_id = "dev"
screenscraper_dev_password = "devpass"
screenscraper_user = "me"
screenscraper_password = "secret"
source_order = ["Steam", "ScreenScraper", "IGDB"]
sources_off = ["ScreenScraper", "Lutris"]
emu_source_order = ["libretro", "GameTDB", "ScreenScraper", "SteamGridDB"]
emu_sources_off = ["ScreenScraper", "Steam"]
"#;
        let (c, problems) = Config::parse_lenient(old);
        assert!(problems.is_empty(), "{problems:?}");
        let m = &c.metadata;
        assert_eq!(m.thegamesdb_key.as_deref(), Some("tgdb-key"));
        assert_eq!(m.source_order, ["Steam", "IGDB"]);
        assert_eq!(m.sources_off, ["Lutris"]);
        assert_eq!(m.emu_source_order, ["libretro", "GameTDB", "SteamGridDB"]);
        assert_eq!(m.emu_sources_off.as_deref(), Some(&["Steam".to_string()][..]));
        let rows = crate::art::sources::ordered_sources_for(m, crate::art::ArtList::Emulated);
        let names: Vec<&str> = rows.iter().map(|(k, _)| k.name()).collect();
        assert_eq!(&names[..4], ["libretro", "GameTDB", "SteamGridDB", "IGDB"], "no gap where it was");
        assert_eq!(rows.len(), crate::art::SourceKind::ALL.len());
        let saved = toml::to_string_pretty(&c).unwrap();
        assert!(!saved.to_lowercase().contains("screenscraper"), "{saved}");
    }

    #[test]
    fn roundtrip_and_partial() {
        let c = Config::default();
        let s = toml::to_string_pretty(&c).unwrap();
        let back: Config = toml::from_str(&s).unwrap();
        assert_eq!(c, back);
        let partial: Config = toml::from_str("start_mode = \"controller\"\n[desktop]\nview = \"list\"\n").unwrap();
        assert_eq!(partial.start_mode, StartMode::Controller);
        assert_eq!(partial.desktop.view, DesktopView::List);
        assert_eq!(partial.desktop.cover_width, 170.0);
    }

    #[test]
    fn button_prompts_setting() {
        use crate::input::ButtonStyle as S;
        // Old configs (no key) load as Automatic.
        let old: Config = toml::from_str("[controller]\nswap_ab = true\n").unwrap();
        assert_eq!(old.controller.button_prompts, ButtonPrompts::Auto);
        assert!(old.controller.swap_ab);
        for (p, word) in [
            (ButtonPrompts::Auto, "auto"),
            (ButtonPrompts::Xbox, "xbox"),
            (ButtonPrompts::PlayStation, "playstation"),
            (ButtonPrompts::Nintendo, "nintendo"),
        ] {
            let mut c = Config::default();
            c.controller.button_prompts = p;
            let text = toml::to_string_pretty(&c).unwrap();
            assert!(text.contains(&format!("button_prompts = \"{word}\"")), "{text}");
            let back: Config = toml::from_str(&text).unwrap();
            assert_eq!(back.controller.button_prompts, p);
        }
        // A manual choice wins over the detected pad; Automatic follows it, Xbox without one.
        assert_eq!(ButtonPrompts::Nintendo.resolve(|| Some(S::PlayStation)), S::Nintendo);
        assert_eq!(ButtonPrompts::Xbox.resolve(|| Some(S::PlayStation)), S::Xbox);
        assert_eq!(ButtonPrompts::Auto.resolve(|| Some(S::PlayStation)), S::PlayStation);
        assert_eq!(ButtonPrompts::Auto.resolve(|| None), S::Xbox);
        // Cycling visits every choice.
        let mut p = ButtonPrompts::Auto;
        for want in [ButtonPrompts::Xbox, ButtonPrompts::PlayStation, ButtonPrompts::Nintendo, ButtonPrompts::Auto] {
            p = p.next();
            assert_eq!(p, want);
        }
    }

    #[test]
    fn emulation_defaults_and_lenient_rom_folders() {
        let e = EmulationConfig::default();
        assert!(e.auto_emulators && e.auto_folders && e.fullscreen);
        assert!(RomFolder::default().enabled);
        // A missing section and missing fields use the defaults.
        let c: Config = toml::from_str("[emulation.rom_folders.f1]\npath = \"/roms\"\n").unwrap();
        assert!(c.emulation.fullscreen && c.emulation.rom_folders["f1"].enabled);
        // One broken folder entry (and one bad field of another) drops only that.
        let text = r#"
[emulation]
fullscreen = false
[emulation.rom_folders]
f2 = "not a table"
[emulation.rom_folders.f1]
path = "/mnt/games/ps2"
platform = "sony_playstation2"
[emulation.rom_folders.f3]
path = "/mnt/games"
enabled = "yes"
[emulation.platform_emulator]
sony_playstation2 = "pcsx2/default"
"#;
        let (c, problems) = Config::parse_lenient(text);
        assert!(!problems.is_empty());
        let e = &c.emulation;
        assert!(!e.fullscreen);
        assert_eq!(e.rom_folders.len(), 2, "{:?}", e.rom_folders);
        assert_eq!(e.rom_folders["f1"].path, PathBuf::from("/mnt/games/ps2"));
        assert_eq!(e.rom_folders["f1"].platform, "sony_playstation2");
        assert_eq!(e.rom_folders["f3"].path, PathBuf::from("/mnt/games"));
        assert!(e.rom_folders["f3"].enabled);
        assert_eq!(e.platform_emulator["sony_playstation2"], "pcsx2/default");
        // And it round-trips.
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back.emulation, c.emulation);
    }

    #[test]
    fn one_bad_value_does_not_reset_everything() {
        let text = r#"
start_mode = "controller"
ui_scale = "huge"
future_setting = 3

[desktop]
view = "carousel"
cover_width = 220.0
sort = "playtime"

[metadata]
steamgriddb_key = "abc123"

[scan]
disabled_sources = ["itch"]
include_uninstalled = "yes"
"#;
        let (c, problems) = Config::parse_lenient(text);
        assert_eq!(c.start_mode, StartMode::Controller);
        assert_eq!(c.ui_scale, 1.0);
        assert_eq!(c.desktop.view, DesktopView::Grid);
        assert_eq!(c.desktop.cover_width, 220.0);
        assert_eq!(c.desktop.sort, SortBy::Playtime);
        assert_eq!(c.metadata.steamgriddb_key.as_deref(), Some("abc123"));
        assert_eq!(c.scan.disabled_sources, vec!["itch".to_string()]);
        // The bad value falls back to the default, whatever that is.
        assert_eq!(c.scan.include_uninstalled, ScanConfig::default().include_uninstalled);
        assert_eq!(problems.len(), 3, "{problems:?}");
        // Old configs with a whole section of the wrong shape.
        let (c, p) = Config::parse_lenient(
            "desktop = 5
ui_scale = 1.5
",
        );
        assert_eq!((c.ui_scale, c.desktop.cover_width, p.len()), (1.5, 170.0, 1));
        // NaN would poison the zoom factor.
        let (c, _) = Config::parse_lenient(
            "ui_scale = nan
[desktop]
cover_width = -inf
",
        );
        assert_eq!((c.ui_scale, c.desktop.cover_width), (1.0, 170.0));
    }

    #[test]
    fn one_bad_game_option_drops_only_itself() {
        use crate::game_options::{FlagSetting, Tri};
        let text = r#"
[game_options]
hdr = "on"
gs_filter = "bicubic"

[game_options.gs_more]
"--rt" = "on"
"--hdr-itm-enabled" = 5

[game_options_by_source.steam]
gs_sharpness = 300
mangohud = "on"

[game_options_by_source.heroic]
gs_sharpness = 5
"#;
        let (c, problems) = Config::parse_lenient(text);
        assert_eq!(c.game_options.hdr, Tri::On);
        assert_eq!(c.game_options.gs_filter, None);
        assert_eq!(c.game_options.gs_more.get("--rt"), Some(&FlagSetting::On));
        assert!(!c.game_options.gs_more.contains_key("--hdr-itm-enabled"));
        let steam = &c.game_options_by_source["steam"];
        assert_eq!((steam.mangohud, steam.gs_sharpness), (Tri::On, None));
        assert_eq!(c.game_options_by_source["heroic"].gs_sharpness, Some(5), "other libraries survive");
        // Reported, so Config::load keeps a .bak of the original.
        let joined = problems.join("\n");
        assert_eq!(problems.len(), 3, "{joined}");
        for p in [
            "game_options.gs_filter",
            "game_options.gs_more.--hdr-itm-enabled",
            "game_options_by_source.steam.gs_sharpness",
        ] {
            assert!(joined.contains(p), "{p}: {joined}");
        }
        // The strict loader agrees (it is what parse_lenient tries first).
        let strict: Config = toml::from_str(text).unwrap();
        assert_eq!(strict, c);
        // With another bad setting (the per-leaf merge path), each problem is listed once.
        let (c, p) = Config::parse_lenient(&format!("ui_scale = \"big\"\n{text}"));
        assert_eq!((c.ui_scale, c.game_options.hdr), (1.0, Tri::On));
        assert_eq!(c.game_options_by_source["heroic"].gs_sharpness, Some(5));
        assert_eq!(p.len(), 4, "{p:?}");
        // A non-table where a library's options belong drops only that library.
        let (c, p) = Config::parse_lenient(
            "[game_options_by_source]\nsteam = 5\n[game_options_by_source.heroic]\nmangohud = \"on\"\n",
        );
        assert_eq!(c.game_options_by_source["heroic"].mangohud, Tri::On);
        assert_eq!(p.len(), 1, "{p:?}");
        // A valid config reports nothing.
        let (_, p) =
            Config::parse_lenient("[game_options]\nhdr = \"on\"\n[game_options_by_source.steam]\nmangohud = \"off\"\n");
        assert!(p.is_empty(), "{p:?}");
    }

    #[test]
    fn tables_missing_from_the_defaults_merge_per_leaf() {
        let (c, p) = Config::parse_lenient("[addons.settings.foo]\nkey = \"x\"\nbad = 5\n");
        assert_eq!(c.addons.settings["foo"].get("key").map(String::as_str), Some("x"));
        assert!(!c.addons.settings["foo"].contains_key("bad"));
        assert_eq!(p, vec!["ignored invalid `addons.settings.foo.bad`".to_string()]);
        // A table where a non-table belongs is still rejected whole.
        let (c, p) = Config::parse_lenient("ui_scale = 1.5\n[scan.disabled_sources]\na = 1\n");
        assert_eq!(c.ui_scale, 1.5);
        assert_eq!(p.len(), 1, "{p:?}");
    }

    #[test]
    fn launcher_settings_default_and_parse() {
        let c = Config::default();
        assert!(c.launchers.hide_launchers && c.launchers.close_launchers_after);
        assert_eq!(c.launchers.one_game_at_a_time, OneGameAtATime::Ask);
        // Old configs without the section keep the defaults.
        let (c, p) = Config::parse_lenient("ui_scale = 1.25\n");
        assert_eq!((c.launchers, p.len()), (LaunchersConfig::default(), 0));
        let (c, p) =
            Config::parse_lenient("[launchers]\nhide_launchers = false\none_game_at_a_time = \"close_running\"\n");
        assert!(p.is_empty(), "{p:?}");
        assert!(!c.launchers.hide_launchers && c.launchers.close_launchers_after);
        assert_eq!(c.launchers.one_game_at_a_time, OneGameAtATime::CloseRunning);
        let (c, p) = Config::parse_lenient("[launchers]\none_game_at_a_time = \"sometimes\"\n");
        assert_eq!((c.launchers.one_game_at_a_time, p.len()), (OneGameAtATime::Ask, 1));
        let s = toml::to_string_pretty(&Config::default()).unwrap();
        assert!(s.contains("one_game_at_a_time = \"ask\""), "{s}");
    }

    #[test]
    fn when_closing_defaults_and_migrates_the_old_setting() {
        let d = Config::default();
        assert_eq!((d.when_closing, d.tray_icon, d.tray_hint_shown), (None, true, false));
        assert_eq!(WhenClosing::effective(None, true), WhenClosing::TrayWhilePlaying);
        assert_eq!(WhenClosing::effective(None, false), WhenClosing::MinimiseWhilePlaying);
        assert_eq!(WhenClosing::effective(Some(WhenClosing::Ask), true), WhenClosing::Ask);
        // Older configs don't have it.
        let (c, p) = Config::parse_lenient("ui_scale = 1.25\n");
        assert_eq!((c.when_closing, c.tray_icon, p.len()), (None, true, 0));
        // 0.3's setting: minimise (the old default) becomes automatic; quit and ask stay.
        for (old, new) in [("minimise", None), ("quit", Some(WhenClosing::Quit)), ("ask", Some(WhenClosing::Ask))] {
            let (c, p) = Config::parse_lenient(&format!("close_while_playing = \"{old}\"\n"));
            assert_eq!((c.when_closing, c.close_while_playing, p.len()), (new, None, 0), "{old}");
        }
        // A new value wins over an old one left in the file.
        let (c, _) = Config::parse_lenient("close_while_playing = \"quit\"\nwhen_closing = \"tray_always\"\n");
        assert_eq!(c.when_closing, Some(WhenClosing::TrayAlways));
        let (c, p) = Config::parse_lenient("when_closing = \"explode\"\n");
        assert_eq!((c.when_closing, p.len()), (None, 1));
        let (c, p) = Config::parse_lenient("close_while_playing = \"explode\"\nui_scale = 2.0\n");
        assert_eq!((c.when_closing, c.ui_scale, p.len()), (None, 2.0, 1));
        // Saved: the new key only; automatic isn't written at all.
        let s = toml::to_string_pretty(&Config { when_closing: Some(WhenClosing::MinimiseWhilePlaying), ..c }).unwrap();
        assert!(s.contains("when_closing = \"minimise_while_playing\"") && !s.contains("close_while_playing"), "{s}");
        assert_eq!(Config::parse_lenient(&s).0.when_closing, Some(WhenClosing::MinimiseWhilePlaying));
        let s = toml::to_string_pretty(&Config::default()).unwrap();
        assert!(!s.contains("when_closing") && s.contains("tray_icon = true"), "{s}");
    }

    #[test]
    fn broken_file_is_backed_up_and_save_is_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "start_mode = \"controller\"\n[desktop\nview = ").unwrap();
        let c = Config::load(&path);
        assert_eq!(c, Config::default());
        let bak = dir.path().join("config.toml.bak");
        assert!(std::fs::read_to_string(&bak).unwrap().contains("[desktop"));
        c.save(&path).unwrap();
        assert!(!dir.path().join("config.toml.tmp").exists());
        assert_eq!(Config::load(&path), c);
        // A valid file is never backed up.
        std::fs::remove_file(&bak).unwrap();
        let _ = Config::load(&path);
        assert!(!bak.exists());
        // Missing file: defaults, no backup; a first start has nothing from before to check.
        let mut first = Config::default();
        first.emulation.emulator_dirs_checked = true;
        assert_eq!(Config::load(&dir.path().join("nope.toml")), first);
        // Configs from before don't have the flag: they get the one-time check.
        assert!(!Config::parse_lenient("[emulation]\nauto_emulators = false\n").0.emulation.emulator_dirs_checked);
    }

    #[test]
    fn exclude_rules_roundtrip_and_old_configs_load() {
        // A config from before 4e has no [exclude]: no user rules, every built-in on.
        let (c, p) = Config::parse_lenient("start_mode = \"desktop\"\n[scan]\non_startup = false\n");
        assert!(p.is_empty());
        assert_eq!(c.exclude, ExcludeConfig::default());
        let mut c = Config::default();
        c.exclude.rules.push(ExcludeRule { pattern: "Proton *".into(), source: Some("lutris".into()), enabled: false });
        c.exclude.rules.push(ExcludeRule { pattern: "*Redist*".into(), ..Default::default() });
        c.exclude.builtin_off.push("setup".into());
        let s = toml::to_string_pretty(&c).unwrap();
        let (back, p) = Config::parse_lenient(&s);
        assert!(p.is_empty(), "{p:?}");
        assert_eq!(back.exclude, c.exclude);
        // A rule without `enabled` is on.
        let (c, _) = Config::parse_lenient("[[exclude.rules]]\npattern = \"Foo*\"\n");
        assert_eq!(c.exclude.rules, vec![ExcludeRule { pattern: "Foo*".into(), source: None, enabled: true }]);
    }

    /// Settings that no longer exist (the controller's "extra lists" switch) are ignored without
    /// resetting anything else or asking for a backup.
    #[test]
    fn removed_settings_still_load() {
        let (c, problems) = Config::parse_lenient("[controller]\nextra_lists = \"hide\"\nswap_ab = true\n");
        assert!(problems.is_empty(), "{problems:?}");
        assert!(c.controller.swap_ab);
    }
}
