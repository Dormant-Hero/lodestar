//! Desktop-mode settings: the library view (grid / list / details), sorting, grouping, quick
//! filter presets, and the windows' remembered sizes.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DesktopView {
    #[default]
    Grid,
    List,
    Details,
}

/// Sort order of game lists. The normal (not reversed, `descending = false`) direction is the
/// useful one for each: A→Z for `Name`, `Source`, `InstallFolder` and `RomPath`, newest / most /
/// biggest / best first for the others, `Completion` in its status order (see `Library::sort`).
/// Games without the value (no size, no score …) stay at the end in both directions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SortBy {
    #[default]
    Name,
    LastPlayed,
    Playtime,
    Added,
    /// Release date (the full date when known, else the year). Kept as `release_year` in
    /// config files written before 0.9.
    ReleaseYear,
    Source,
    PlayCount,
    InstallSize,
    Completion,
    UserScore,
    /// Last time the game's data changed (edits, favourite, tags, metadata, artwork).
    Modified,
    InstallFolder,
    /// The ROM file of emulated games.
    RomPath,
    /// Share of achievements unlocked (games without any last).
    Achievements,
}

impl SortBy {
    /// In menu order.
    pub const ALL: [SortBy; 14] = [
        SortBy::Name,
        SortBy::LastPlayed,
        SortBy::Playtime,
        SortBy::PlayCount,
        SortBy::Added,
        SortBy::Modified,
        SortBy::ReleaseYear,
        SortBy::UserScore,
        SortBy::Completion,
        SortBy::Achievements,
        SortBy::InstallSize,
        SortBy::Source,
        SortBy::InstallFolder,
        SortBy::RomPath,
    ];
    pub fn label(self) -> &'static str {
        match self {
            SortBy::Name => "Name",
            SortBy::LastPlayed => "Last played",
            SortBy::Playtime => "Play time",
            SortBy::Added => "Recently added",
            SortBy::ReleaseYear => "Release date",
            SortBy::Source => "Source",
            SortBy::PlayCount => "Play count",
            SortBy::InstallSize => "Install size",
            SortBy::Completion => "Completion",
            SortBy::UserScore => "User score",
            SortBy::Modified => "Recently modified",
            SortBy::InstallFolder => "Install folder",
            SortBy::RomPath => "ROM file",
            SortBy::Achievements => "Achievements",
        }
    }
}

/// Group headers in the desktop views (Grid, List, Details list).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    #[default]
    None,
    Source,
    Platform,
    Genre,
    Developer,
    Publisher,
    ReleaseYear,
    Installed,
    Completion,
    /// Today / This week / This month / Earlier / Never.
    LastPlayed,
    /// Play time buckets.
    Playtime,
    /// The drive (mount point) of the install folder / ROM.
    InstallDrive,
    Tag,
    /// The user's own collections.
    Collection,
}

impl GroupBy {
    /// In menu order.
    pub const ALL: [GroupBy; 14] = [
        GroupBy::None,
        GroupBy::Source,
        GroupBy::Platform,
        GroupBy::Genre,
        GroupBy::Developer,
        GroupBy::Publisher,
        GroupBy::ReleaseYear,
        GroupBy::Installed,
        GroupBy::Completion,
        GroupBy::LastPlayed,
        GroupBy::Playtime,
        GroupBy::InstallDrive,
        GroupBy::Tag,
        GroupBy::Collection,
    ];
    pub fn label(self) -> &'static str {
        match self {
            GroupBy::None => "None",
            GroupBy::Source => "Source",
            GroupBy::Platform => "Platform",
            GroupBy::Genre => "Genre",
            GroupBy::Developer => "Developer",
            GroupBy::Publisher => "Publisher",
            GroupBy::ReleaseYear => "Release year",
            GroupBy::Installed => "Installed",
            GroupBy::Completion => "Completion",
            GroupBy::LastPlayed => "Last played",
            GroupBy::Playtime => "Play time",
            GroupBy::InstallDrive => "Install drive",
            GroupBy::Tag => "Tag",
            GroupBy::Collection => "Collection",
        }
    }
    /// Stable short key (remembered collapsed groups, testing hooks).
    pub fn key(self) -> &'static str {
        match self {
            GroupBy::None => "none",
            GroupBy::Source => "source",
            GroupBy::Platform => "platform",
            GroupBy::Genre => "genre",
            GroupBy::Developer => "developer",
            GroupBy::Publisher => "publisher",
            GroupBy::ReleaseYear => "release_year",
            GroupBy::Installed => "installed",
            GroupBy::Completion => "completion",
            GroupBy::LastPlayed => "last_played",
            GroupBy::Playtime => "playtime",
            GroupBy::InstallDrive => "install_drive",
            GroupBy::Tag => "tag",
            GroupBy::Collection => "collection",
        }
    }
}

/// The quick presets next to the search box (they narrow whatever the sidebar shows).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum QuickPreset {
    #[default]
    All,
    Installed,
    Favorites,
    /// Played games, last played first.
    Recent,
    /// Played games, most play time first.
    MostPlayed,
}

impl QuickPreset {
    pub const ALL: [QuickPreset; 5] = [
        QuickPreset::All,
        QuickPreset::Installed,
        QuickPreset::Favorites,
        QuickPreset::Recent,
        QuickPreset::MostPlayed,
    ];
    pub fn label(self) -> &'static str {
        match self {
            QuickPreset::All => "All",
            QuickPreset::Installed => "Installed",
            QuickPreset::Favorites => "Favourites",
            QuickPreset::Recent => "Recent",
            QuickPreset::MostPlayed => "Most played",
        }
    }
    pub fn tip(self) -> &'static str {
        match self {
            QuickPreset::All => "Every game",
            QuickPreset::Installed => "Only installed games",
            QuickPreset::Favorites => "Only favourites",
            QuickPreset::Recent => "Recently played, newest first",
            QuickPreset::MostPlayed => "Played games, most play time first",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesktopConfig {
    pub view: DesktopView,
    pub sort: SortBy,
    pub sort_descending: bool,
    /// Cover width in the grid view, logical px.
    pub cover_width: f32,
    pub show_sidebar: bool,
    pub show_details_panel: bool,
    /// Old "Installed" toggle; kept in sync with `preset` (true = `QuickPreset::Installed`).
    pub installed_only: bool,
    pub show_hidden: bool,
    /// Quick preset next to the search box.
    pub preset: QuickPreset,
    /// Group headers in every view.
    pub group_by: GroupBy,
    /// Collapsed groups, as "<group by key>:<group key>".
    pub collapsed_groups: Vec<String>,
    /// Blurred game background behind the whole window: None = as the theme says.
    pub background_image: Option<bool>,
    /// Small gamepad badge on covers of games with full controller support.
    pub controller_badge: bool,
    /// The game's logo, or its name when it has no logo, over the background at the top of the
    /// Details view's game page.
    pub show_page_logo: bool,
    /// Settings, Add-ons, Edit game details, Emulated games… and the artwork editor open in their
    /// own windows (off: drawn inside the main window, as before).
    pub own_windows: bool,
    /// Those windows' and the main window's (`main`) last size and position, by window key
    /// ("main", "settings", "addons", "edit", …).
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub windows: std::collections::BTreeMap<String, WindowGeom>,
}

/// A window's size (logical px) and, where the session tells (X11), its position.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowGeom {
    pub w: f32,
    pub h: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<f32>,
    /// Maximised (`w` / `h` are then the size from before).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub maximized: bool,
    /// The desktop's scale (pixels per logical px) when it was saved: X11 monitors are read in
    /// pixels, the place above is logical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
}

impl Default for DesktopConfig {
    fn default() -> Self {
        DesktopConfig {
            view: DesktopView::Grid,
            sort: SortBy::Name,
            sort_descending: false,
            cover_width: 170.0,
            show_sidebar: true,
            show_details_panel: true,
            installed_only: false,
            show_hidden: false,
            preset: QuickPreset::All,
            group_by: GroupBy::None,
            collapsed_groups: vec![],
            background_image: None,
            controller_badge: true,
            show_page_logo: true,
            own_windows: true,
            windows: Default::default(),
        }
    }
}
