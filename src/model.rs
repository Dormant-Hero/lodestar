//! Core data types shared by every module.
//!
//! CONTRACT: other modules depend on these types. Add fields/variants freely, but do not
//! rename or remove existing ones without updating every user.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

mod crop;
pub use crop::*;

pub type GameId = i64;

/// Where a game came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Source {
    Steam,
    Faugus,
    Heroic,
    Lutris,
    Bottles,
    Itch,
    /// `.desktop` entries with `Categories=Game` (native games, Flatpaks, AppImages …)
    Desktop,
    /// Added by hand in the UI.
    Manual,
    /// Epic Games Store (via Heroic / Legendary).
    Epic,
    /// GOG (via Heroic; gogdl keeps no library of its own).
    Gog,
    /// Amazon Games / Prime Gaming (via Heroic / nile).
    Amazon,
    /// Battle.net games installed in a Wine prefix.
    BattleNet,
    /// EA app games installed in a Wine prefix.
    Ea,
    /// Ubisoft Connect games installed in a Wine prefix.
    Ubisoft,
    /// Rockstar Games Launcher games installed in a Wine prefix.
    Rockstar,
    /// Riot Client games installed in a Wine prefix.
    Riot,
    /// HoYoPlay games installed in a Wine prefix.
    HoYoPlay,
    /// Minecraft instances (Prism Launcher, MultiMC, ATLauncher).
    Minecraft,
    /// Apps of paired Moonlight / Sunshine / Apollo streaming hosts.
    Moonlight,
    /// Games from an external add-on (`source_id` = `<addon id>:<game id>`).
    Addon,
    /// Emulated games: ROMs found in ROM folders, launched through an emulator
    /// (`source_id` = `<ROM folder id>/<path relative to that folder>`).
    Emulator,
    /// Games on the user's RomM server (`source_id` = RomM's ROM id); downloaded ones become
    /// emulated games (romm.rs).
    Romm,
}

impl Source {
    /// Store sources whose ids are permanent store ids and that can list owned-but-not-installed games.
    pub fn is_store(self) -> bool {
        matches!(self, Source::Steam | Source::Epic | Source::Gog | Source::Amazon | Source::Itch)
    }

    /// A store game the user owns (Steam app, Epic / GOG / Amazon / itch.io game, a Wine
    /// store launcher's game): when it drops out of a scan it was uninstalled, it never goes
    /// missing. Steam's non-Steam shortcuts are not owned (deleting one in Steam removes it).
    pub fn owned_entry(self, source_id: &str) -> bool {
        match self {
            Source::Steam => !source_id.starts_with("shortcut:"),
            Source::Epic | Source::Gog | Source::Amazon | Source::Itch => true,
            Source::BattleNet | Source::Ea | Source::Ubisoft | Source::Rockstar | Source::Riot | Source::HoYoPlay => {
                true
            }
            _ => false,
        }
    }

    /// The hint on a missing game's page: "No longer in Steam".
    pub fn missing_text(self) -> String {
        match self {
            Source::Desktop => "No longer in the app menu".into(),
            Source::Emulator => "No longer in its ROM folder".into(),
            Source::Addon => "No longer listed by its add-on".into(),
            Source::Moonlight => "No longer on the streaming host".into(),
            Source::Romm => "No longer on the RomM server".into(),
            Source::Minecraft => "No longer in the Minecraft launcher".into(),
            s => format!("No longer in {}", s.display()),
        }
    }

    pub const ALL: [Source; 22] = [
        Source::Steam,
        Source::Faugus,
        Source::Heroic,
        Source::Lutris,
        Source::Bottles,
        Source::Itch,
        Source::Desktop,
        Source::Manual,
        Source::Epic,
        Source::Gog,
        Source::Amazon,
        Source::BattleNet,
        Source::Ea,
        Source::Ubisoft,
        Source::Rockstar,
        Source::Riot,
        Source::HoYoPlay,
        Source::Minecraft,
        Source::Moonlight,
        Source::Addon,
        Source::Emulator,
        Source::Romm,
    ];

    /// Stable key used in the database and config. Never change these strings.
    pub fn key(self) -> &'static str {
        match self {
            Source::Steam => "steam",
            Source::Faugus => "faugus",
            Source::Heroic => "heroic",
            Source::Lutris => "lutris",
            Source::Bottles => "bottles",
            Source::Itch => "itch",
            Source::Desktop => "desktop",
            Source::Manual => "manual",
            Source::Epic => "epic",
            Source::Gog => "gog",
            Source::Amazon => "amazon",
            Source::BattleNet => "battlenet",
            Source::Ea => "ea",
            Source::Ubisoft => "ubisoft",
            Source::Rockstar => "rockstar",
            Source::Riot => "riot",
            Source::HoYoPlay => "hoyoplay",
            Source::Minecraft => "minecraft",
            Source::Moonlight => "moonlight",
            Source::Addon => "addon",
            Source::Emulator => "emulator",
            Source::Romm => "romm",
        }
    }

    pub fn from_key(s: &str) -> Option<Source> {
        Source::ALL.into_iter().find(|src| src.key() == s)
    }

    pub fn display(self) -> &'static str {
        match self {
            Source::Steam => "Steam",
            Source::Faugus => "Faugus",
            Source::Heroic => "Heroic",
            Source::Lutris => "Lutris",
            Source::Bottles => "Bottles",
            Source::Itch => "itch.io",
            Source::Desktop => "Desktop",
            Source::Manual => "Manual",
            Source::Epic => "Epic Games",
            Source::Gog => "GOG",
            Source::Amazon => "Amazon Games",
            Source::BattleNet => "Battle.net",
            Source::Ea => "EA app",
            Source::Ubisoft => "Ubisoft Connect",
            Source::Rockstar => "Rockstar",
            Source::Riot => "Riot Games",
            Source::HoYoPlay => "HoYoPlay",
            Source::Minecraft => "Minecraft",
            Source::Moonlight => "Moonlight",
            Source::Addon => "Add-on",
            Source::Emulator => "Emulators",
            Source::Romm => "RomM",
        }
    }
}

/// How to start a game.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchSpec {
    /// Run a program directly (no shell). `program` is looked up in PATH if not absolute.
    Command {
        program: String,
        args: Vec<String>,
        #[serde(default)]
        cwd: Option<PathBuf>,
        #[serde(default)]
        env: Vec<(String, String)>,
    },
    /// Open a URL / URI with the system handler (xdg-open), e.g. `heroic://launch/...`.
    Url(String),
    /// Steam game: launched through Steam (`steam://rungameid/<appid>`), tracked via the reaper process.
    Steam { appid: u32 },
}

/// Per-game launch customisation set by the user in Lodestar (never written back to Steam etc.).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LaunchOverride {
    /// Steam-style launch options, e.g. `MANGOHUD=1 gamemoderun %command% -dx11`.
    /// `%command%` expands to the base command. Without `%command%` the text is appended as
    /// extra arguments. Leading `VAR=value` words become environment variables.
    pub options: String,
    /// Replace the source's launch command entirely (shell-like quoting, no shell is run).
    /// `options` are then applied around this command.
    pub command: Option<String>,
    /// Working directory for `command`.
    pub cwd: Option<PathBuf>,
}

impl LaunchOverride {
    pub fn is_empty(&self) -> bool {
        self.options.trim().is_empty()
            && self.command.as_deref().is_none_or(|c| c.trim().is_empty())
            && self.cwd.is_none()
    }
}

/// Local artwork files. All optional; paths are absolute.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Art {
    /// Portrait box art (~2:3), e.g. Steam `library_600x900.jpg`.
    pub cover: Option<PathBuf>,
    /// Wide background / hero image, e.g. Steam `library_hero.jpg`.
    pub hero: Option<PathBuf>,
    /// Transparent title logo, e.g. Steam `logo.png`.
    pub logo: Option<PathBuf>,
    /// Small square icon.
    pub icon: Option<PathBuf>,
    /// Wide capsule/header (~460x215), e.g. Steam `header.jpg`.
    pub banner: Option<PathBuf>,
}

impl Art {
    /// Fill any empty slot from `other`.
    pub fn fill_from(&mut self, other: &Art) {
        fn f(a: &mut Option<PathBuf>, b: &Option<PathBuf>) {
            if a.is_none() {
                a.clone_from(b);
            }
        }
        f(&mut self.cover, &other.cover);
        f(&mut self.hero, &other.hero);
        f(&mut self.logo, &other.logo);
        f(&mut self.icon, &other.icon);
        f(&mut self.banner, &other.banner);
    }

    /// The picture behind the game's page: the background, else the banner, else the cover
    /// (what a `background_placement` crops).
    pub fn background(&self) -> Option<&PathBuf> {
        self.hero.as_ref().or(self.banner.as_ref()).or(self.cover.as_ref())
    }

    pub fn is_empty(&self) -> bool {
        self.cover.is_none()
            && self.hero.is_none()
            && self.logo.is_none()
            && self.icon.is_none()
            && self.banner.is_none()
    }
}

/// How well a game plays with a controller (Steam's own classification).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ControllerSupport {
    Full,
    Partial,
    /// Explicitly no controller support (only ever set by the user).
    None,
}

impl ControllerSupport {
    pub fn label(self) -> &'static str {
        match self {
            ControllerSupport::Full => "Full controller support",
            ControllerSupport::Partial => "Partial controller support",
            ControllerSupport::None => "No controller support",
        }
    }
    /// Playable on the couch (full or partial support).
    pub fn is_playable(self) -> bool {
        matches!(self, ControllerSupport::Full | ControllerSupport::Partial)
    }
}

/// Steam Deck compatibility rating (a good hint for how well a game runs on Linux).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeckRating {
    Verified,
    Playable,
    Unsupported,
}

impl DeckRating {
    pub fn label(self) -> &'static str {
        match self {
            DeckRating::Verified => "Steam Deck Verified",
            DeckRating::Playable => "Steam Deck Playable",
            DeckRating::Unsupported => "Steam Deck Unsupported",
        }
    }
}

/// Where the player is with a game (user-set; shown, filtered and grouped on).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionStatus {
    NotPlayed,
    PlanToPlay,
    Playing,
    OnHold,
    Beaten,
    Completed,
    Abandoned,
}

impl CompletionStatus {
    pub const ALL: [CompletionStatus; 7] = [
        CompletionStatus::NotPlayed,
        CompletionStatus::PlanToPlay,
        CompletionStatus::Playing,
        CompletionStatus::OnHold,
        CompletionStatus::Beaten,
        CompletionStatus::Completed,
        CompletionStatus::Abandoned,
    ];
    pub fn label(self) -> &'static str {
        match self {
            CompletionStatus::NotPlayed => "Not played",
            CompletionStatus::PlanToPlay => "Plan to play",
            CompletionStatus::Playing => "Playing",
            CompletionStatus::OnHold => "On hold",
            CompletionStatus::Beaten => "Beaten",
            CompletionStatus::Completed => "Completed",
            CompletionStatus::Abandoned => "Abandoned",
        }
    }
}

/// A named web link of a game (store page, wiki, ProtonDB …).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameLink {
    pub name: String,
    pub url: String,
}

/// How Lodestar decides a game launched by an action is still running.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrackingMode {
    /// The launched process tree / session / launch tag (Steam: its reaper).
    #[default]
    Default,
    /// Only the started process itself.
    Process,
    /// Any process whose executable or working directory is inside the game's install folder
    /// (launchers that hand off to the real game).
    Folder,
    /// Don't track (no playtime).
    None,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Run a program (`path` + `args`, shell-like quoting, no shell).
    #[default]
    Command,
    /// Open a URL / URI with the system handler.
    Url,
    /// Open a file with its default application.
    File,
    /// Emulated games: start the game's ROM with an emulator + profile (Playnite's emulator
    /// action): `args` are extra arguments placed by the profile's template, or the whole
    /// argument list with `replace_args`.
    Emulator,
}

/// A user-defined way to start something for a game ("Play", "Settings", "Launcher" …).
/// `path`, `args` and `cwd` may use `{InstallDir}`, `{Name}`, `{GameId}`, `{Prefix}`, `{Proton}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameAction {
    pub name: String,
    pub kind: ActionKind,
    pub path: String,
    pub args: String,
    pub cwd: Option<String>,
    pub tracking: TrackingMode,
    /// The Play button runs this action (instead of the source's own launch). With several
    /// Play actions, Play asks which one ("Choose what to play"; the first is the default).
    pub play: bool,
    /// Emulator actions: the emulator instance id (None = the game's emulator, automatic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emulator: Option<String>,
    /// Emulator actions: the profile within `emulator` (None = its best one for the game).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Emulator actions: `args` replace the profile's arguments (`{Rom}` / `{rom}` is filled in,
    /// the ROM is added at the end when they don't name it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub replace_args: bool,
    /// Emulator actions: the emulator's switches for this action (absent = the game's / global).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub options: std::collections::BTreeMap<String, bool>,
    /// Emulator actions: which of the game's ROMs it starts (`GameRom::key`; None = the
    /// default ROM, the first of the list).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rom: Option<String>,
    /// Emulator actions: Play asks which profile of the emulator to use (Playnite's "Choose on
    /// startup"): one choice per profile.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub choose_profile: bool,
    /// Emulator actions: Play asks which of the game's ROMs to start: one choice per ROM.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub choose_rom: bool,
}

/// One ROM of an emulated game (Edit → Installation → ROMs; Playnite's ROM list): "Pokemon
/// Blue" + "Pokemon Azul" in one entry, or a PS3 game's `EBOOT.BIN` + its online mode's
/// `o/MGO2.SELF`. The list is stored only once the user changes it; until then the game has one
/// ROM, the file(s) the scan found.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameRom {
    /// What Play's chooser calls it ("" = the game's name for the scanned ROM, else the file's).
    pub name: String,
    /// "" = the file(s) the scan found (follows rescans; a multi-disc set is one ROM). Else a
    /// file or folder, `{InstallDir}/…` when it is inside the game's folder.
    pub path: String,
}

/// What one line of "Choose what to play" starts (also "Always use this", `UserEdits::play_default`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayChoice {
    /// The Play action (`edits.actions[i]`); None = the game's own launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<usize>,
    /// The action's name when it was chosen (a remembered choice only applies to the same one).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub action_name: String,
    /// The emulator of the profile below (the action's own, or the one Automatic picked).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emulator: Option<String>,
    /// The emulator profile ("Choose when playing").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// The ROM (`GameRom::key`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rom: Option<String>,
}

/// Shell snippets run around a game session (run with `sh -c`, as the user).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameScripts {
    pub before_start: String,
    pub after_start: String,
    pub after_exit: String,
    /// Also run the global scripts from Settings.
    pub use_global: bool,
}

impl Default for GameScripts {
    fn default() -> Self {
        GameScripts {
            before_start: String::new(),
            after_start: String::new(),
            after_exit: String::new(),
            use_global: true,
        }
    }
}

/// Everything the user edited by hand in Edit Game Details. `None` / empty = use the source's
/// value. Stored as JSON (`edits_json`); applied over the source values when the game is loaded,
/// so rescans never overwrite edits.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserEdits {
    pub sort_name: Option<String>,
    pub platform: Option<String>,
    pub description: Option<String>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub genres: Option<Vec<String>>,
    /// Free-form features ("Single player", "Co-op", …).
    pub features: Option<Vec<String>>,
    /// `Some(CLEARED_YEAR)` = cleared by the user.
    pub release_year: Option<i32>,
    /// "YYYY-MM-DD" when the full date is known.
    pub release_date: Option<String>,
    pub series: Option<String>,
    pub age_rating: Option<String>,
    pub region: Option<String>,
    pub completion: Option<CompletionStatus>,
    /// 0..=100
    pub user_score: Option<u8>,
    /// `Some(CLEARED_SCORE)` = cleared by the user (also `community_score`).
    pub critic_score: Option<u8>,
    pub community_score: Option<u8>,
    pub version: Option<String>,
    /// Manual: a file path or URL.
    pub manual: Option<String>,
    pub notes: String,
    /// Game options: this game's overrides of the global defaults.
    /// Lenient: one invalid option (a newer build's enum value, a hand edit) drops only itself.
    #[serde(
        default,
        skip_serializing_if = "crate::game_options::GameOptions::is_default",
        deserialize_with = "crate::game_options::deserialize_lenient"
    )]
    pub game_options: crate::game_options::GameOptions,
    /// Deprecated: migrated into `game_options.hdr` on load (see [`UserEdits::migrate`]); kept so
    /// old `edits_json` rows (`"hdr":true`) still parse.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hdr: bool,
    pub install_dir: Option<PathBuf>,
    pub installed: Option<bool>,
    pub install_size: Option<u64>,
    /// The user's own links (automatic ones come from `Game::auto_links`).
    pub links: Vec<GameLink>,
    pub hide_auto_links: bool,
    pub actions: Vec<GameAction>,
    pub scripts: GameScripts,
    /// Where the logo sits on the game page (dragged / resized by the user); None = automatic.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo_placement: Option<LogoPlacement>,
    /// Which part of the background shows (Arrange mode, panned / zoomed by the user); None =
    /// automatic. Used by every box the background fills (header, ambient, controller mode).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crop::deserialize_background_lenient"
    )]
    pub background_placement: Option<BackgroundPlacement>,
    /// Emulated games: the emulator instance id the user picked for this game (None = automatic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emulator: Option<String>,
    /// Emulated games: the profile within `emulator` (None = the emulator's best one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emulator_profile: Option<String>,
    /// Emulated games: the emulator's switches for this game (absent = the console's / global).
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub emulator_options: std::collections::BTreeMap<String, bool>,
    /// Emulated games: extra emulator arguments for this game (placed by the profile's template).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub emulator_args: String,
    /// Emulated games: the ROM list (empty = one ROM, the scanned one). The first is the default.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roms: Vec<GameRom>,
    /// "Choose what to play" → "Always use this": Play starts this one without asking.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play_default: Option<PlayChoice>,
    /// The RetroAchievements game id the user set (Edit → Installation); shows as
    /// `Metadata::ra_game_id` and wins over the automatic match. Kept here so rescans never
    /// touch it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ra_game_id: Option<u32>,
}

/// A hand-placed logo on the game page, relative to the header image so it survives window
/// resizes: centre as a fraction of the header's width / height, height as a fraction of the
/// header's height (the width follows the logo's aspect ratio).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LogoPlacement {
    pub cx: f32,
    pub cy: f32,
    pub h: f32,
}

impl LogoPlacement {
    /// Kept on screen and at a usable size whatever a hand-edited value says.
    pub fn clamped(self) -> LogoPlacement {
        let ok = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        LogoPlacement { cx: ok(self.cx, 0.0, 1.0, 0.5), cy: ok(self.cy, 0.0, 1.0, 0.5), h: ok(self.h, 0.05, 1.0, 0.3) }
    }
}

impl UserEdits {
    pub fn is_empty(&self) -> bool {
        *self == UserEdits::default()
    }

    /// Move the old `hdr: true` into `game_options.hdr` (On, unless the game already has its own
    /// HDR value). Idempotent; the next save persists the result.
    pub fn migrate(&mut self) {
        if self.hdr {
            if self.game_options.hdr == crate::game_options::Tri::Default {
                self.game_options.hdr = crate::game_options::Tri::On;
            }
            self.hdr = false;
        }
    }
}

/// What an emulated game is: its console, ROM files and identity. Stored as JSON (`emu_json`),
/// rewritten by every scan. The launch command is rebuilt from it at play time (so a different
/// emulator choice applies without a rescan).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmuRef {
    /// Playnite platform id ("sony_playstation2").
    pub platform: String,
    /// ROM folder id (config key, or "auto-<hash>" for auto-found roots).
    pub folder: String,
    /// What is launched by default: .m3u/.cue/.iso/.chd/.rvz/.nsp/… or a folder (Wii U, PS3).
    pub rom: PathBuf,
    /// The game's other files (bins, other discs): size, ROM list, alternative launch files.
    pub files: Vec<PathBuf>,
    /// Emulator instance id chosen at scan time ("" = none found).
    pub emulator: String,
    /// Profile id within `emulator`.
    pub profile: String,
    pub serial: Option<String>,
    /// Lower-case 8 hex digits.
    pub crc: Option<String>,
    /// No-Intro / Redump name from the hash database (the best libretro art key).
    pub dat_name: Option<String>,
    pub region: Option<String>,
}

/// Descriptive metadata (from scanners or online lookup).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub description: Option<String>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub genres: Vec<String>,
    pub release_year: Option<i32>,
    /// From Steam's app data (`controller_support` / categories 28, 18) or the store.
    #[serde(default)]
    pub controller: Option<ControllerSupport>,
    #[serde(default)]
    pub deck: Option<DeckRating>,
    /// ProtonDB rating tier ("platinum", "gold", "silver", "bronze", "borked", "native", "pending").
    #[serde(default)]
    pub protondb: Option<String>,
    /// "YYYY-MM-DD" when the full date is known (IGDB).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    /// IGDB themes ("Fantasy", "Horror" …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<String>,
    /// Modes and perspectives ("Single player", "Co-operative", "Side view" …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
    /// 0..=100 (IGDB `aggregated_rating`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critic_score: Option<u8>,
    /// 0..=100 (IGDB `rating`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_score: Option<u8>,
    /// The game's IGDB page, where the scores come from (the game page links it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub igdb_url: Option<String>,
    /// RetroAchievements game id the user set by hand (Edit → Installation), over the automatic
    /// match by ROM hash / name. Stored as `UserEdits::ra_game_id` (so rescans keep it) and
    /// applied here when the game loads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ra_game_id: Option<u32>,
    /// Series / franchise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub series: Option<String>,
    /// The English Wikipedia article `description` was taken from (its URL): the game page
    /// credits it ("From Wikipedia", CC BY-SA). Travels with `description` in every merge.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wikipedia: Option<String>,
    /// Fetched metadata only: what a lookup the user asked for (Edit → Download metadata…)
    /// found. It then wins over the scanner's own values (the user picked this game's data);
    /// background lookups only fill gaps.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub looked_up: bool,
    /// How many can play: "1", "1-4", "1+" (single player and multiplayer
    /// of unknown size), "2+". From Steam's categories or IGDB's game modes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub players: Option<String>,
}

/// "1" / "1-4" / "1+" / "2+" from whether a game has a single-player mode, a multiplayer
/// mode, and the most players a mode allows (0 = unknown). None when nothing is known.
pub fn players_text(single: bool, multi: bool, max: u32) -> Option<String> {
    let lo = if single || !multi { 1 } else { 2 };
    match (single, multi) {
        (_, true) if max > lo => Some(format!("{lo}-{max}")),
        (true, true) => Some("1+".into()),
        (false, true) => Some("2+".into()),
        (true, false) => Some("1".into()),
        (false, false) if max > 0 => Some(if max == 1 { "1".into() } else { format!("1-{max}") }),
        (false, false) => None,
    }
}

/// Players from game modes / feature names ("Single player", "Multiplayer", "Co-operative",
/// "Split screen" …, IGDB's and Steam's wording).
pub fn players_from_features(features: &[String]) -> Option<String> {
    let (single, multi) = player_modes(features);
    players_text(single, multi, 0)
}

/// (has a single-player mode, has a multiplayer mode) from game mode / feature names.
pub fn player_modes(features: &[String]) -> (bool, bool) {
    let mut single = false;
    let mut multi = false;
    for f in features {
        let l = f.to_ascii_lowercase();
        if l.contains("single") {
            single = true;
        } else if l.contains("multi")
            || l.contains("co-op")
            || l.contains("cooperative")
            || l.contains("co-operative")
            || l.contains("split screen")
            || l.contains("split-screen")
            || l.contains("pvp")
            || l.contains("battle royale")
            || l.contains("mmo")
        {
            multi = true;
        }
    }
    (single, multi)
}

/// Players from Steam category ids (store and local app data): 2 single-player; 1, 9, 20, 24,
/// 36–39, 47–49 multiplayer / co-op / split screen / online.
pub fn players_from_steam_categories(cats: &[u64]) -> Option<String> {
    let single = cats.contains(&2);
    let multi = cats.iter().any(|c| matches!(c, 1 | 9 | 20 | 24 | 36..=39 | 47..=49));
    players_text(single, multi, 0)
}

/// `UserEdits::release_year` of a year the user cleared (no real game is from year 0).
pub const CLEARED_YEAR: i32 = 0;
/// `UserEdits::critic_score` / `community_score` the user cleared (scores are 0..=100).
pub const CLEARED_SCORE: u8 = u8::MAX;

impl Metadata {
    /// What an explicit lookup stores over the metadata fetched before (`self`): everything
    /// `found` has, and nothing left from an earlier (maybe wrong) match — except the facts of
    /// a Steam app id (controller support, Steam Deck, ProtonDB), which aren't about the name
    /// looked up and stay until they are answered again.
    pub fn replaced_by(&self, found: &Metadata) -> Metadata {
        let mut m = found.clone();
        if m.controller.is_none() {
            m.controller = self.controller;
        }
        if m.deck.is_none() {
            m.deck = self.deck;
        }
        if m.protondb.is_none() {
            m.protondb.clone_from(&self.protondb);
        }
        m.looked_up = true;
        m
    }

    /// Drop every field `known` already has (so a merge that lets new values win only fills
    /// gaps).
    pub fn keep_only_missing(&mut self, known: &Metadata) {
        fn drop<T>(v: &mut Option<T>, k: &Option<T>) {
            if k.is_some() {
                *v = None;
            }
        }
        fn drop_list(v: &mut Vec<String>, k: &[String]) {
            if !k.is_empty() {
                v.clear();
            }
        }
        if known.description.is_some() {
            self.wikipedia = None;
        }
        drop(&mut self.description, &known.description);
        drop(&mut self.developer, &known.developer);
        drop(&mut self.publisher, &known.publisher);
        drop(&mut self.release_year, &known.release_year);
        drop(&mut self.release_date, &known.release_date);
        drop(&mut self.critic_score, &known.critic_score);
        drop(&mut self.community_score, &known.community_score);
        drop(&mut self.igdb_url, &known.igdb_url);
        drop(&mut self.series, &known.series);
        drop(&mut self.controller, &known.controller);
        drop(&mut self.deck, &known.deck);
        drop(&mut self.protondb, &known.protondb);
        drop(&mut self.players, &known.players);
        drop(&mut self.ra_game_id, &known.ra_game_id);
        drop_list(&mut self.genres, &known.genres);
        drop_list(&mut self.themes, &known.themes);
        drop_list(&mut self.features, &known.features);
    }

    pub fn fill_from(&mut self, other: &Metadata) {
        if self.controller.is_none() {
            self.controller = other.controller;
        }
        if self.deck.is_none() {
            self.deck = other.deck;
        }
        if self.protondb.is_none() {
            self.protondb.clone_from(&other.protondb);
        }
        if self.description.is_none() {
            self.description.clone_from(&other.description);
            // The credit belongs to whichever description is kept.
            self.wikipedia.clone_from(&other.wikipedia);
        }
        if self.developer.is_none() {
            self.developer.clone_from(&other.developer);
        }
        if self.publisher.is_none() {
            self.publisher.clone_from(&other.publisher);
        }
        if self.genres.is_empty() {
            self.genres.clone_from(&other.genres);
        }
        if self.release_year.is_none() {
            self.release_year = other.release_year;
        }
        if self.release_date.is_none() {
            self.release_date.clone_from(&other.release_date);
        }
        if self.themes.is_empty() {
            self.themes.clone_from(&other.themes);
        }
        if self.features.is_empty() {
            self.features.clone_from(&other.features);
        }
        if self.critic_score.is_none() {
            self.critic_score = other.critic_score;
        }
        if self.community_score.is_none() {
            self.community_score = other.community_score;
        }
        if self.igdb_url.is_none() {
            self.igdb_url.clone_from(&other.igdb_url);
        }
        if self.series.is_none() {
            self.series.clone_from(&other.series);
        }
        if self.players.is_none() {
            self.players.clone_from(&other.players);
        }
        if self.ra_game_id.is_none() {
            self.ra_game_id = other.ra_game_id;
        }
    }
}

/// What a scanner reports for one game. Merged into the DB by `(source, source_id)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScannedGame {
    pub source: Source,
    /// Stable id within the source (Steam appid, Faugus gameid, desktop file id …).
    pub source_id: String,
    pub name: String,
    pub installed: bool,
    pub install_dir: Option<PathBuf>,
    pub launch: Option<LaunchSpec>,
    pub art: Art,
    pub meta: Metadata,
    /// Playtime the source itself knows about (Steam, Lutris, Heroic). Replaces the stored value
    /// when it is larger.
    pub playtime_secs: Option<u64>,
    /// Unix seconds.
    pub last_played: Option<i64>,
    pub size_bytes: Option<u64>,
    /// "linux", "windows", … when known.
    pub platform: Option<String>,
    /// For desktop entries that are really shortcuts into another source
    /// (e.g. `steam steam://rungameid/123`): that source and its id, so the orchestrator
    /// can drop the duplicate and borrow its art.
    pub duplicate_of: Option<(Source, String)>,
    /// Launch options configured in the source launcher (Steam's per-game "Launch Options",
    /// Faugus launch_arguments …). Informational: shown read-only in the editor.
    pub source_launch_options: Option<String>,
    /// Emulated games (`Source::Emulator`): console, ROM files and identity.
    pub emu: Option<EmuRef>,
    /// Entries of other sources this game takes the place of (a RomM game once downloaded: the
    /// ROM found on disk replaces it): their library rows hand the user's data (favourite,
    /// tags, collections, playtime …) to this game's row and go (`Db::merge_scan`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub absorbs: Vec<(Source, String)>,
}

impl ScannedGame {
    pub fn new(source: Source, source_id: impl Into<String>, name: impl Into<String>) -> Self {
        ScannedGame {
            source,
            source_id: source_id.into(),
            name: name.into(),
            installed: true,
            install_dir: None,
            launch: None,
            art: Art::default(),
            meta: Metadata::default(),
            playtime_secs: None,
            last_played: None,
            size_bytes: None,
            platform: None,
            duplicate_of: None,
            source_launch_options: None,
            emu: None,
            absorbs: vec![],
        }
    }
}

/// A game in the library (DB row + user data).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Game {
    pub id: GameId,
    pub source: Source,
    pub source_id: String,
    /// Name as reported by the source.
    pub name: String,
    /// Name the user set; overrides `name` everywhere when present.
    pub user_name: Option<String>,
    /// Lowercased, leading articles removed; used for sorting and letter jumps.
    pub sort_name: String,
    pub installed: bool,
    pub install_dir: Option<PathBuf>,
    pub launch: Option<LaunchSpec>,
    pub art: Art,
    pub meta: Metadata,
    pub playtime_secs: u64,
    pub last_played: Option<i64>,
    pub play_count: u32,
    /// Unix seconds when first seen.
    pub added: i64,
    /// Unix seconds of the last change to the game's data (edits, favourite, hidden, tags,
    /// artwork, metadata); None = not changed since it was added (or before 0.9).
    pub modified: Option<i64>,
    pub size_bytes: Option<u64>,
    pub platform: Option<String>,
    pub favorite: bool,
    pub hidden: bool,
    pub tags: Vec<String>,
    /// 0..=5
    pub user_rating: Option<u8>,
    /// User launch customisation (None = launch as the source does).
    pub launch_override: Option<LaunchOverride>,
    /// See `ScannedGame::source_launch_options`.
    pub source_launch_options: Option<String>,
    /// The user's own controller-support setting; overrides `meta.controller`.
    pub controller_override: Option<ControllerSupport>,
    /// Hand edits (already applied to `meta`, `platform`, `install_dir`, `installed`,
    /// `size_bytes`, `sort_name` when loaded; kept here so the editor knows what is overridden).
    pub edits: UserEdits,
    /// Values as the source / online lookup gave them, before `edits` (for "reset to source").
    pub source_meta: Metadata,
    /// Emulated games: console, ROM files and identity (see [`EmuRef`]).
    pub emu: Option<EmuRef>,
    /// The game vanished from a library that scanned fine (None = it is there).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missing: Option<Missing>,
}

/// A game no longer in the library it came from (e.g. a non-Steam shortcut deleted in Steam).
/// Kept because it has the user's data (playtime, favourite, tags …) until the user removes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Missing {
    /// Unix seconds of the scan that first didn't find it.
    pub since: i64,
    /// The user chose Keep (Settings → Libraries → Missing games): no longer listed there.
    pub kept: bool,
}

impl Game {
    /// An empty game (the starting point of Add Game → Manually…).
    pub fn blank(name: &str) -> Game {
        Game {
            id: 0,
            source: Source::Manual,
            source_id: String::new(),
            name: name.to_string(),
            user_name: None,
            sort_name: sort_key(name),
            installed: true,
            install_dir: None,
            launch: None,
            art: Art::default(),
            meta: Metadata::default(),
            playtime_secs: 0,
            last_played: None,
            play_count: 0,
            added: 0,
            modified: None,
            size_bytes: None,
            platform: None,
            favorite: false,
            hidden: false,
            tags: vec![],
            user_rating: None,
            launch_override: None,
            source_launch_options: None,
            controller_override: None,
            edits: UserEdits::default(),
            source_meta: Metadata::default(),
            emu: None,
            missing: None,
        }
    }

    pub fn display_name(&self) -> &str {
        self.user_name.as_deref().unwrap_or(&self.name)
    }

    /// Apply `edits` over the source values (keeps the source values in `source_meta`).
    pub fn with_edits_applied(mut self) -> Game {
        self.source_meta = self.meta.clone();
        let e = &self.edits;
        let m = &mut self.meta;
        fn over<T: Clone>(slot: &mut Option<T>, v: &Option<T>) {
            if v.is_some() {
                slot.clone_from(v);
            }
        }
        if e.description.is_some() {
            // The user's own text isn't Wikipedia's.
            m.wikipedia = None;
        }
        over(&mut m.description, &e.description);
        over(&mut m.developer, &e.developer);
        over(&mut m.publisher, &e.publisher);
        over(&mut m.release_year, &e.release_year);
        over(&mut m.release_date, &e.release_date);
        over(&mut m.series, &e.series);
        over(&mut m.critic_score, &e.critic_score);
        over(&mut m.community_score, &e.community_score);
        over(&mut m.ra_game_id, &e.ra_game_id.filter(|i| *i > 0));
        if let Some(f) = &e.features {
            m.features.clone_from(f);
        }
        if let Some(g) = &e.genres {
            m.genres.clone_from(g);
        }
        over(&mut self.platform, &e.platform);
        // A year / score the user cleared shows as empty too (like the text fields below).
        if m.release_year == Some(CLEARED_YEAR) {
            m.release_year = None;
        }
        for s in [&mut m.critic_score, &mut m.community_score] {
            if *s == Some(CLEARED_SCORE) {
                *s = None;
            }
        }
        // A field the user cleared (`Some("")`) shows as empty, whatever the source has.
        for slot in [
            &mut m.description,
            &mut m.developer,
            &mut m.publisher,
            &mut m.release_date,
            &mut m.series,
            &mut self.platform,
        ] {
            if slot.as_deref().is_some_and(|v| v.trim().is_empty()) {
                *slot = None;
            }
        }
        if e.install_dir.is_some() {
            self.install_dir.clone_from(&e.install_dir);
        }
        if let Some(i) = e.installed {
            self.installed = i;
        }
        if e.install_size.is_some() {
            self.size_bytes = e.install_size;
        }
        if let Some(s) = e.sort_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            self.sort_name = sort_key(s);
        }
        self
    }

    /// The action the Play button runs, when the user defined one.
    pub fn play_action(&self) -> Option<&GameAction> {
        self.edits.actions.iter().find(|a| a.play)
    }

    /// Steam app id for store games (not shortcuts).
    pub fn steam_appid(&self) -> Option<u32> {
        if self.source != Source::Steam {
            return None;
        }
        self.source_id.parse().ok()
    }

    /// Links Lodestar knows by itself (Steam store, SteamDB, ProtonDB, PCGamingWiki for Steam games).
    pub fn auto_links(&self) -> Vec<GameLink> {
        let Some(id) = self.steam_appid() else { return vec![] };
        let l = |n: &str, u: String| GameLink { name: n.into(), url: u };
        vec![
            l("Steam", format!("https://store.steampowered.com/app/{id}")),
            l("ProtonDB", format!("https://www.protondb.com/app/{id}")),
            l("PCGamingWiki", format!("https://pcgamingwiki.com/api/appid.php?appid={id}")),
            l("SteamDB", format!("https://steamdb.info/app/{id}/")),
        ]
    }

    /// All links to show: automatic (unless hidden) then the user's own.
    pub fn links(&self) -> Vec<GameLink> {
        let mut v = if self.edits.hide_auto_links { vec![] } else { self.auto_links() };
        v.extend(self.edits.links.iter().cloned());
        v
    }

    /// The user's completion status (None = not set).
    pub fn completion(&self) -> Option<CompletionStatus> {
        self.edits.completion
    }

    /// Controller support to show: the user's setting, else what the source says.
    pub fn controller(&self) -> Option<ControllerSupport> {
        self.controller_override.or(self.meta.controller)
    }
}

/// Compute a sort key: lowercase, strip leading "the "/"a "/"an ", strip non-alphanumerics at start.
pub fn sort_key(name: &str) -> String {
    let lower = name.trim().to_lowercase();
    let stripped = ["the ", "a ", "an "].iter().find_map(|p| lower.strip_prefix(p)).unwrap_or(&lower);
    stripped.trim_start_matches(|c: char| !c.is_alphanumeric()).to_string()
}

/// Format seconds as "12h 30m" / "45m" / "Never played".
pub fn format_playtime(secs: u64) -> String {
    if secs == 0 {
        return "Never played".into();
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    if h > 0 {
        format!("{h}h {m}m")
    } else if m > 0 {
        format!("{m}m")
    } else {
        "<1m".into()
    }
}

/// Unix seconds now.
pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// "Today", "Yesterday", "3 days ago", "12 Mar 2025" …
pub fn format_last_played(ts: Option<i64>) -> String {
    let Some(ts) = ts else { return "Never".into() };
    let now = now_unix();
    let days = (now - ts).max(0) / 86400;
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=13 => format!("{days} days ago"),
        14..=59 => format!("{} weeks ago", days / 7),
        _ => {
            let (y, m, d) = civil_from_days(ts.div_euclid(86400));
            const MONTHS: [&str; 12] =
                ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            format!("{d} {} {y}", MONTHS[(m - 1) as usize])
        }
    }
}

/// Days since 1970-01-01 -> (year, month 1-12, day 1-31). Howard Hinnant's algorithm.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_keys() {
        assert_eq!(sort_key("The Witcher 3"), "witcher 3");
        assert_eq!(sort_key("  ELDEN RING"), "elden ring");
        assert_eq!(sort_key("\"Hello\" World"), "hello\" world");
        assert_eq!(sort_key("A Plague Tale"), "plague tale");
    }

    #[test]
    fn civil() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_000), (2024, 10, 4));
    }

    #[test]
    fn a_cleared_field_stays_empty() {
        // Clearing Developer / Description in Edit must not bring the source's text back: a
        // cleared field is saved as Some("") and shows as empty.
        let mut g = Game::blank("X");
        g.meta.developer = Some("Source Dev".into());
        g.meta.description = Some("From the store".into());
        g.meta.genres = vec!["Action".into()];
        g.edits.developer = Some(String::new());
        g.edits.description = Some("  ".into());
        g.edits.genres = Some(vec![]);
        let g = g.with_edits_applied();
        assert_eq!(g.meta.developer, None);
        assert_eq!(g.meta.description, None);
        assert!(g.meta.genres.is_empty());
        // The source's values are still known (Reset to the source brings them back).
        assert_eq!(g.source_meta.developer.as_deref(), Some("Source Dev"));
    }

    #[test]
    fn source_keys_roundtrip() {
        for s in Source::ALL {
            assert_eq!(Source::from_key(s.key()), Some(s));
        }
    }

    #[test]
    fn background_placement_in_edits_json() {
        // Not written while automatic (old rows stay the same).
        assert!(!serde_json::to_string(&UserEdits::default()).unwrap().contains("background"));
        let e = UserEdits {
            background_placement: Some(BackgroundPlacement { x: 0.25, y: 0.75, zoom: 2.0 }),
            ..Default::default()
        };
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<UserEdits>(&json).unwrap(), e);
        assert!(!e.is_empty());
        // A broken value drops only itself, the other edits survive.
        let e: UserEdits = serde_json::from_str(
            r#"{"notes":"hi","background_placement":"top","logo_placement":{"cx":0.5,"cy":0.5,"h":0.3}}"#,
        )
        .unwrap();
        assert_eq!(e.background_placement, None);
        assert_eq!(e.notes, "hi");
        assert!(e.logo_placement.is_some());
    }
}
