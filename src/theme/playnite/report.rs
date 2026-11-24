//! The import report, grouped by area (Colours, Fonts, Layout, Covers, Background, Videos and
//! sounds, Not possible), in plain words.

use serde::{Deserialize, Serialize};

/// What part of the look a report line is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Area {
    Colours,
    Fonts,
    Layout,
    Covers,
    Background,
    Media,
    Options,
    NotPossible,
}

impl Area {
    pub const ALL: [Area; 8] = [
        Area::Colours,
        Area::Fonts,
        Area::Layout,
        Area::Covers,
        Area::Background,
        Area::Media,
        Area::Options,
        Area::NotPossible,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Area::Colours => "Colours",
            Area::Fonts => "Fonts",
            Area::Layout => "Layout",
            Area::Covers => "Covers",
            Area::Background => "Background",
            Area::Media => "Videos and sounds",
            Area::Options => "Theme options",
            Area::NotPossible => "Not possible",
        }
    }
}

/// One line: carried over (`ok`) or not.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub ok: bool,
    pub text: String,
}

/// The lines of one area.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub title: String,
    pub lines: Vec<Line>,
}

/// Lines collected while importing, grouped at the end.
#[derive(Default)]
pub(crate) struct Builder {
    lines: Vec<(Area, bool, String)>,
}

impl Builder {
    pub fn ok(&mut self, a: Area, s: impl Into<String>) {
        self.lines.push((a, true, s.into()));
    }

    pub fn no(&mut self, a: Area, s: impl Into<String>) {
        self.lines.push((a, false, s.into()));
    }

    pub fn extend(&mut self, lines: Vec<(Area, bool, String)>) {
        self.lines.extend(lines);
    }

    /// The groups in area order (carried lines first in each), empty areas left out.
    pub fn groups(&self) -> Vec<Group> {
        Area::ALL
            .iter()
            .filter_map(|a| {
                let mut lines: Vec<Line> = self
                    .lines
                    .iter()
                    .filter(|(x, _, _)| x == a)
                    .map(|(_, ok, text)| Line { ok: *ok, text: text.clone() })
                    .collect();
                lines.sort_by_key(|l| !l.ok);
                lines.dedup();
                (!lines.is_empty()).then(|| Group { title: a.title().into(), lines })
            })
            .collect()
    }
}

/// An add-on's display name from the id a theme uses for it.
fn addon_display_name(id: &str) -> &str {
    match id {
        "ExtraMetadataLoader" | "ExtraMetadata" => "Extra Metadata Loader",
        "ThemeExtras" => "Theme Extras",
        "PlayniteAchievements" => "Playnite Achievements",
        "HowLongToBeat" | "HowLongToBeatPlugin" => "HowLongToBeat",
        "SuccessStory" => "SuccessStory",
        _ => id,
    }
}

/// "Made for the Playnite add-ons A and B; …": what Lodestar does about each.
pub(crate) fn addons_line(names: &[(String, usize)]) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    // Add-ons by their display names (the full reader reports the ids themes use).
    let names: Vec<(String, usize)> = names.iter().map(|(n, c)| (addon_display_name(n).to_string(), *c)).collect();
    let names = names.as_slice();
    let list: Vec<&str> = names.iter().take(6).map(|(n, _)| n.as_str()).collect();
    let joined = match list.len() {
        1 => list[0].to_string(),
        n => format!("{} and {}", list[..n - 1].join(", "), list[n - 1]),
    };
    let more = names.len().saturating_sub(6);
    let mut what = vec![];
    let has = |k: &str| names.iter().any(|(n, _)| n.eq_ignore_ascii_case(k));
    if has("Extra Metadata Loader") {
        what.push("Lodestar shows logos itself; add-on trailers aren't available");
    }
    if has("SuccessStory") || has("Playnite Achievements") {
        what.push("achievements aren't available");
    }
    if has("HowLongToBeat") {
        what.push("HowLongToBeat times aren't available");
    }
    if has("Theme Extras") {
        what.push("ratings and favourites come from Lodestar's own details");
    }
    if has("AnikiHelper") {
        what.push("AnikiHelper's screens aren't available");
    }
    if what.is_empty() {
        what.push("their panels aren't available in Lodestar");
    }
    Some(format!(
        "Made for the Playnite add-on{} {joined}{}; {}",
        if names.len() == 1 { "" } else { "s" },
        if more > 0 { format!(" and {more} more") } else { String::new() },
        what.join(", ")
    ))
}

/// Bytes in words ("12 MB", "640 KB").
pub fn size_text(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 10.0 {
        format!("{} MB", mb.round())
    } else if mb >= 1.0 {
        format!("{:.1} MB", mb)
    } else {
        format!("{} KB", (bytes as f64 / 1024.0).ceil().max(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_follow_the_area_order() {
        let mut b = Builder::default();
        b.no(Area::NotPossible, "x");
        b.ok(Area::Colours, "c");
        b.no(Area::Fonts, "f2");
        b.ok(Area::Fonts, "f1");
        let g = b.groups();
        let titles: Vec<&str> = g.iter().map(|g| g.title.as_str()).collect();
        assert_eq!(titles, ["Colours", "Fonts", "Not possible"]);
        assert_eq!(g[1].lines[0].text, "f1", "carried lines first");
    }

    #[test]
    fn addons_are_named_with_what_lodestar_does() {
        assert_eq!(addons_line(&[]), None);
        let l = addons_line(&[("Extra Metadata Loader".into(), 9), ("SuccessStory".into(), 3)]).unwrap();
        assert!(l.starts_with("Made for the Playnite add-ons Extra Metadata Loader and SuccessStory;"), "{l}");
        assert!(l.contains("Lodestar shows logos itself") && l.contains("achievements aren't available"), "{l}");
        let one = addons_line(&[("Theme Modifier".into(), 1)]).unwrap();
        assert!(one.starts_with("Made for the Playnite add-on Theme Modifier;"), "{one}");
    }

    #[test]
    fn sizes_in_words() {
        assert_eq!(size_text(12 * 1024 * 1024 + 300_000), "12 MB");
        assert_eq!(size_text(1536 * 1024), "1.5 MB");
        assert_eq!(size_text(2000), "2 KB");
    }
}
