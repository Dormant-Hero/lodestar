//! Controller-mode settings: theme, fullscreen, the side lists, text size, the power menu and
//! button prompts.

use super::*;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ControllerConfig {
    /// Theme name used in controller mode.
    pub theme: String,
    /// Fullscreen when in controller mode.
    pub fullscreen: bool,
    /// Theme variant (themes can offer several).
    pub variant: Option<String>,
    /// The theme's colour scheme (`[color_schemes]`; None: its own colours).
    pub color_scheme: Option<String>,
    /// Swap A/B (Nintendo layout).
    pub swap_ab: bool,
    /// Initial repeat delay / repeat interval for held directions, ms.
    pub repeat_delay_ms: u64,
    pub repeat_interval_ms: u64,
    /// Show the quit option in the quick menu (disable for kiosk setups).
    pub allow_quit: bool,
    /// The menu button in desktop mode switches to controller mode. Off by default: on the
    /// desktop the button may be meant for Steam. (In controller mode the menu button always
    /// opens and closes the quick menu; this switch doesn't apply there.)
    pub guide_opens_controller_mode: bool,
    /// Which controller's labels button prompts show. `auto` follows the pad last used.
    pub button_prompts: ButtonPrompts,
    /// The menu button (Home or the combination) brings Lodestar to the front (from the tray,
    /// minimised or behind other windows); while a game runs it opens the in-game menu.
    pub guide_opens_lodestar: bool,
    /// The controller drives Lodestar even when another window is active (no game running).
    /// While a game runs this never applies: only the Guide button gets through then.
    /// Settings: "Only use the controller when Lodestar is the active window" = the opposite.
    pub background_input: bool,
    /// Hide the mouse pointer in controller mode until the mouse moves.
    pub hide_cursor: bool,
    /// Controller mode shows the time / the battery level (laptops, handhelds).
    pub show_clock: bool,
    pub show_battery: bool,
    /// Controller mode visuals (the layout reads these).
    pub blur_background: bool,
    pub darken_background: bool,
    pub show_titles: bool,
    pub darken_not_installed: bool,
    pub text_size: TextSize,
    /// Power options shown in the controller menu.
    pub power: PowerItems,
    /// What opens Lodestar's menu: a button combination (`menu_combo`) or Home.
    pub menu_button: MenuButton,
    /// The button combination held to open Lodestar's menu (when `menu_button` is Combo).
    pub menu_combo: MenuCombo,
    /// "Which button opens Lodestar's menu?" was answered (asked once, on the first press).
    pub menu_button_asked: bool,
    /// Settings → Audio: navigation sounds (the theme's, else Lodestar's own clicks).
    /// Automatic = on when the controller theme brings its own sounds.
    pub nav_sounds: AudioMode,
    /// Settings → Audio: background music (the theme's, the automatic music folder or `music_folder`).
    /// Automatic = on when the controller theme brings its own music.
    pub music: AudioMode,
    /// Sound and music volume, percent.
    pub volume: u8,
    /// Music folder chosen in Settings → Audio (None = automatic).
    pub music_folder: Option<String>,
    /// Settings → Visuals: play theme videos (needs ffmpeg; otherwise the still picture shows).
    pub theme_videos: bool,
    /// Older versions: what the Home button did while a Steam game ran. Read once into
    /// `menu_button` (see `Config::sanitized`), never written.
    #[serde(skip_serializing)]
    pub steam_home: Option<SteamHome>,
    /// Theme animations (transitions) show their end state at once. None = follow the
    /// desktop's own "no animations" setting (KDE: animation speed "Instant").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduce_motion: Option<bool>,
}

/// Controller settings → Visuals → Text size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextSize {
    Small,
    #[default]
    Normal,
    Large,
    Larger,
}

impl TextSize {
    pub const ALL: [TextSize; 4] = [TextSize::Small, TextSize::Normal, TextSize::Large, TextSize::Larger];

    pub fn label(self) -> &'static str {
        match self {
            TextSize::Small => "Small",
            TextSize::Normal => "Normal",
            TextSize::Large => "Large",
            TextSize::Larger => "Extra large",
        }
    }

    /// Multiplier for controller-mode text.
    pub fn factor(self) -> f32 {
        match self {
            TextSize::Small => 0.88,
            TextSize::Normal => 1.0,
            TextSize::Large => 1.15,
            TextSize::Larger => 1.3,
        }
    }

    pub fn next(self) -> TextSize {
        let i = Self::ALL.iter().position(|t| *t == self).unwrap_or(1);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Controller settings → Menus: which power options the controller menu offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PowerItems {
    pub shutdown: bool,
    pub restart: bool,
    pub suspend: bool,
    pub lock: bool,
    pub logout: bool,
}

impl Default for PowerItems {
    fn default() -> Self {
        PowerItems { shutdown: true, restart: true, suspend: true, lock: false, logout: false }
    }
}

/// Settings → Controller → Button prompts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ButtonPrompts {
    /// The style of the gamepad used last (Xbox without one).
    #[default]
    Auto,
    Xbox,
    PlayStation,
    Nintendo,
}

impl ButtonPrompts {
    pub const ALL: [ButtonPrompts; 4] =
        [ButtonPrompts::Auto, ButtonPrompts::Xbox, ButtonPrompts::PlayStation, ButtonPrompts::Nintendo];

    pub fn label(self) -> &'static str {
        match self {
            ButtonPrompts::Auto => "Automatic",
            ButtonPrompts::Xbox => "Xbox",
            ButtonPrompts::PlayStation => "PlayStation",
            ButtonPrompts::Nintendo => "Nintendo",
        }
    }

    /// The style to draw: a manual choice always wins; `auto` uses `detected()` (the pad in
    /// use), Xbox without one.
    pub fn resolve(self, detected: impl FnOnce() -> Option<crate::input::ButtonStyle>) -> crate::input::ButtonStyle {
        use crate::input::ButtonStyle as S;
        match self {
            ButtonPrompts::Auto => detected().unwrap_or_default(),
            ButtonPrompts::Xbox => S::Xbox,
            ButtonPrompts::PlayStation => S::PlayStation,
            ButtonPrompts::Nintendo => S::Nintendo,
        }
    }

    pub fn next(self) -> ButtonPrompts {
        let i = Self::ALL.iter().position(|p| *p == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

impl ControllerConfig {
    /// Navigation sounds play with this theme.
    pub fn sounds_enabled(&self, theme: &crate::theme::model::ThemeFile) -> bool {
        self.nav_sounds.enabled(!theme.sounds.is_empty())
    }

    /// Background music plays with this theme.
    pub fn music_enabled(&self, theme: &crate::theme::model::ThemeFile) -> bool {
        self.music.enabled(!theme.music.is_empty())
    }
}

impl Default for ControllerConfig {
    fn default() -> Self {
        ControllerConfig {
            theme: "Lodestar".into(),
            fullscreen: true,
            variant: None,
            color_scheme: None,
            swap_ab: false,
            repeat_delay_ms: 380,
            repeat_interval_ms: 70,
            allow_quit: true,
            guide_opens_controller_mode: false,
            button_prompts: ButtonPrompts::Auto,
            guide_opens_lodestar: true,
            background_input: true,
            hide_cursor: true,
            show_clock: true,
            show_battery: true,
            blur_background: true,
            darken_background: true,
            show_titles: true,
            darken_not_installed: true,
            text_size: TextSize::Normal,
            power: PowerItems::default(),
            menu_button: MenuButton::Combo,
            menu_combo: MenuCombo::STICKS,
            menu_button_asked: false,
            nav_sounds: AudioMode::Auto,
            music: AudioMode::Auto,
            volume: 70,
            music_folder: None,
            theme_videos: true,
            steam_home: None,
            reduce_motion: None,
        }
    }
}
