//! Text for `{DynamicResource LOCxxx}`: a theme's own strings (`Localization/en_US.xaml`, or
//! a `LOC…` string it defines elsewhere, e.g. in Constants.xaml where theme options can change
//! it), then Lodestar's English for Playnite's standard keys the fullscreen views use (Play,
//! Install, Time played …). Playnite's own string tables aren't copied: these are Lodestar's
//! words for the same things.
//!
//! CONTRACT:
//!   /// Lodestar's English for a standard Playnite key ("LOCPlayGame" or "PlayGame").
//!   pub fn standard(key: &str) -> Option<&'static str>
//!   pub(crate) struct Strings  — one theme's strings (`Strings::load`, `own`), for Lodestar's
//!                                layouts (fill.rs)
//!   pub(crate) struct ViewStrings — the strings of the translated views (compile.rs), with
//!                                Playnite's own English as the fallback

use super::source::Source;
use std::collections::HashMap;

/// Biggest localisation file read (Aniki ReMake's are ~300 KB).
const LIMIT: u64 = 2 * 1024 * 1024;

/// Lodestar's English for Playnite's standard LOC keys (fullscreen views, menus, game facts).
const STANDARD: &[(&str, &str)] = &[
    // Game actions.
    ("PlayGame", "Play"),
    ("InstallGame", "Install"),
    ("UninstallGame", "Uninstall"),
    ("GameOptions", "Options"),
    ("GameDetails", "Details"),
    ("GameIsUnInstalledTitle", "Not installed"),
    ("GameIsInstalledTitle", "Installed"),
    ("GameLaunching", "Launching"),
    ("GameRunning", "Running"),
    ("PLAYING", "PLAYING"),
    ("Playing", "Playing"),
    ("Uninstalling", "Uninstalling"),
    ("Installing", "Installing"),
    ("OpenGameLocation", "Open install folder"),
    ("AddFavorites", "Add to favourites"),
    ("RemoveFavorites", "Remove from favourites"),
    ("EditGame", "Edit"),
    ("MenuStartGame", "Play"),
    // Game facts.
    ("TimePlayed", "Time played"),
    ("GameLastActivityTitle", "Last played"),
    ("LastPlayed", "Last played"),
    ("GameReleaseDateTitle", "Release date"),
    ("GameReleaseYearTitle", "Release year"),
    ("GamePlatformTitle", "Platform"),
    ("PlatformTitle", "Platform"),
    ("PlatformsTitle", "Platforms"),
    ("GameGenresTitle", "Genres"),
    ("GenreLabel", "Genre"),
    ("GenresLabel", "Genres"),
    ("GameDevelopersTitle", "Developers"),
    ("DeveloperLabel", "Developer"),
    ("DevelopersLabel", "Developers"),
    ("GamePublishersTitle", "Publishers"),
    ("PublisherLabel", "Publisher"),
    ("PublishersLabel", "Publishers"),
    ("GameFeaturesTitle", "Features"),
    ("FeatureLabel", "Feature"),
    ("FeaturesLabel", "Features"),
    ("GameTagsTitle", "Tags"),
    ("TagsLabel", "Tags"),
    ("GameCategoriesTitle", "Categories"),
    ("CategoriesLabel", "Categories"),
    ("GameSourceTitle", "Source"),
    ("SourceLabel", "Source"),
    ("GameLibraryTitle", "Library"),
    ("Library", "Library"),
    ("LibraryLabel", "Library"),
    ("SeriesLabel", "Series"),
    ("GameSeriesTitle", "Series"),
    ("AgeRatingLabel", "Age rating"),
    ("RegionLabel", "Region"),
    ("VersionLabel", "Version"),
    ("NotesLabel", "Notes"),
    ("LinksLabel", "Links"),
    ("DescriptionLabel", "Description"),
    ("GameDescriptionTitle", "Description"),
    ("GameInstallDirTitle", "Install folder"),
    ("InstallSizeLabel", "Install size"),
    ("GameInstallSizeTitle", "Install size"),
    ("GameAddedTitle", "Added"),
    ("DateAddedLabel", "Added"),
    ("GameModifiedTitle", "Modified"),
    ("PlayCountLabel", "Play count"),
    ("GamePlayCountTitle", "Play count"),
    ("CompletionStatus", "Completion status"),
    ("CompletionStatusLabel", "Completion status"),
    ("GameCompletionStatusTitle", "Completion status"),
    ("CompletedGamesTitle", "Completed"),
    ("GameFavoriteTitle", "Favourite"),
    ("FavoriteGame", "Favourite"),
    ("GameHiddenTitle", "Hidden"),
    ("CommunityScore", "Community score"),
    ("CommunityScoreTitle", "Community score"),
    ("CriticScore", "Critic score"),
    ("CriticScoreTitle", "Critic score"),
    ("UserScore", "User score"),
    ("UserScoreTitle", "User score"),
    ("GameScoreTitle", "Score"),
    ("StatsTotalPlayTime", "Total time played"),
    ("Never", "Never"),
    ("PlayedNone", "Not played"),
    ("PlayedNoneString", "Not played"),
    ("PlayedSeconds", "{0} seconds"),
    ("PlayedMinutes", "{0} minutes"),
    ("PlayedHours", "{0}h {1}m"),
    ("PlayedDays", "{0}d {1}h {2}m"),
    ("Yes", "Yes"),
    ("No", "No"),
    ("None", "None"),
    // Completion statuses.
    ("CompletionStatusNotPlayed", "Not played"),
    ("CompletionStatusPlayed", "Played"),
    ("CompletionStatusBeaten", "Beaten"),
    ("CompletionStatusCompleted", "Completed"),
    ("CompletionStatusPlaying", "Playing"),
    ("CompletionStatusAbandoned", "Abandoned"),
    ("CompletionStatusOnHold", "On hold"),
    ("CompletionStatusPlanToPlay", "Plan to play"),
    // Lists, filters, search.
    ("AllGames", "All games"),
    ("All", "All"),
    ("Default", "Default"),
    ("Filter", "Filter"),
    ("Filters", "Filters"),
    ("FilterActive", "Filter active"),
    ("Search", "Search"),
    ("SearchLabel", "Search"),
    ("SearchGames", "Search games"),
    ("NoGamesFound", "No games found"),
    ("GameListSearchResults", "Search results"),
    ("GameListExtraFiltered", "Filtered"),
    ("Sortby", "Sort by"),
    ("SortByLabel", "Sort by"),
    ("GroupByLabel", "Group by"),
    ("Direction", "Direction"),
    ("Ascending", "Ascending"),
    ("Descending", "Descending"),
    ("Newest", "Newest"),
    ("Oldest", "Oldest"),
    ("Show", "Show"),
    ("Hide", "Hide"),
    ("Name", "Name"),
    ("NameLabel", "Name"),
    ("ClearLabel", "Clear"),
    ("ClearFilters", "Clear filters"),
    ("Loading", "Loading"),
    ("Games", "Games"),
    ("GamesLabel", "Games"),
    ("RecentlyPlayed", "Recently played"),
    ("Favorites", "Favourites"),
    ("Installed", "Installed"),
    ("Uninstalled", "Not installed"),
    // Buttons and prompts.
    ("BackLabel", "Back"),
    ("OKLabel", "OK"),
    ("CancelLabel", "Cancel"),
    ("CloseLabel", "Close"),
    ("SaveLabel", "Save"),
    ("YesLabel", "Yes"),
    ("NoLabel", "No"),
    ("ConfirmLabel", "Confirm"),
    ("SelectLabel", "Select"),
    ("ButtonPrompt_Select", "Select"),
    ("ButtonPrompt_Navigate", "Navigate"),
    ("ButtonPrompt_Back", "Back"),
    ("ButtonPrompt_Details", "Details"),
    ("ButtonPrompt_Options", "Options"),
    ("ButtonPrompt_Search", "Search"),
    ("ButtonPrompt_Filter", "Filter"),
    ("ButtonPrompt_Menu", "Menu"),
    ("Hold", "Hold"),
    ("SwitchSection", "Switch section"),
    // Menus.
    ("MainMenu", "Menu"),
    ("MenuHelpTitle", "Help"),
    ("MenuTools", "Tools"),
    ("MenuSelectRandomGame", "Random game"),
    ("MenuReloadLibrary", "Update library"),
    ("MenuCancelLibraryUpdate", "Cancel library update"),
    ("MenuOpenClient", "Open client"),
    ("MenuShutdownSystem", "Shut down"),
    ("MenuRestartSystem", "Restart"),
    ("MenuSuspendSystem", "Sleep"),
    ("MenuHibernateSystem", "Hibernate"),
    ("MenuLockSystem", "Lock"),
    ("MenuLogoutUser", "Log out"),
    ("MenuIssues", "Report a problem"),
    ("MenuPatreonSupport", "Support"),
    ("MenuKofiSupport", "Support"),
    ("Power", "Power"),
    ("ExitPlaynite", "Quit"),
    ("ExitAppLabel", "Quit"),
    ("Minimize", "Minimise"),
    ("BackToDesktopMode", "Desktop mode"),
    ("SettingsLabel", "Settings"),
    ("Settings", "Settings"),
    ("SettingsFSSection", "Settings"),
    ("SettingsFSSectionGeneral", "General"),
    ("SettingsFSSectionVisuals", "Visuals"),
    ("SettingsFSSectionAudio", "Audio"),
    ("SettingsFSSectionLayout", "Layout"),
    ("SettingsFSSectionMenus", "Menus"),
    ("SettingsGameDetailsLabel", "Game details"),
    ("Extensions", "Add-ons"),
    ("ExtensionsInstalled", "Installed add-ons"),
    ("Notifications", "Notifications"),
    ("NotificationsLabel", "Notifications"),
    ("CrashRestartPlaynite", "Restart"),
    ("CrashRestartSafe", "Restart in safe mode"),
    ("UpdateAvailable", "Update available"),
    ("Achievements", "Achievements"),
];

fn bare(key: &str) -> &str {
    let k = key.trim().trim_matches(|c| c == '{' || c == '}').trim();
    let k = k.strip_prefix("DynamicResource").or_else(|| k.strip_prefix("StaticResource")).map(str::trim).unwrap_or(k);
    k.strip_prefix("LOC").unwrap_or(k)
}

/// Lodestar's English for a standard Playnite key ("LOCPlayGame" or "PlayGame").
pub fn standard(key: &str) -> Option<&'static str> {
    let k = bare(key);
    STANDARD.iter().find(|(s, _)| *s == k).map(|(_, v)| *v)
}

/// One theme's own strings (LOC keys → text).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Strings {
    own: HashMap<String, String>,
}

impl Strings {
    /// The theme's English localisation (`Localization/en_US.xaml`, else another English one,
    /// e.g. `en_EN.xaml`), then `LOC…` strings in its other XAML files (Constants.xaml, theme
    /// options), which win as Playnite's later dictionaries do.
    pub fn load(source: &Source) -> Strings {
        let files = source.files();
        let mut own = HashMap::new();
        let loc: Vec<&String> = files
            .iter()
            .map(|(f, _)| f)
            .filter(|f| {
                let l = f.to_ascii_lowercase();
                l.starts_with("localization/") && l.ends_with(".xaml")
            })
            .collect();
        let pick = loc.iter().find(|f| f.to_ascii_lowercase().ends_with("/en_us.xaml")).or_else(|| {
            loc.iter().find(|f| f.rsplit('/').next().is_some_and(|b| b.to_ascii_lowercase().starts_with("en")))
        });
        if let Some(f) = pick
            && let Some(t) = source.read_text(f, LIMIT)
        {
            own.extend(strings_of(&t));
        }
        for (f, size) in &files {
            let l = f.to_ascii_lowercase();
            if l.ends_with(".xaml")
                && !l.starts_with("localization/")
                && *size <= LIMIT
                && l.contains("constants")
                && let Some(t) = source.read_text(f, LIMIT)
            {
                own.extend(strings_of(&t));
            }
        }
        Strings { own }
    }

    /// Strings from XAML texts given directly (theme option overrides, tests).
    pub fn extend_from_xaml(&mut self, text: &str) {
        self.own.extend(strings_of(text));
    }

    /// The theme's own text for `key`, else Lodestar's English.
    #[cfg(test)]
    pub fn get(&self, key: &str) -> Option<String> {
        let k = bare(key);
        self.own.get(k).cloned().or_else(|| standard(k).map(str::to_string))
    }

    /// The theme's own text only.
    pub fn own(&self, key: &str) -> Option<&str> {
        self.own.get(bare(key)).map(String::as_str)
    }
}

/// `<sys:String x:Key="LOCx">text</sys:String>` entries of a XAML text (keys without "LOC").
fn strings_of(text: &str) -> Vec<(String, String)> {
    let opts = roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: 400_000, ..Default::default() };
    let Ok(doc) = roxmltree::Document::parse_with_options(text.trim_start_matches('\u{feff}'), opts) else {
        return vec![];
    };
    let mut out = vec![];
    for n in doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == "String") {
        let key = n.attributes().find(|a| a.name() == "Key").map(|a| a.value().to_string());
        let Some(key) = key.filter(|k| k.starts_with("LOC")) else { continue };
        let value: String = n.children().filter_map(|c| c.text()).collect();
        out.push((key[3..].to_string(), value.trim().to_string()));
    }
    out
}

/// The `LOC…` strings of the translated views (compile.rs): the theme's own (its
/// Localization/en_US.xaml or another English file), then Playnite's own English for the
/// standard keys those views use. Kept apart from [`Strings`]: a translated view reproduces
/// the theme as Playnite shows it (Playnite's wording and capitals, "Time Played"), while
/// Lodestar's layouts use Lodestar's words for the same keys.
#[derive(Default)]
pub(crate) struct ViewStrings {
    own: HashMap<String, String>,
}

impl ViewStrings {
    pub fn load(source: &Source, files: &[(String, u64)]) -> ViewStrings {
        let mut own = HashMap::new();
        let mut english: Vec<&String> = files
            .iter()
            .map(|(f, _)| f)
            .filter(|f| {
                let l = f.to_ascii_lowercase();
                l.starts_with("localization/") && l.ends_with(".xaml") && l.contains("/en")
            })
            .collect();
        // en_US first.
        english.sort_by_key(|f| !f.to_ascii_lowercase().contains("en_us"));
        if let Some(f) = english.first()
            && let Some(text) = source.read_text(f, 1024 * 1024)
            && let Ok(doc) = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        {
            for n in doc.root_element().children().filter(|c| c.is_element()) {
                if let (Some(k), Some(v)) = (super::xaml::key_of(n), n.text()) {
                    own.insert(k, v.trim().to_string());
                }
            }
        }
        ViewStrings { own }
    }

    #[cfg(test)]
    pub fn with(pairs: &[(&str, &str)]) -> ViewStrings {
        ViewStrings { own: pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    pub fn text(&self, key: &str) -> Option<String> {
        if let Some(v) = self.own.get(key) {
            return Some(v.clone());
        }
        playnite_english(key).map(str::to_string)
    }
}

/// Playnite's English for the standard strings its views use.
fn playnite_english(key: &str) -> Option<&'static str> {
    Some(match key {
        "LOCTimePlayed" => "Time Played",
        "LOCGameLastActivityTitle" | "LOCLastPlayed" => "Last Played",
        "LOCGameReleaseDateTitle" | "LOCReleaseDate" => "Release Date",
        "LOCGamePlatformTitle" | "LOCPlatformTitle" | "LOCPlatform" => "Platform",
        "LOCPlayGame" => "Play",
        "LOCInstallGame" => "Install",
        "LOCGameDetails" | "LOCSettingsGameDetailsLabel" => "Game Details",
        "LOCGameOptions" => "Game Options",
        "LOCSearchLabel" => "Search",
        "LOCFilter" | "LOCFilters" => "Filter",
        "LOCNoGamesFound" => "No games found",
        "LOCAllGames" => "All Games",
        "LOCBackLabel" => "Back",
        "LOCGameFavoriteTitle" | "LOCFavorite" => "Favorite",
        "LOCCompletionStatus" => "Completion Status",
        "LOCLibrary" => "Library",
        "LOCGenresLabel" | "LOCGenreLabel" => "Genres",
        "LOCDeveloperLabel" | "LOCDevelopersLabel" => "Developers",
        "LOCPublishersLabel" | "LOCPublisherLabel" => "Publishers",
        "LOCSourceLabel" => "Source",
        "LOCSeriesLabel" => "Series",
        "LOCTagsLabel" => "Tags",
        "LOCFeaturesLabel" => "Features",
        "LOCUserScore" => "User Score",
        "LOCCriticScore" => "Critic Score",
        "LOCCommunityScore" => "Community Score",
        "LOCPlayCountLabel" => "Play Count",
        "LOCInstallSize" => "Install Size",
        "LOCAddedLabel" => "Added",
        "LOCModifiedLabel" => "Modified",
        "LOCSettingsLabel" => "Settings",
        "LOCMainMenu" => "Main Menu",
        "LOCMenuSortByTitle" | "LOCSortby" => "Sort by",
        "LOCQuickFilterRecentlyPlayed" => "Recently Played",
        "LOCDescriptionLabel" => "Description",
        "LOCGameListSearchResults" => "Showing search results for:",
        "LOCGameListExtraFiltered" => "Additional filters are active",
        "LOCNameLabel" => "Name",
        "LOCLibraries" => "Libraries",
        "LOCGameProviderTitle" => "Library",
        "LOCGameInstallationStatus" => "Installation Status",
        "LOCInstallSizeLabel" => "Install Size",
        "LOCPlatformsTitle" => "Platforms",
        "LOCGamePublishersTitle" => "Publishers",
        "LOCGameDevelopersTitle" => "Developers",
        "LOCGameCategoriesTitle" | "LOCCategoriesLabel" => "Categories",
        "LOCGameReleaseYearTitle" => "Release Year",
        "LOCGameDescriptionTitle" => "Description",
        "LOCAgeRatingLabel" => "Age Rating",
        "LOCAgeRatingsLabel" => "Age Ratings",
        "LOCRegionLabel" => "Region",
        "LOCRegionsLabel" => "Regions",
        "LOCVersionLabel" => "Version",
        "LOCNotesLabel" => "Notes",
        "LOCLinksLabel" => "Links",
        "LOCMediaLabel" => "Media",
        "LOCDateAddedLabel" => "Date Added",
        "LOCSelect" => "Select",
        "LOCOKLabel" => "OK",
        "LOCCancelLabel" => "Cancel",
        "LOCClearLabel" => "Clear",
        "LOCClose" | "LOCCloseLabel" => "Close",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EN: &str = r#"<ResourceDictionary xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" xmlns:sys="clr-namespace:System;assembly=mscorlib">
        <sys:String x:Key="LOCMyTheme_Recent">Recently played</sys:String>
        <sys:String x:Key="LOCPlayGame">Start</sys:String>
        <sys:String x:Key="NotALocKey">x</sys:String>
    </ResourceDictionary>"#;

    fn theme(files: &[(&str, &str)]) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for (f, t) in files {
            let p = d.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
        d
    }

    #[test]
    fn standard_keys_have_lodestar_english() {
        for (k, want) in [
            ("LOCPlayGame", "Play"),
            ("InstallGame", "Install"),
            ("{DynamicResource LOCTimePlayed}", "Time played"),
            ("LOCGameLastActivityTitle", "Last played"),
            ("LOCGamePlatformTitle", "Platform"),
            ("LOCBackLabel", "Back"),
            ("LOCNoGamesFound", "No games found"),
        ] {
            assert_eq!(standard(k), Some(want), "{k}");
        }
        assert_eq!(standard("LOCSomeAddon_Thing"), None);
        // No key twice.
        let mut seen = std::collections::HashSet::new();
        for (k, _) in STANDARD {
            assert!(seen.insert(*k), "{k} twice");
        }
    }

    #[test]
    fn theme_strings_win_then_english() {
        let d = theme(&[
            ("theme.yaml", "Name: T"),
            ("Localization/en_US.xaml", EN),
            ("Localization/de_DE.xaml", EN.replace("Start", "Spielen").as_str()),
        ]);
        let s = Strings::load(&Source::open(d.path()).unwrap());
        assert_eq!(s.get("LOCMyTheme_Recent").as_deref(), Some("Recently played"));
        assert_eq!(s.get("LOCPlayGame").as_deref(), Some("Start"));
        assert_eq!(s.get("LOCInstallGame").as_deref(), Some("Install"));
        assert_eq!(s.get("LOCNotThere"), None);
        assert_eq!(s.get("NotALocKey"), None);
        assert_eq!(s.get("LOCTimePlayed").as_deref(), Some("Time played"));
    }

    #[test]
    fn views_use_the_themes_strings_then_playnites_english() {
        let s = ViewStrings::with(&[("LOCPlayGame", "Start")]);
        assert_eq!(s.text("LOCPlayGame").as_deref(), Some("Start"));
        assert_eq!(s.text("LOCInstallSizeLabel").as_deref(), Some("Install Size"));
        assert_eq!(s.text("LOCCloseLabel").as_deref(), Some("Close"));
        assert_eq!(s.text("LOCNotThere"), None);
    }

    #[test]
    fn other_english_files_and_constants() {
        let constants = r#"<ResourceDictionary xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
            xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" xmlns:sys="clr-namespace:System;assembly=mscorlib">
            <sys:String x:Key="LOCPlayGame">Launch</sys:String></ResourceDictionary>"#;
        let d = theme(&[("theme.yaml", "Name: T"), ("Localization/en_EN.xaml", EN), ("Constants.xaml", constants)]);
        let mut s = Strings::load(&Source::open(d.path()).unwrap());
        assert_eq!(s.get("LOCMyTheme_Recent").as_deref(), Some("Recently played"));
        // Constants (where theme options live) win over the localisation file.
        assert_eq!(s.get("LOCPlayGame").as_deref(), Some("Launch"));
        s.extend_from_xaml(&constants.replace("Launch", "Go"));
        assert_eq!(s.get("PlayGame").as_deref(), Some("Go"));
    }
}
