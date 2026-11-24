//! `{Binding …}` paths taken apart and mapped onto Lodestar's bindings (`{game.*}`,
//! `{art.*}`, `{list.*}`, `{collection.*}`).

use super::*;

pub(super) fn parse_binding(raw: &str) -> Bind {
    let mut b = Bind::default();
    // Split on commas outside braces.
    let mut parts = vec![];
    let (mut depth, mut cur) = (0i32, String::new());
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
    for (i, p) in parts.iter().enumerate() {
        let p = p.trim();
        match p.split_once('=') {
            Some((k, v)) if !k.contains('{') && !k.contains('[') => {
                let v = v.trim().to_string();
                match k.trim() {
                    "Path" => b.path = v,
                    "ElementName" => b.element = Some(v),
                    "RelativeSource" | "Source" => b.relative = true,
                    "Converter" => {
                        b.converter = Some(match tree::markup(&v) {
                            Markup::Resource(r) => r,
                            _ => v,
                        })
                    }
                    "ConverterParameter" => b.param = Some(v.trim_matches('\'').to_string()),
                    "StringFormat" => b.format = Some(v.trim_matches('\'').to_string()),
                    _ => {}
                }
            }
            _ if i == 0 => b.path = p.to_string(),
            _ => {}
        }
    }
    b
}

/// The binding path without the view model prefixes (`SelectedGame.`, `Game.` …).
pub(super) fn game_path(path: &str) -> String {
    let mut p = path.trim();
    loop {
        let before = p;
        for pre in ["DataContext.", "SelectedGame.", "SelectedGameDetails.", "Game.", "GameDetails."] {
            if let Some(r) = p.strip_prefix(pre) {
                p = r;
            }
        }
        if p == before {
            break;
        }
    }
    p.to_string()
}

/// A game / view-model property → a Lodestar binding key.
pub(super) fn game_key(path: &str) -> Option<&'static str> {
    let p = game_path(path);
    let head = p.split(['.', '[']).next().unwrap_or("");
    Some(match head {
        "DisplayName" | "Name" | "SortingName" => "game.name",
        "Description" | "Notes" if head == "Description" => "game.description",
        "Platforms" | "Platform" => "game.platform",
        "Genres" | "Genre" => "game.genres",
        "Developers" | "Developer" => "game.developer",
        "Publishers" | "Publisher" => "game.publisher",
        "ReleaseYear" | "ReleaseDate" => "game.year",
        "Playtime" => "game.playtime",
        "LastActivity" => "game.lastplayed",
        "PlayCount" => "game.playcount",
        "Added" => "game.added",
        "InstallSize" => "game.size",
        "Source" | "Library" | "LibraryPlugin" => "game.source",
        "CommunityScore" => "game.communityscore",
        "CriticScore" => "game.criticscore",
        "UserScore" => "game.score",
        "CompletionStatus" => "game.completion",
        "Tags" => "game.tags",
        "NameGroup" => "game.letter",
        "IsInstalled" => "game.installed",
        "Favorite" => "game.favorite",
        _ => return None,
    })
}

/// Flags Lodestar knows are always false for a game shown in the list.
pub(super) fn game_false(path: &str) -> bool {
    let p = game_path(path);
    matches!(p.as_str(), "IsInstalling" | "IsUninstalling" | "IsLaunching" | "IsRunning" | "Hidden" | "IsCustomGame")
}

/// A text binding → a Lodestar text template (and whether it is upper-cased).
pub(super) fn bind_text(b: &Bind, main: bool) -> Option<(String, bool)> {
    let conv = b.converter.as_deref().unwrap_or("");
    let key = if let Some(el) = &b.element {
        match (el.as_str(), b.path.trim()) {
            ("PART_ListGameItems", "Items.Count") => "list.count",
            ("PART_ListGameItems", "SelectedIndex") => "list.index",
            _ => return None,
        }
    } else if b.relative {
        return None;
    } else {
        let p = b.path.trim();
        match p {
            "AppSettings.Fullscreen.FilterSettings.Name"
            | "ActiveFilterPreset.Name"
            | "ActiveFilterPreset"
            | "AppSettings.Fullscreen.SelectedFilterPreset.Name" => "collection.name",
            "GamesView.Items.Count" | "GamesView.CollectionView.Count" => "list.count",
            _ if main && !p.contains("SelectedGame") && !p.starts_with("Game.") => return None,
            _ => game_key(p)?,
        }
    };
    let key = match (key, conv) {
        ("game.genres", _) if b.path.contains('[') => "game.genre",
        (k, _) => k,
    };
    let mut tpl = format!("{{{key}}}");
    if let Some(f) = &b.format {
        let f = f.strip_prefix("{}").unwrap_or(f);
        if let Some(i) = f.find("{0") {
            let end = f[i..].find('}').map(|e| i + e + 1).unwrap_or(f.len());
            tpl = format!("{}{tpl}{}", &f[..i], &f[end..]);
        }
    }
    let upper = conv.contains("UpperCase") || conv.contains("ToUpper");
    Some((tpl, upper))
}

/// An image source binding → a Lodestar art binding.
pub(super) fn bind_art(b: &Bind) -> Option<&'static str> {
    if b.element.is_some() || b.relative {
        return None;
    }
    let p = game_path(&b.path);
    let l = p.to_ascii_lowercase();
    if l.contains("platform") || l.contains("library") || l.contains("source") {
        return None;
    }
    if l.contains("cover") {
        Some("{art.cover}")
    } else if l.contains("background") {
        Some("{art.hero}")
    } else if l.contains("icon") {
        Some("{art.icon}")
    } else if l.contains("logo") {
        Some("{art.logo}")
    } else {
        None
    }
}
