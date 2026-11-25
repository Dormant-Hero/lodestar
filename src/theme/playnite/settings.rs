//! What a theme's `{Settings …}`, `{PluginSettings …}` and `{PluginStatus …}` read when its
//! views are translated: Playnite's own settings at their defaults (the way themes are shown in
//! their screenshots), and the add-ons whose features Lodestar provides itself (game logos,
//! theme options) as installed and on. Every other add-on is missing: its settings are unset,
//! its status "not installed" — what themes are made to handle.
//!
//! CONTRACT:
//!   pub(crate) enum SVal { Bool, Num, Text }
//!   /// `{Settings Fullscreen.Rows}` → its default (None: not a setting the views read).
//!   pub(crate) fn setting(path: &str) -> Option<SVal>
//!   /// Playnite settings that are the player's choice, offered as theme options.
//!   pub(crate) const PLAYER_SETTINGS: &[PlayerSetting]  — key, Playnite's name, Lodestar's label
//!   pub(crate) fn player_setting(key) -> Option<&PlayerSetting>
//!   /// The player's values for those over the defaults; `get` / `value` read a `{Settings …}`.
//!   pub(crate) struct PlayerSettings
//!   /// `{PluginSettings …}` / `{PluginStatus …}` (the markup's inner text, with its keyword).
//!   /// ThemeOptions' variables at their defaults (options.yaml).
//!   pub(crate) struct AddOnOptions; AddOnOptions::parse(yaml)
//!   pub(crate) fn plugin(raw: &str, options: &AddOnOptions) -> Plugin  — Value / PerGame("{art.logo}") / Fallback / Missing
//!   /// Its Converter / ConverterParameter (applied to a provided value).
//!   /// A provided value through the markup's Converter.
//!   pub(crate) fn plugin_text(raw: &str, v: &SVal) -> Option<String>

use std::collections::BTreeMap;

/// A setting's value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SVal {
    Bool(bool),
    Num(f32),
    Text(String),
}

impl SVal {
    /// As a binding's raw string (convert.rs: "True" / "False", invariant numbers).
    pub fn text(&self) -> String {
        match self {
            SVal::Bool(true) => "True".into(),
            SVal::Bool(false) => "False".into(),
            SVal::Num(n) => n.to_string(),
            SVal::Text(t) => t.clone(),
        }
    }
}

/// Playnite's settings at their defaults: the fullscreen ones (`Fullscreen.X`) and the desktop
/// grid's. `ShowNamesUnderCovers` is the one exception: Lodestar's own "Show game names" switch
/// (on until the user turns it off) stands for it, so a theme's names are kept at import and
/// that switch shows or hides them.
pub(crate) fn setting(path: &str) -> Option<SVal> {
    Some(match setting_name(path) {
        "ShowNamesUnderCovers"
        | "ShowClock"
        | "ShowBattery"
        | "ShowBatteryPercentage"
        | "EnableMainBackgroundImage"
        | "EnableGameTitles"
        | "HorizontalLayout"
        | "SmoothScrolling" => SVal::Bool(true),
        "ShowGameTitles"
        | "ShowGridItemBackground"
        | "DarkenUninstalledGamesGrid"
        | "UsePrimaryDisplay"
        | "SwapStartDetailsAction" => SVal::Bool(false),
        "Rows" => SVal::Num(ROWS),
        "Columns" => SVal::Num(COLUMNS),
        "FontSize" => SVal::Num(22.0),
        "FontSizeSmall" => SVal::Num(18.0),
        "MainBackgroundImageBlurAmount" => SVal::Num(0.0),
        "MainBackgroundImageDarkAmount" => SVal::Num(40.0),
        "FullscreenItemSpacing" => SVal::Num(ITEM_SPACING),
        // Playnite's cover aspect choice: 3 = 2:3 portrait covers (the default).
        "GridItemHeightRatio" => SVal::Num(3.0),
        "CoverArtStretch" => SVal::Text("UniformToFill".into()),
        _ => return None,
    })
}

/// The setting a `{Settings …}` markup reads, without `Path=`, the binding's other parts and
/// the `Fullscreen.` prefix ("Fullscreen.Rows, Mode=OneWay" → "Rows").
fn setting_name(path: &str) -> &str {
    let p = path.trim().trim_start_matches("Path=").trim();
    let p = p.split(',').next().unwrap_or(p).trim();
    p.strip_prefix("Fullscreen.").unwrap_or(p)
}

/// A Playnite setting that is its user's choice rather than the theme's.
pub(crate) struct PlayerSetting {
    /// As a `{Settings …}` markup names it.
    pub key: &'static str,
    /// What Playnite's settings call it.
    pub playnite_name: &'static str,
    /// The option's label in Lodestar. Not Playnite's name: Lodestar has a switch of that
    /// name of its own (controller Settings → Visuals), which hides what this one shows.
    pub label: &'static str,
}

/// Playnite settings an imported theme offers as options of its own (options.rs), at
/// Playnite's default until the player changes them. A theme's screenshots show its author's
/// choice, which is no more the theme's look than the default is.
pub(crate) const PLAYER_SETTINGS: &[PlayerSetting] = &[PlayerSetting {
    key: "Fullscreen.ShowGameTitles",
    playnite_name: "Show game titles",
    label: "Theme's game titles (Playnite setting)",
}];

/// The Playnite setting an option stands for (None: one of the theme's own).
pub(crate) fn player_setting(key: &str) -> Option<&'static PlayerSetting> {
    PLAYER_SETTINGS.iter().find(|s| s.key == key)
}

/// The values the player chose for [`PLAYER_SETTINGS`]; every other setting reads its default.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct PlayerSettings(BTreeMap<String, SVal>);

impl PlayerSettings {
    pub fn set(&mut self, path: &str, value: SVal) {
        self.0.insert(setting_name(path).to_string(), value);
    }

    /// What a `{Settings …}` markup reads: the player's value, else the default ([`setting`]).
    pub fn get(&self, path: &str) -> Option<SVal> {
        self.0.get(setting_name(path)).cloned().or_else(|| setting(path))
    }

    /// The markup's value as its binding gives it: through the markup's Converter, if it names
    /// one. None: not a setting the views read.
    pub fn value(&self, raw: &str) -> Option<String> {
        plugin_text(raw, &self.get(raw)?)
    }
}

/// `Fullscreen.Rows` / `Fullscreen.Columns` / `FullscreenItemSpacing` at their defaults: the
/// game list's FullscreenTilePanel sizes its items from them.
pub(crate) const ROWS: f32 = 2.0;
pub(crate) const COLUMNS: f32 = 7.0;
pub(crate) const ITEM_SPACING: f32 = 10.0;

/// A cover's height ÷ width at the `GridItemHeightRatio` above (2:3 portrait covers).
pub(crate) const COVER_ASPECT: f32 = 1.5;

/// What an add-on markup reads.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Plugin {
    /// A fixed value.
    Value(SVal),
    /// True per game when this Lodestar binding resolves ("{art.logo}").
    PerGame(&'static str),
    /// The add-on isn't there and the binding names a FallbackValue: that value, as written
    /// (WPF uses it without the binding's converter).
    Fallback(String),
    /// The add-on isn't there: unset (null) / not installed.
    Missing,
}

/// The add-on and setting a `PluginSettings Plugin=X, Path=Y` / `PluginStatus Plugin=X …` names.
fn parts(raw: &str) -> (bool, String, String, Option<String>) {
    let raw = raw.trim().trim_start_matches('{').trim_end_matches('}').trim();
    let status = raw.starts_with("PluginStatus");
    let (mut plugin, mut path, mut fallback) = (String::new(), String::new(), None);
    for part in raw.split(',') {
        let part = part.trim().trim_start_matches("PluginSettings").trim_start_matches("PluginStatus").trim();
        if let Some((k, v)) = part.split_once('=') {
            match k.trim() {
                "Plugin" => plugin = v.trim().to_string(),
                "Path" => path = v.trim().to_string(),
                "FallbackValue" => fallback = Some(v.trim().trim_matches('\'').to_string()),
                _ => {}
            }
        }
    }
    // Add-on ids carry a GUID suffix ("ThemeOptions_904cbf3b-573f-…").
    if let Some((name, id)) = plugin.split_once('_')
        && id.len() >= 32
        && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
    {
        plugin = name.to_string();
    }
    (status, plugin.to_ascii_lowercase(), path, fallback)
}

/// A theme's ThemeOptions variables at their defaults (the `Variables:` of its options.yaml):
/// what `{PluginSettings Plugin=ThemeOptions, Path=Options[KEY]}` reads until the user changes
/// them in the add-on's settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct AddOnOptions(BTreeMap<String, SVal>);

impl AddOnOptions {
    /// From options.yaml: each variable under `Variables:` with its `Default:` (headers and
    /// variables without a default are left out).
    pub fn parse(yaml: &str) -> AddOnOptions {
        let mut out = BTreeMap::new();
        let (mut in_vars, mut key_indent, mut key) = (false, usize::MAX, None::<String>);
        for line in yaml.lines() {
            let body = line.split(" #").next().unwrap_or(line).trim_end();
            let text = body.trim_start();
            if text.is_empty() || text.starts_with('#') {
                continue;
            }
            let indent = body.len() - text.len();
            if indent == 0 {
                in_vars = text.trim_end_matches(':') == "Variables";
                key_indent = usize::MAX;
                continue;
            }
            if !in_vars {
                continue;
            }
            if indent <= key_indent && text.ends_with(':') {
                key_indent = indent;
                key = Some(text.trim_end_matches(':').trim().to_string());
            } else if let (Some(k), Some(v)) = (&key, text.strip_prefix("Default:")) {
                let v = v.trim().trim_matches(|c| c == '\'' || c == '"');
                let typed = match v.to_ascii_lowercase().as_str() {
                    "true" => SVal::Bool(true),
                    "false" => SVal::Bool(false),
                    _ => v.parse::<f32>().map(SVal::Num).unwrap_or_else(|_| SVal::Text(v.to_string())),
                };
                out.insert(k.clone(), typed);
            }
        }
        AddOnOptions(out)
    }
}

/// The screen shape ThemeOptions reports (`AspectRatio`) for Playnite's 1920 × 1080 views.
const ASPECT_RATIO: &str = "dsp169";

/// See the module docs.
pub(crate) fn plugin(raw: &str, options: &AddOnOptions) -> Plugin {
    let (status, name, path, fallback) = parts(raw);
    let provided = matches!(name.as_str(), "themeoptions" | "thememodifier" | "extrametadataloader");
    if status {
        return if provided { Plugin::Value(SVal::Bool(true)) } else { Plugin::Missing };
    }
    let option = path.strip_prefix("Options[").and_then(|p| p.strip_suffix(']'));
    match (name.as_str(), path.as_str()) {
        ("themeoptions" | "thememodifier", "IsInstalled") => Plugin::Value(SVal::Bool(true)),
        // Lodestar stands in for a current version of the add-on.
        ("themeoptions" | "thememodifier", p) if p.starts_with("MinimalVersion[") => Plugin::Value(SVal::Bool(true)),
        ("themeoptions", "AspectRatio") => Plugin::Value(SVal::Text(ASPECT_RATIO.into())),
        ("themeoptions", _) if let Some(v) = option.and_then(|k| options.0.get(k)) => Plugin::Value(v.clone()),
        // Lodestar finds and draws game logos itself.
        ("extrametadataloader", "EnableLogos") => Plugin::Value(SVal::Bool(true)),
        ("extrametadataloader", "IsLogoAvailable" | "IsAnyLogoAvailable") => Plugin::PerGame("{art.logo}"),
        // Its trailers and video player aren't there.
        ("extrametadataloader", _) if path.contains("Video") || path.contains("Trailer") => {
            Plugin::Value(SVal::Bool(false))
        }
        // A missing add-on's binding gives its FallbackValue.
        _ => match fallback {
            Some(f) => Plugin::Fallback(f),
            None => Plugin::Missing,
        },
    }
}

/// What an add-on markup gives as a binding's raw text, when that is fixed: a provided value
/// through its converter, its FallbackValue, a missing add-on's "not installed" ("False") or
/// unset value (""). None: it changes per game.
pub(crate) fn plugin_value(raw: &str, options: &AddOnOptions) -> Option<String> {
    match plugin(raw, options) {
        Plugin::Value(v) => plugin_text(raw, &v),
        Plugin::Fallback(f) => Some(f),
        Plugin::Missing if raw.trim_start().starts_with("PluginStatus") => Some("False".into()),
        Plugin::Missing => Some(String::new()),
        Plugin::PerGame(_) => None,
    }
}

/// The `Converter=` / `ConverterParameter=` of an add-on or settings markup.
fn converter(raw: &str) -> Option<(String, Option<String>)> {
    let (mut conv, mut param) = (None, None);
    let (mut depth, mut cur, mut parts) = (0i32, String::new(), vec![]);
    for ch in raw.chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(ch);
    }
    parts.push(cur);
    for p in &parts {
        if let Some((k, v)) = p.split_once('=') {
            match k.trim() {
                "Converter" => conv = Some(v.trim().to_string()),
                "ConverterParameter" => param = Some(v.trim().trim_matches('\'').to_string()),
                _ => {}
            }
        }
    }
    conv.map(|c| (c, param))
}

/// A provided add-on or setting value as its binding gives it (through the markup's
/// converter).
pub(crate) fn plugin_text(raw: &str, v: &SVal) -> Option<String> {
    match converter(raw) {
        Some((c, param)) => super::convert::apply(&c, &v.text(), param.as_deref()),
        None => Some(v.text()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_at_defaults(raw: &str) -> Plugin {
        plugin(raw, &AddOnOptions::default())
    }

    #[test]
    fn playnite_defaults() {
        assert_eq!(setting("Fullscreen.HorizontalLayout"), Some(SVal::Bool(true)));
        assert_eq!(setting("Fullscreen.HorizontalLayout, Mode=TwoWay"), Some(SVal::Bool(true)));
        assert_eq!(setting("Path=Fullscreen.Columns"), Some(SVal::Num(7.0)));
        assert_eq!(setting("Fullscreen.Unknown"), None);
        assert_eq!(setting("Fullscreen.ShowGameTitles"), Some(SVal::Bool(false)));
        assert_eq!(
            setting("ShowNamesUnderCovers, Converter={StaticResource BooleanToVisibilityConverter}"),
            Some(SVal::Bool(true))
        );
        let defaults = PlayerSettings::default();
        assert_eq!(
            defaults
                .value("ShowGridItemBackground, Converter={StaticResource InvertedBooleanToVisibilityConverter}")
                .as_deref(),
            Some("Visible")
        );
        assert_eq!(defaults.value("Fullscreen.ShowGameTitles").as_deref(), Some("False"));
        assert_eq!(SVal::Bool(false).text(), "False");
        assert_eq!(SVal::Num(2.0).text(), "2");
    }

    #[test]
    fn the_players_choice_stands_over_the_default() {
        let mut chosen = PlayerSettings::default();
        chosen.set("Fullscreen.ShowGameTitles", SVal::Bool(true));
        assert_eq!(chosen.get("Path=Fullscreen.ShowGameTitles, Mode=OneWay"), Some(SVal::Bool(true)));
        let raw = "Fullscreen.ShowGameTitles, Converter={StaticResource BooleanToVisibilityConverter}";
        assert_eq!(chosen.value(raw).as_deref(), Some("Visible"));
        assert_eq!(chosen.get("Fullscreen.Rows"), Some(SVal::Num(ROWS)));
    }

    #[test]
    fn add_ons_lodestar_provides() {
        assert_eq!(
            plugin_at_defaults(
                "PluginStatus Plugin=ThemeOptions_904cbf3b-573f-48f8-9642-0a09d05c64ef, Status=Installed"
            ),
            Plugin::Value(SVal::Bool(true))
        );
        assert_eq!(plugin_at_defaults("PluginStatus Plugin=SuccessStory, Status=Installed"), Plugin::Missing);
        assert_eq!(
            plugin_at_defaults("PluginSettings Plugin=ExtraMetadataLoader, Path=IsLogoAvailable, FallbackValue=False"),
            Plugin::PerGame("{art.logo}")
        );
        assert_eq!(
            plugin_at_defaults("PluginSettings Plugin=ExtraMetadataLoader, Path=EnableLogos"),
            Plugin::Value(SVal::Bool(true))
        );
        assert_eq!(
            plugin_at_defaults("PluginSettings Plugin=ExtraMetadataLoader, Path=IsAnyVideoAvailable"),
            Plugin::Value(SVal::Bool(false))
        );
        assert_eq!(plugin_at_defaults("PluginSettings Plugin=ThemeOptions, Path=Options[TO_Avatar]"), Plugin::Missing);
        let raw = "PluginStatus Plugin=ThemeOptions, Status=Installed, Converter={StaticResource InvertedBooleanToVisibilityConverter}";
        assert_eq!(plugin_text(raw, &SVal::Bool(true)).as_deref(), Some("Collapsed"));
        assert_eq!(
            plugin_at_defaults(
                "PluginSettings Plugin=SteamScreenshots, Path=IsControlVisible, FallbackValue=PluginUnavailable"
            ),
            Plugin::Fallback("PluginUnavailable".into())
        );
    }

    #[test]
    fn theme_options_read_the_themes_variables_at_their_defaults() {
        let yaml = "Presets:\n  Look:\n    Name: Look\n    Presets:\n      Default:\n        Name: Plain\nVariables:\n  Header_:\n    Title: Startup\n    Type: Header\n  TO_Intro:\n    Title: Show the intro\n    Type: Boolean\n    Default: True # comment\n  TO_Scale:\n    Type: Double\n    Default: 1.25\n";
        let o = AddOnOptions::parse(yaml);
        let read = |p: &str| plugin(&format!("PluginSettings Plugin=ThemeOptions, Path={p}"), &o);
        assert_eq!(read("Options[TO_Intro]"), Plugin::Value(SVal::Bool(true)));
        assert_eq!(read("Options[TO_Scale]"), Plugin::Value(SVal::Num(1.25)));
        assert_eq!(read("Options[Header_]"), Plugin::Missing);
        assert_eq!(read("Options[Unknown]"), Plugin::Missing);
        assert_eq!(read("MinimalVersion[0.23]"), Plugin::Value(SVal::Bool(true)));
        assert_eq!(read("AspectRatio"), Plugin::Value(SVal::Text("dsp169".into())));
    }
}
