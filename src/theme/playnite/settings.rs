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
//!   /// `{PluginSettings …}` / `{PluginStatus …}` (the markup's inner text, with its keyword).
//!   pub(crate) fn plugin(raw: &str) -> Plugin  — Value / PerGame("{art.logo}") / Fallback / Missing
//!   /// Its Converter / ConverterParameter (applied to a provided value).
//!   /// A provided value through the markup's Converter.
//!   pub(crate) fn plugin_text(raw: &str, v: &SVal) -> Option<String>

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

/// Playnite's fullscreen settings at their defaults.
pub(crate) fn setting(path: &str) -> Option<SVal> {
    let p = path.trim().trim_start_matches("Path=").trim();
    let p = p.split(',').next().unwrap_or(p).trim();
    let p = p.strip_prefix("Fullscreen.").unwrap_or(p);
    Some(match p {
        "ShowGameTitles"
        | "ShowClock"
        | "ShowBattery"
        | "ShowBatteryPercentage"
        | "EnableMainBackgroundImage"
        | "EnableGameTitles"
        | "HorizontalLayout"
        | "SmoothScrolling" => SVal::Bool(true),
        "DarkenUninstalledGamesGrid" | "UsePrimaryDisplay" | "SwapStartDetailsAction" => SVal::Bool(false),
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

/// `Fullscreen.Rows` / `Fullscreen.Columns` / `FullscreenItemSpacing` at their defaults: the
/// game list's FullscreenTilePanel sizes its items from them.
pub(crate) const ROWS: f32 = 2.0;
pub(crate) const COLUMNS: f32 = 7.0;
pub(crate) const ITEM_SPACING: f32 = 10.0;

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

/// See the module docs.
pub(crate) fn plugin(raw: &str) -> Plugin {
    let (status, name, path, fallback) = parts(raw);
    let provided = matches!(name.as_str(), "themeoptions" | "thememodifier" | "extrametadataloader");
    if status {
        return if provided { Plugin::Value(SVal::Bool(true)) } else { Plugin::Missing };
    }
    match (name.as_str(), path.as_str()) {
        ("themeoptions" | "thememodifier", "IsInstalled") => Plugin::Value(SVal::Bool(true)),
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

/// The `Converter=` / `ConverterParameter=` of an add-on markup.
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

/// A provided add-on value as its binding gives it (through the markup's converter).
pub(crate) fn plugin_text(raw: &str, v: &SVal) -> Option<String> {
    match converter(raw) {
        Some((c, param)) => super::convert::apply(&c, &v.text(), param.as_deref()),
        None => Some(v.text()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playnite_defaults() {
        assert_eq!(setting("Fullscreen.HorizontalLayout"), Some(SVal::Bool(true)));
        assert_eq!(setting("Fullscreen.HorizontalLayout, Mode=TwoWay"), Some(SVal::Bool(true)));
        assert_eq!(setting("Path=Fullscreen.Columns"), Some(SVal::Num(7.0)));
        assert_eq!(setting("Fullscreen.Unknown"), None);
        assert_eq!(SVal::Bool(false).text(), "False");
        assert_eq!(SVal::Num(2.0).text(), "2");
    }

    #[test]
    fn add_ons_lodestar_provides() {
        assert_eq!(
            plugin("PluginStatus Plugin=ThemeOptions_904cbf3b-573f-48f8-9642-0a09d05c64ef, Status=Installed"),
            Plugin::Value(SVal::Bool(true))
        );
        assert_eq!(plugin("PluginStatus Plugin=SuccessStory, Status=Installed"), Plugin::Missing);
        assert_eq!(
            plugin("PluginSettings Plugin=ExtraMetadataLoader, Path=IsLogoAvailable, FallbackValue=False"),
            Plugin::PerGame("{art.logo}")
        );
        assert_eq!(
            plugin("PluginSettings Plugin=ExtraMetadataLoader, Path=EnableLogos"),
            Plugin::Value(SVal::Bool(true))
        );
        assert_eq!(
            plugin("PluginSettings Plugin=ExtraMetadataLoader, Path=IsAnyVideoAvailable"),
            Plugin::Value(SVal::Bool(false))
        );
        assert_eq!(plugin("PluginSettings Plugin=ThemeOptions, Path=Options[TO_Avatar]"), Plugin::Missing);
        let raw = "PluginStatus Plugin=ThemeOptions, Status=Installed, Converter={StaticResource InvertedBooleanToVisibilityConverter}";
        assert_eq!(plugin_text(raw, &SVal::Bool(true)).as_deref(), Some("Collapsed"));
        assert_eq!(
            plugin("PluginSettings Plugin=SteamScreenshots, Path=IsControlVisible, FallbackValue=PluginUnavailable"),
            Plugin::Fallback("PluginUnavailable".into())
        );
    }
}
