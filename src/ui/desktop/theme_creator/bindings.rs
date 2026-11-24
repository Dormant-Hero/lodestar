//! The values a theme can show (`{game.name}`, `{art.cover}` …), in plain words, for the
//! bindings picker. The list follows theme/model.rs.

/// What a value is good for (which fields the picker offers it in).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    Text,
    Picture,
    /// 0..1 (progress bars, ratings).
    Share,
    /// True / empty (conditions).
    Flag,
}

pub struct Binding {
    pub key: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub kind: Use,
}

const fn b(group: &'static str, key: &'static str, label: &'static str, kind: Use) -> Binding {
    Binding { key, label, group, kind }
}

use Use::*;

pub const ALL: &[Binding] = &[
    b("Game", "game.name", "Name", Text),
    b("Game", "game.description", "Description", Text),
    b("Game", "game.developer", "Developer", Text),
    b("Game", "game.publisher", "Publisher", Text),
    b("Game", "game.genre", "Genres (comma separated)", Text),
    b("Game", "game.genrelist", "Genres (one pill each)", Text),
    b("Game", "game.year", "Release year", Text),
    b("Game", "game.platform", "Platform", Text),
    b("Game", "game.source", "Store / library", Text),
    b("Game", "game.tags", "Tags", Text),
    b("Game", "game.letter", "First letter", Text),
    b("Game", "game.size", "Install size", Text),
    b("Game", "game.status", "Install status", Text),
    b("Play", "game.playtimelabel", "Play time (“12h 30m played”)", Text),
    b("Play", "game.playtime", "Play time", Text),
    b("Play", "game.lastplayedlabel", "Last played (“Last played today”)", Text),
    b("Play", "game.lastplayed", "Last played", Text),
    b("Play", "game.playcount", "Times played", Text),
    b("Play", "game.added", "Added to the library", Text),
    b("Play", "game.completion", "Completion (“Beaten”)", Text),
    b("Play", "game.completionkey", "Completion (plan_to_play, beaten …)", Flag),
    b("Play", "game.played", "Ever played", Flag),
    b("Play", "game.favorite", "Is a favourite", Flag),
    b("Play", "game.favoritelabel", "“★ Favourite”", Text),
    b("Play", "game.installed", "Is installed", Flag),
    b("Scores", "game.rating", "Rating (0 to 1)", Share),
    b("Scores", "game.score", "Your score (0–100)", Text),
    b("Scores", "game.criticscore", "Critics' score", Text),
    b("Scores", "game.communityscore", "Players' score", Text),
    b("Scores", "game.scorelabel", "Best score (“Critics 85”)", Text),
    b("Achievements", "game.achievements", "Achievements (“12 / 40”)", Text),
    b("Achievements", "game.achievementslabel", "“12 of 40 achievements”", Text),
    b("Achievements", "game.achievementsunlocked", "Unlocked", Text),
    b("Achievements", "game.achievementstotal", "Total", Text),
    b("Achievements", "game.achievementspercent", "Share unlocked (0 to 1)", Share),
    b("Controller", "game.controllerlabel", "Controller support (“Full controller support”)", Text),
    b("Controller", "game.controllershort", "Controller support (“Full support”)", Text),
    b("Controller", "game.controller", "Controller support (full / partial / none)", Flag),
    b("Controller", "game.deck", "Steam Deck rating (verified / playable / unsupported)", Flag),
    b("Controller", "game.decklabel", "Steam Deck rating (“Deck Verified”)", Text),
    b("Controller", "game.deckshort", "Steam Deck rating (“Verified”)", Text),
    b("Game art", "art.cover", "Cover", Picture),
    b("Game art", "art.hero", "Background", Picture),
    b("Game art", "art.logo", "Logo", Picture),
    b("Game art", "art.banner", "Banner", Picture),
    b("Game art", "art.icon", "Icon", Picture),
    b("Collection", "collection.name", "Name", Text),
    b("Collection", "collection.countlabel", "Games (“12 games”)", Text),
    b("Collection", "collection.count", "Number of games", Text),
    b("Collection", "collection.key", "Key (“platform:sony_playstation2”)", Text),
    b("Console art", "collection.art.card", "Card (the theme's, else Lodestar's)", Picture),
    b("Console art", "collection.art.hero", "Background (the theme's, else Lodestar's)", Picture),
    b("Console art", "collection.art.mark", "Name badge (the theme's, else Lodestar's)", Picture),
    b("Console art", "collection.art.device", "Device (the theme's, else Lodestar's)", Picture),
    b("Console art", "game.consoleart.card", "The game's console: card", Picture),
    b("Console art", "game.consoleart.hero", "The game's console: background", Picture),
    b("Console art", "game.consoleart.mark", "The game's console: name badge", Picture),
    b("Console art", "game.consoleart.device", "The game's console: device", Picture),
    b("Collection art", "collection.hero", "Background of a recent game", Picture),
    b("Collection art", "collection.cover", "Cover of a recent game", Picture),
    b("Collection art", "collection.logo", "Logo of a recent game", Picture),
    b("Collection art", "collection.banner", "Banner of a recent game", Picture),
    b("Collection art", "collection.icon", "Icon of a recent game", Picture),
    b("List", "list.index", "Position of the selection", Text),
    b("List", "list.count", "Number of items", Text),
    b("List", "list.countlabel", "“283 games”", Text),
    b("List", "section.name", "Open section of the rail", Text),
    b("List", "item.focused", "This item has the focus (item parts)", Flag),
    b("System", "system.battery", "Battery level (“82%”)", Text),
    b("System", "system.charging", "Charging", Flag),
    b("System", "theme.dir", "The theme's folder", Text),
];

pub fn find(key: &str) -> Option<&'static Binding> {
    ALL.iter().find(|b| b.key == key)
}

/// The groups in order.
pub fn groups() -> Vec<&'static str> {
    let mut g: Vec<&str> = vec![];
    for b in ALL {
        if !g.contains(&b.group) {
            g.push(b.group);
        }
    }
    g
}

/// The `{…}` keys in a text that Lodestar doesn't know (typos).
pub fn unknown_in(text: &str) -> Vec<String> {
    let mut out = vec![];
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let key = after[..close].trim();
        if !key.is_empty() && find(key).is_none() && !key.starts_with("game.genres") && !out.iter().any(|k| k == key) {
            out.push(key.to_string());
        }
        rest = &after[close + 1..];
    }
    out
}
