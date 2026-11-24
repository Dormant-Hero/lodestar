//! Settings → Emulation: emulators, ROM folders, per-console choices and BIOS folders.

use super::*;

/// Emulation (Settings → Emulation): emulators, ROM folders, per-console choices, BIOS folders.
/// Tables are maps keyed by short ids so one bad entry in a hand-edited file drops only itself
/// (`Config::parse_lenient` is lenient per leaf of a table, but treats an array as one value).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmulationConfig {
    /// Find installed emulators by themselves (Flatpak, AppImage, PATH, RetroArch cores).
    pub auto_emulators: bool,
    /// Find ROM folders by themselves (~/ROMs, ~/Emulation/roms, drives, emulators' own lists).
    pub auto_folders: bool,
    /// Start emulators fullscreen.
    pub fullscreen: bool,
    /// ROM folders the user added (key = short stable id "f<n>", part of every game's id).
    pub rom_folders: std::collections::BTreeMap<String, RomFolder>,
    /// Auto-found ROM folders the user removed.
    pub ignored_folders: Vec<PathBuf>,
    /// Emulators the user added, or overrides of auto-found ones (same key = override).
    pub emulators: std::collections::BTreeMap<String, EmulatorEntry>,
    /// Auto-found emulator instance ids the user removed.
    pub hidden_emulators: Vec<String>,
    /// The user's own profiles, key "<emulator id>/<profile id>".
    pub profiles: std::collections::BTreeMap<String, ProfileEntry>,
    /// Platform id → "<emulator id>/<profile id>": the emulator a console's games use.
    pub platform_emulator: std::collections::BTreeMap<String, String>,
    /// Folders the user chose that hold BIOS files.
    pub bios_dirs: Vec<PathBuf>,
    /// BIOS folders the user removed: never searched, nor anything inside them (a backup
    /// drive's old BIOS dumps, an old copy of an emulator), unless chosen again.
    pub ignored_bios_dirs: Vec<PathBuf>,
    /// Emulator instance id → its switches and extra arguments for every game.
    pub emulator_settings: std::collections::BTreeMap<String, crate::emulation::options::EmuSettings>,
    /// Platform id → the switches and extra arguments for every game of that console.
    pub console_settings: std::collections::BTreeMap<String, crate::emulation::options::EmuSettings>,
    /// Emulator folders (Settings → Emulation): searched with their sub-folders when emulators
    /// are found automatically (`…/Emulators` with one sub-folder per emulator).
    pub emulator_dirs: Vec<PathBuf>,
    /// "Found emulators in <folder> — Add this folder" suggestions the user dismissed.
    pub dismissed_emulator_dirs: Vec<PathBuf>,
    /// The folders named "Emulators" on the drives were checked once (0.14): before, every one
    /// was searched by itself; the ones that held emulators were added to `emulator_dirs`.
    /// False in configs from before (and until that first check ran); new configs start true.
    pub emulator_dirs_checked: bool,
}

impl EmulationConfig {
    /// No automatic emulator / ROM folder detection (test contexts: never probe the machine).
    #[cfg(test)]
    pub fn detect_nothing() -> Self {
        EmulationConfig { auto_emulators: false, auto_folders: false, ..Default::default() }
    }
}

impl Default for EmulationConfig {
    fn default() -> Self {
        EmulationConfig {
            auto_emulators: true,
            auto_folders: true,
            fullscreen: true,
            rom_folders: Default::default(),
            ignored_folders: vec![],
            emulators: Default::default(),
            hidden_emulators: vec![],
            profiles: Default::default(),
            platform_emulator: Default::default(),
            bios_dirs: vec![],
            ignored_bios_dirs: vec![],
            emulator_settings: Default::default(),
            console_settings: Default::default(),
            emulator_dirs: vec![],
            dismissed_emulator_dirs: vec![],
            emulator_dirs_checked: false,
        }
    }
}

/// A ROM folder the user added.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RomFolder {
    pub path: PathBuf,
    /// Platform id of every ROM in it; "" = automatic (the folder's own name, else its
    /// sub-folders are consoles: "ps2", "gc", "3ds" …).
    pub platform: String,
    /// Emulator instance id for its games; "" = automatic.
    pub emulator: String,
    /// Profile id within `emulator`; "" = automatic.
    pub profile: String,
    /// Glob patterns on paths relative to the folder that are left out.
    pub exclude: Vec<String>,
    pub enabled: bool,
}

impl Default for RomFolder {
    fn default() -> Self {
        RomFolder {
            path: PathBuf::new(),
            platform: String::new(),
            emulator: String::new(),
            profile: String::new(),
            exclude: vec![],
            enabled: true,
        }
    }
}

/// An emulator the user added (or an override of an auto-found one).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmulatorEntry {
    pub name: String,
    /// Built-in definition id it behaves like (profiles, arguments); "" = fully custom.
    pub follows: String,
    /// "flatpak" | "appimage" | "binary".
    pub kind: String,
    /// Flatpak app id (kind "flatpak").
    pub app_id: String,
    /// Program or AppImage (kind "appimage" / "binary").
    pub path: PathBuf,
    /// "" = the profile's arguments; else replaces them.
    pub args: String,
}

/// A profile the user made (how one emulator starts games of some consoles).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProfileEntry {
    pub name: String,
    /// Emulator instance id it belongs to.
    pub emulator: String,
    /// Platform ids.
    pub platforms: Vec<String>,
    /// ROM file extensions, preferred first (no dot).
    pub extensions: Vec<String>,
    pub args: String,
    /// RetroArch core (file name or path); "" = none.
    pub core: String,
    /// Its own program instead of the emulator's (a fork, a test build); empty = the emulator's.
    #[serde(default, skip_serializing_if = "path_is_empty")]
    pub program: PathBuf,
    /// Working folder (`{EmulatorDir}`, `{InstallDir}`); "" = automatic.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub cwd: String,
    /// How Play follows its games.
    #[serde(default, skip_serializing_if = "is_default_tracking")]
    pub tracking: crate::model::TrackingMode,
}

fn path_is_empty(p: &Path) -> bool {
    p.as_os_str().is_empty()
}

fn is_default_tracking(t: &crate::model::TrackingMode) -> bool {
    *t == crate::model::TrackingMode::Default
}
