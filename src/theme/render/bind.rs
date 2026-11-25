//! Binding resolution: `{game.name}`, `{art.cover}`, `{collection.count}` … (see theme/model.rs).

use crate::library::Collection;
use crate::model::*;
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

/// One entry of the collections view.
#[derive(Clone, Debug, PartialEq)]
pub struct CollectionItem {
    pub collection: Collection,
    /// Display name (theme `display_name` override already applied by the caller if wanted).
    pub name: String,
    /// Number of visible games.
    pub count: usize,
    /// Artwork of a representative game (for `{collection.cover}` / `{collection.hero}` …).
    pub art: Art,
}

/// What bindings resolve against.
#[derive(Clone, Copy, Default)]
pub struct Binder<'a> {
    pub game: Option<&'a Game>,
    pub collection: Option<&'a CollectionItem>,
    pub theme_dir: Option<&'a Path>,
    /// (0-based index, count) of the selection in the focused list.
    pub list: Option<(usize, usize)>,
    /// Name of the navigation rail's open section ("Platforms" …) for `{section.name}`.
    pub section: Option<&'a str>,
    /// Item templates (`Common::item_elements`): the item has the focus (`{item.focused}`).
    pub focused: bool,
    /// Drawing a game's page: `{game.achievements…}` show the page's own answer (the library's
    /// count elsewhere).
    pub page: bool,
    /// The theme's own pictures per collection (`[collection_art]`) for `{collection.art.*}` /
    /// `{game.consoleart.*}`; None = Lodestar's console artwork only.
    pub collection_art: Option<&'a std::collections::BTreeMap<String, crate::theme::CollectionArt>>,
}

impl<'a> Binder<'a> {
    pub fn with_game(mut self, g: Option<&'a Game>) -> Self {
        self.game = g;
        self
    }
    pub fn with_collection(mut self, c: Option<&'a CollectionItem>) -> Self {
        self.collection = c;
        self
    }

    /// Value of one binding key. `raw` = machine form (DateTime elements want unix seconds).
    pub fn value(&self, key: &str, raw: bool, out: &mut String) {
        if let Some(kind) = key.strip_prefix("collection.art.") {
            self.console_art_value(kind, false, out);
        } else if let Some(kind) = key.strip_prefix("game.consoleart.") {
            self.console_art_value(kind, true, out);
        } else if key.starts_with("game.") {
            self.game_value(key, raw, out);
        } else if key.starts_with("collection.") {
            self.collection_value(key, out);
        } else {
            self.other_value(key, out);
        }
    }

    /// `collection.art.*` / `game.consoleart.*` (`kind`: card, hero, mark, device): the theme's
    /// own picture for the collection (or the game's console) when it has one, else Lodestar's.
    fn console_art_value(&self, kind: &str, game: bool, out: &mut String) {
        use crate::theme::console_art::{self, ArtKind};
        let Some(art_kind) = ArtKind::parse(kind) else { return };
        let (full, key) = if game {
            let Some(id) = self.game.and_then(console_art::key_for_game) else { return };
            (format!("platform:{id}"), Some(id))
        } else {
            let Some(c) = self.collection else { return };
            (c.collection.key(), console_art::key_for(&c.collection))
        };
        if let Some(map) = self.collection_art
            && let Some(own) = map.get(&full).or_else(|| key.and_then(|k| map.get(k))).and_then(|a| a.get(kind))
        {
            out.push_str(own);
            return;
        }
        if let Some(p) = key.and_then(|k| console_art::path(k, art_kind)) {
            out.push_str(&p.to_string_lossy());
        }
    }

    /// `game.*`: the selected game's facts.
    fn game_value(&self, key: &str, raw: bool, out: &mut String) {
        let g = self.game;
        let flag = |b: bool, out: &mut String| {
            if b {
                out.push_str("true");
            }
        };
        match key {
            "game.name" => {
                if let Some(g) = g {
                    out.push_str(g.display_name());
                }
            }
            "game.description" => {
                if let Some(g) = g
                    && let Some(d) = g.meta.description.as_deref()
                {
                    out.push_str(d.trim());
                    // Wikipedia's text is CC BY-SA: it is credited wherever it is shown.
                    if g.meta.wikipedia.is_some() {
                        out.push_str(crate::metadata::wikipedia::CREDIT_LINE);
                    }
                }
            }
            "game.developer" => out.push_str(g.and_then(|g| g.meta.developer.as_deref()).unwrap_or("")),
            "game.publisher" => out.push_str(g.and_then(|g| g.meta.publisher.as_deref()).unwrap_or("")),
            "game.genre" | "game.genres" => {
                if let Some(g) = g {
                    out.push_str(&g.meta.genres.join(", "));
                }
            }
            // Up to three genres separated by "|": each becomes its own pill in `chips` text.
            "game.genrelist" => {
                if let Some(g) = g {
                    for (i, genre) in g.meta.genres.iter().filter(|s| !s.trim().is_empty()).take(3).enumerate() {
                        if i > 0 {
                            out.push('|');
                        }
                        out.push_str(genre.trim());
                    }
                }
            }
            "game.criticscore" | "game.communityscore" => {
                let v = g
                    .and_then(|g| if key == "game.criticscore" { g.meta.critic_score } else { g.meta.community_score });
                if let Some(v) = v {
                    let _ = write!(out, "{}", v.min(100));
                }
            }
            "game.year" => {
                if let Some(y) = g.and_then(|g| g.meta.release_year) {
                    let _ = write!(out, "{y}");
                }
            }
            "game.playtime" => {
                if let Some(g) = g {
                    if raw {
                        let _ = write!(out, "{}", g.playtime_secs);
                    } else {
                        out.push_str(&format_playtime(g.playtime_secs));
                    }
                }
            }
            "game.played" => flag(g.is_some_and(|g| g.playtime_secs > 0 || g.last_played.is_some()), out),
            "game.lastplayed" => {
                if let Some(g) = g {
                    if raw {
                        if let Some(t) = g.last_played {
                            let _ = write!(out, "{t}");
                        }
                    } else {
                        out.push_str(&format_last_played(g.last_played));
                    }
                }
            }
            "game.added" => {
                if let Some(g) = g {
                    if raw {
                        let _ = write!(out, "{}", g.added);
                    } else {
                        out.push_str(&format_last_played(Some(g.added)));
                    }
                }
            }
            // "1", "1-4", "1+"; empty when unknown.
            "game.players" => {
                if let Some(p) = g.and_then(|g| {
                    g.meta.players.clone().or_else(|| crate::model::players_from_features(&g.meta.features))
                }) {
                    out.push_str(&p);
                }
            }
            "game.playcount" => {
                if let Some(g) = g {
                    let _ = write!(out, "{}", g.play_count);
                }
            }
            "game.source" => {
                if let Some(g) = g {
                    out.push_str(&source_text(g));
                }
            }
            "game.sourcekey" => out.push_str(g.map(|g| g.source.key()).unwrap_or("")),
            "game.rating" => {
                // Star rating, else the 0..100 user score from Edit Game Details.
                if let Some(r) = g.and_then(|g| g.user_rating) {
                    let _ = write!(out, "{}", r.min(5) as f32 / 5.0);
                } else if let Some(s) = g.and_then(|g| g.edits.user_score) {
                    let _ = write!(out, "{}", s.min(100) as f32 / 100.0);
                }
            }
            "game.completion" => {
                if let Some(c) = g.and_then(|g| g.completion()) {
                    out.push_str(c.label());
                }
            }
            "game.completionkey" => {
                if let Some(c) = g.and_then(|g| g.completion()) {
                    out.push_str(completion_key(c));
                }
            }
            "game.score" => {
                if let Some(s) = g.and_then(|g| g.edits.user_score) {
                    let _ = write!(out, "{}", s.min(100));
                }
            }
            "game.favorite" => flag(g.is_some_and(|g| g.favorite), out),
            "game.installed" => flag(g.is_some_and(|g| g.installed), out),
            "game.status" => {
                if let Some(g) = g {
                    out.push_str(if g.installed { "Installed" } else { "Not installed" });
                }
            }
            // As people read it ("Windows", "Sony PlayStation 2"); raw (and in picture paths:
            // theme files are named after it) as stored ("windows").
            "game.platform" => {
                let p = g.and_then(|g| g.platform.as_deref()).unwrap_or("");
                out.push_str(if raw { p } else { crate::emulation::platforms::display_name(p) })
            }
            "game.tags" => {
                if let Some(g) = g {
                    out.push_str(&g.tags.join(", "));
                }
            }
            "game.size" => {
                if let Some(b) = g.and_then(|g| g.size_bytes) {
                    out.push_str(&format_size(b));
                }
            }
            "game.id" => {
                if let Some(g) = g {
                    let _ = write!(out, "{}", g.id);
                }
            }
            "game.letter" => {
                if let Some(c) = g.and_then(|g| g.sort_name.chars().next()) {
                    out.extend(c.to_uppercase());
                }
            }
            _ => self.game_label_value(key, out),
        }
    }

    /// `game.*` labels in words: scores, play time, last played, favourite, controller /
    /// Steam Deck / ProtonDB ratings.
    fn game_label_value(&self, key: &str, out: &mut String) {
        let g = self.game;
        match key {
            "game.achievements"
            | "game.achievementsunlocked"
            | "game.achievementstotal"
            | "game.achievementspercent"
            | "game.achievementslabel" => {
                if let Some((n, total)) = g.and_then(|g| achievements_of(g.id, self.page)) {
                    achievements_value(key, n, total, out);
                }
            }
            // The best known score in words: critics, else players, else the user's own.
            "game.scorelabel" => {
                if let Some(g) = g {
                    if let Some(v) = g.meta.critic_score {
                        let _ = write!(out, "Critics {}", v.min(100));
                    } else if let Some(v) = g.meta.community_score {
                        let _ = write!(out, "Players {}", v.min(100));
                    } else if let Some(v) = g.edits.user_score {
                        let _ = write!(out, "Your score {}", v.min(100));
                    }
                }
            }
            "game.playtimelabel" => {
                if let Some(g) = g.filter(|g| g.playtime_secs > 0) {
                    out.push_str(&format_playtime(g.playtime_secs));
                    out.push_str(" played");
                }
            }
            "game.lastplayedlabel" => {
                if let Some(t) = g.and_then(|g| g.last_played) {
                    out.push_str("Last played ");
                    let s = format_last_played(Some(t));
                    // "Last played Today" reads oddly capitalised.
                    if matches!(s.as_str(), "Today" | "Yesterday") || s.ends_with("ago") {
                        out.push_str(&s.to_lowercase());
                    } else {
                        out.push_str(&s);
                    }
                }
            }
            "game.favoritelabel" => {
                if g.is_some_and(|g| g.favorite) {
                    out.push_str("★ Favourite");
                }
            }
            "game.controller" | "game.controllerlabel" | "game.controllershort" => {
                if let Some(c) = g.and_then(|g| g.controller()) {
                    out.push_str(match (key, c) {
                        ("game.controller", ControllerSupport::Full) => "full",
                        ("game.controller", ControllerSupport::Partial) => "partial",
                        ("game.controller", ControllerSupport::None) => "none",
                        ("game.controllershort", ControllerSupport::Full) => "Full support",
                        ("game.controllershort", ControllerSupport::Partial) => "Partial support",
                        ("game.controllershort", ControllerSupport::None) => "No support",
                        _ => c.label(),
                    });
                }
            }
            "game.deck" | "game.decklabel" | "game.deckshort" => {
                if let Some(d) = g.and_then(|g| g.meta.deck) {
                    out.push_str(match (key, d) {
                        ("game.deck", DeckRating::Verified) => "verified",
                        ("game.deck", DeckRating::Playable) => "playable",
                        ("game.deck", DeckRating::Unsupported) => "unsupported",
                        ("game.decklabel", DeckRating::Verified) => "Deck Verified",
                        ("game.decklabel", DeckRating::Playable) => "Deck Playable",
                        ("game.decklabel", DeckRating::Unsupported) => "Deck Unsupported",
                        (_, DeckRating::Verified) => "Verified",
                        (_, DeckRating::Playable) => "Playable",
                        (_, DeckRating::Unsupported) => "Unsupported",
                    });
                }
            }
            // ProtonDB tier: "gold" / "ProtonDB Gold" / "Gold"; empty when unknown.
            "game.protondb" | "game.protondblabel" | "game.protondbshort" => {
                if let Some((t, label)) = g
                    .and_then(|g| g.meta.protondb.as_deref())
                    .and_then(|t| Some((t, crate::metadata::protondb::tier_label(t)?)))
                {
                    match key {
                        "game.protondb" => out.push_str(&t.trim().to_ascii_lowercase()),
                        "game.protondblabel" => {
                            out.push_str("ProtonDB ");
                            out.push_str(label);
                        }
                        _ => out.push_str(label),
                    }
                }
            }
            _ => {}
        }
    }

    /// `collection.*`: the collection entry being drawn.
    fn collection_value(&self, key: &str, out: &mut String) {
        match key {
            "collection.cover" | "collection.hero" | "collection.logo" | "collection.banner" | "collection.icon" => {
                if let Some(c) = self.collection {
                    let p = match key {
                        "collection.cover" => &c.art.cover,
                        "collection.hero" => &c.art.hero,
                        "collection.logo" => &c.art.logo,
                        "collection.banner" => &c.art.banner,
                        _ => &c.art.icon,
                    };
                    if let Some(p) = p {
                        out.push_str(&p.to_string_lossy());
                    }
                }
            }
            "collection.name" => out.push_str(self.collection.map(|c| c.name.as_str()).unwrap_or("")),
            "collection.key" => {
                if let Some(c) = self.collection {
                    out.push_str(&c.collection.key());
                }
            }
            "collection.count" => {
                if let Some(c) = self.collection {
                    let _ = write!(out, "{}", c.count);
                }
            }
            "collection.countlabel" => {
                if let Some(c) = self.collection {
                    let _ = write!(out, "{} game{}", c.count, if c.count == 1 { "" } else { "s" });
                }
            }
            _ => {}
        }
    }

    /// The selected game's art, the focused list, the rail's section, the system, the theme.
    fn other_value(&self, key: &str, out: &mut String) {
        let g = self.game;
        let flag = |b: bool, out: &mut String| {
            if b {
                out.push_str("true");
            }
        };
        let art = |sel: fn(&Art) -> &Option<std::path::PathBuf>, out: &mut String| {
            if let Some(p) = g.and_then(|g| sel(&g.art).as_ref()) {
                out.push_str(&p.to_string_lossy());
            }
        };
        match key {
            "item.focused" => flag(self.focused, out),
            "art.cover" => art(|a| &a.cover, out),
            "art.hero" => art(|a| &a.hero, out),
            "art.logo" => art(|a| &a.logo, out),
            "art.icon" => art(|a| &a.icon, out),
            "art.banner" => art(|a| &a.banner, out),
            "list.index" => {
                if let Some((i, n)) = self.list
                    && n > 0
                {
                    let _ = write!(out, "{}", i + 1);
                }
            }
            "list.countlabel" => {
                if let Some((_, n)) = self.list {
                    let _ = write!(out, "{n} game{}", if n == 1 { "" } else { "s" });
                }
            }
            "list.count" => {
                if let Some((_, n)) = self.list {
                    let _ = write!(out, "{n}");
                }
            }
            "section.name" => out.push_str(self.section.unwrap_or("")),
            // "82%" on machines with a battery (handhelds, laptops), "" otherwise.
            "system.battery" => {
                if let Some((pct, _)) = battery() {
                    let _ = write!(out, "{pct}%");
                }
            }
            "system.charging" => {
                if battery().is_some_and(|(_, c)| c) {
                    out.push_str("true");
                }
            }
            "theme.dir" => {
                if let Some(d) = self.theme_dir {
                    out.push_str(&d.to_string_lossy());
                }
            }

            _ => {}
        }
    }

    /// Resolve every `{key}` in `tpl`. Text without braces is returned borrowed.
    pub fn resolve<'t>(&self, tpl: &'t str, raw: bool) -> Cow<'t, str> {
        if !tpl.contains('{') {
            return Cow::Borrowed(tpl);
        }
        let mut out = String::with_capacity(tpl.len() + 16);
        self.resolve_into(tpl, raw, &mut out, false);
        Cow::Owned(out)
    }

    /// Resolve `tpl` into `out`.
    /// `path`: a picture's path, where values that name files are used as stored.
    fn resolve_into(&self, tpl: &str, raw: bool, out: &mut String, path: bool) {
        let mut rest = tpl;
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open + 1..];
            match after.find('}') {
                Some(close) => {
                    let key = after[..close].trim();
                    match key {
                        "game.platform" if path => self.value(key, true, out),
                        _ => self.value(key, raw, out),
                    }
                    rest = &after[close + 1..];
                }
                None => {
                    out.push_str(&rest[open..]);
                    rest = "";
                }
            }
        }
        out.push_str(rest);
    }

    /// Candidate strings for an image path: the resolved template (nothing for an empty one).
    pub fn path_candidates(&self, tpl: &str, out: &mut Vec<String>) {
        if tpl.trim().is_empty() {
            return;
        }
        let mut s = String::with_capacity(tpl.len() + 16);
        self.resolve_into(tpl, false, &mut s, true);
        // A theme made before a source was renamed has its picture under the old name.
        let renamed = self.renamed_source(tpl).map(|src| s.replace(src.display(), src.former_name().unwrap_or("")));
        out.push(s);
        out.extend(renamed);
        // A theme's own console picture that is missing falls back to Lodestar's.
        if self.collection_art.is_some() && (tpl.contains("{collection.art.") || tpl.contains("{game.consoleart.")) {
            let mut s = String::with_capacity(tpl.len() + 16);
            Binder { collection_art: None, ..*self }.resolve_into(tpl, false, &mut s, true);
            if out.last() != Some(&s) {
                out.push(s);
            }
        }
    }

    /// The renamed source `tpl` prints by name (`{game.source}`, a source's `{collection.name}`).
    fn renamed_source(&self, tpl: &str) -> Option<Source> {
        let game = self.game.filter(|g| g.emu.is_none() && tpl.contains("{game.source}")).map(|g| g.source);
        let coll =
            self.collection.filter(|_| tpl.contains("{collection.name}")).and_then(|c| match c.collection.kind {
                crate::library::CollectionKind::Source(s) => Some(s),
                _ => None,
            });
        game.into_iter().chain(coll).find(|s| s.former_name().is_some())
    }

    /// The part of the picture an image element shows ("focus"): the game's background
    /// placement when the element is bound to the game's background (`{art.hero}` in its path
    /// or a fallback) and `chosen` is that background (the hero, else the banner or cover
    /// standing in for it). None = the element's own automatic crop.
    pub fn image_crop(
        &self,
        tpl: &str,
        fallbacks: &[String],
        chosen: &Path,
    ) -> Option<crate::model::BackgroundPlacement> {
        let g = self.game?;
        let pl = g.edits.background_placement?;
        let bound = |t: &str| t.contains("{art.hero}");
        if !(bound(tpl) || fallbacks.iter().any(|f| bound(f))) {
            return None;
        }
        (g.art.background().map(|p| p.as_path()) == Some(chosen)).then_some(pl)
    }

    /// Visibility rules of an element.
    pub fn visible(&self, c: &crate::theme::Common) -> bool {
        if !c.visible || c.opacity <= 0.0 {
            return false;
        }
        if let Some(v) = &c.visible_if
            && !self.condition(v)
        {
            return false;
        }
        if let Some(h) = &c.hidden_if
            && self.condition(h)
        {
            return false;
        }
        true
    }

    /// A `visible_if` / `hidden_if` condition. A binding is true when it resolves to something
    /// non-empty ("{art.logo}", "{game.favorite}"); `!` negates one; `a == b`, `a != b` compare
    /// (as numbers when both are numbers, else as trimmed text ignoring case; a side may be a
    /// binding or a literal, quotes optional; against `true` / `false` as truth values), `<`,
    /// `<=`, `>`, `>=` compare numbers (false when a side isn't one); `&&` and `||` combine
    /// (`&&` first, no brackets).
    pub fn condition(&self, expr: &str) -> bool {
        if !expr.contains("||") && !expr.contains("&&") {
            return self.term(expr);
        }
        expr.split("||").any(|any| any.split("&&").all(|t| self.term(t)))
    }

    fn term(&self, t: &str) -> bool {
        let t = t.trim();
        const OPS: [&str; 6] = ["==", "!=", ">=", "<=", ">", "<"];
        // The first operator outside a binding.
        let mut found: Option<(usize, &str)> = None;
        let mut depth = 0i32;
        let b = t.as_bytes();
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ if depth <= 0 => {
                    if let Some(op) = OPS.iter().find(|op| t[i..].starts_with(**op)) {
                        found = Some((i, op));
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        let Some((at, op)) = found else {
            return match t.strip_prefix('!') {
                Some(rest) => !self.truthy(rest),
                None => self.truthy(t),
            };
        };
        let side = |s: &str| {
            let s = s.trim();
            let s = s
                .strip_prefix('"')
                .and_then(|x| x.strip_suffix('"'))
                .or_else(|| s.strip_prefix('\'').and_then(|x| x.strip_suffix('\'')))
                .unwrap_or(s);
            self.resolve(s, true).trim().to_string()
        };
        let (l, r) = (side(&t[..at]), side(&t[at + op.len()..]));
        // Flags resolve to "true" / "": against `true` / `false` they compare as truth values
        // (`{game.installed} == false`, Playnite's "IsInstalled = False" trigger).
        let is_bool = |s: &str| s.eq_ignore_ascii_case("true") || s.eq_ignore_ascii_case("false");
        if matches!(op, "==" | "!=") && (is_bool(&l) || is_bool(&r)) {
            let truth = |s: &str| !s.is_empty() && !s.eq_ignore_ascii_case("false");
            return (truth(&l) == truth(&r)) == (op == "==");
        }
        let nums = (l.parse::<f64>().ok(), r.parse::<f64>().ok());
        match (op, nums) {
            ("==", (Some(a), Some(b))) => a == b,
            ("!=", (Some(a), Some(b))) => a != b,
            ("==", _) => same_text(&l, &r),
            ("!=", _) => !same_text(&l, &r),
            (">=", (Some(a), Some(b))) => a >= b,
            ("<=", (Some(a), Some(b))) => a <= b,
            (">", (Some(a), Some(b))) => a > b,
            ("<", (Some(a), Some(b))) => a < b,
            _ => false,
        }
    }

    fn truthy(&self, t: &str) -> bool {
        !self.resolve(t.trim(), true).trim().is_empty()
    }
}

/// Text comparison of conditions: equal ignoring case, and a renamed source's old name equals
/// its new one (a theme's `{game.source} == Desktop` still matches).
fn same_text(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
        || Source::ALL.into_iter().any(|s| {
            s.former_name().is_some_and(|old| {
                let new = s.display();
                (a.eq_ignore_ascii_case(new) && b.eq_ignore_ascii_case(old))
                    || (a.eq_ignore_ascii_case(old) && b.eq_ignore_ascii_case(new))
            })
        })
}

/// Stable machine key of a completion status (`{game.completionkey}`, image paths, `visible_if`).
pub fn completion_key(c: CompletionStatus) -> &'static str {
    match c {
        CompletionStatus::NotPlayed => "not_played",
        CompletionStatus::PlanToPlay => "plan_to_play",
        CompletionStatus::Playing => "playing",
        CompletionStatus::OnHold => "on_hold",
        CompletionStatus::Beaten => "beaten",
        CompletionStatus::Completed => "completed",
        CompletionStatus::Abandoned => "abandoned",
    }
}

pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[u])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// `{game.source}`: the library a game comes from — for emulated games their console
/// ("PlayStation 2", "GameCube") instead of "Emulators".
pub fn source_text(g: &Game) -> Cow<'static, str> {
    let Some(p) = g.emu.as_ref().and_then(|e| crate::emulation::platforms::by_id(&e.platform)) else {
        return g.source_name().into();
    };
    if p.id == "nintendo_nes" {
        return "NES".into();
    }
    // Without the maker when the rest still names the console ("Nintendo 64" stays).
    // Same rule as the controller's console cards (ui/controller/lists.rs `console_label`).
    const MAKERS: &[&str] = &[
        "Sony",
        "Nintendo",
        "Microsoft",
        "Sega",
        "NEC",
        "SNK",
        "Bandai",
        "Atari",
        "Commodore",
        "Mattel",
        "Coleco",
        "Magnavox",
        "GCE",
        "Sharp",
        "Sinclair",
        "Amstrad",
        "Thomson",
        "Watara",
    ];
    MAKERS
        .iter()
        .find_map(|m| p.name.strip_prefix(m)?.strip_prefix(' '))
        .filter(|rest| rest.chars().count() >= 3 && rest.starts_with(|c: char| c.is_alphabetic()))
        .unwrap_or(p.name)
        .into()
}

// ------------------------------------------------------------------ dates

/// Local broken-down time (year, month 1-12, day, hour, minute, weekday 0=Sunday).
pub fn local_time(ts: i64) -> (i64, u32, u32, u32, u32, u32) {
    // SAFETY: localtime_r only writes into the provided struct.
    unsafe {
        let t: libc::time_t = ts as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            let (y, m, d) = civil_from_days(ts.div_euclid(86400));
            let secs = ts.rem_euclid(86400);
            return (
                y,
                m,
                d,
                (secs / 3600) as u32,
                ((secs % 3600) / 60) as u32,
                ((ts.div_euclid(86400) + 4).rem_euclid(7)) as u32,
            );
        }
        (
            tm.tm_year as i64 + 1900,
            (tm.tm_mon + 1).clamp(1, 12) as u32,
            tm.tm_mday.max(1) as u32,
            tm.tm_hour.max(0) as u32,
            tm.tm_min.max(0) as u32,
            tm.tm_wday.clamp(0, 6) as u32,
        )
    }
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// strftime subset: %Y %y %m %d %e %H %I %M %p %b %B %a %%. "relative" = "3 days ago".
pub fn format_date(ts: i64, fmt: &str) -> String {
    if fmt.trim().eq_ignore_ascii_case("relative") {
        return format_last_played(Some(ts));
    }
    let (y, mo, d, h, mi, wd) = local_time(ts);
    let mut out = String::new();
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => {
                let _ = write!(out, "{y}");
            }
            Some('y') => {
                let _ = write!(out, "{:02}", y.rem_euclid(100));
            }
            Some('m') => {
                let _ = write!(out, "{mo:02}");
            }
            Some('d') => {
                let _ = write!(out, "{d:02}");
            }
            Some('e') => {
                let _ = write!(out, "{d}");
            }
            Some('H') => {
                let _ = write!(out, "{h:02}");
            }
            Some('I') => {
                let _ = write!(out, "{:02}", if h % 12 == 0 { 12 } else { h % 12 });
            }
            Some('M') => {
                let _ = write!(out, "{mi:02}");
            }
            Some('p') => out.push_str(if h < 12 { "AM" } else { "PM" }),
            Some('b') => out.push_str(MONTHS[(mo - 1) as usize]),
            Some('B') => out.push_str(MONTHS_LONG[(mo - 1) as usize]),
            Some('a') => out.push_str(DAYS[wd as usize]),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

/// Battery charge (percent, charging) of the first system battery, read from
/// /sys/class/power_supply at most every 30 s. None = no battery (desktop PCs).
/// Screenshot checks of the battery corner on a machine without one:
/// `LODESTAR_FAKE_BATTERY=82` (or `82+` for charging). `None` = not set.
pub fn fake_battery() -> Option<Option<(u8, bool)>> {
    let v = std::env::var("LODESTAR_FAKE_BATTERY").ok()?;
    let charging = v.ends_with('+');
    Some(v.trim_end_matches('+').trim().parse::<u8>().ok().map(|p| (p.min(100), charging)))
}

pub fn battery() -> Option<(u8, bool)> {
    use std::sync::Mutex;
    if let Some(fake) = fake_battery() {
        return fake;
    }
    use std::time::{Duration, Instant};
    type Reading = Option<(Instant, Option<(u8, bool)>)>;
    static CACHE: Mutex<Reading> = Mutex::new(None);
    let mut c = CACHE.lock().ok()?;
    if let Some((t, v)) = *c
        && t.elapsed() < Duration::from_secs(30)
    {
        return v;
    }
    // Later readings on a helper thread (laptop firmware can take tens of milliseconds to
    // answer); the last reading stays meanwhile.
    if let Some((_, v)) = *c {
        static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        use std::sync::atomic::Ordering;
        if !BUSY.swap(true, Ordering::AcqRel) {
            let spawned = std::thread::Builder::new().name("battery".into()).spawn(|| {
                let v = read_battery(Path::new("/sys/class/power_supply"));
                if let Ok(mut c) = CACHE.lock() {
                    *c = Some((Instant::now(), v));
                }
                BUSY.store(false, Ordering::Release);
            });
            if spawned.is_err() {
                BUSY.store(false, Ordering::Release);
            }
        }
        return v;
    }
    let v = read_battery(Path::new("/sys/class/power_supply"));
    *c = Some((Instant::now(), v));
    v
}

pub(crate) fn read_battery(root: &Path) -> Option<(u8, bool)> {
    let mut dirs: Vec<_> = std::fs::read_dir(root).ok()?.flatten().map(|e| e.path()).collect();
    dirs.sort();
    dirs.into_iter().find_map(|d| {
        let kind = std::fs::read_to_string(d.join("type")).ok()?;
        if kind.trim() != "Battery" {
            return None;
        }
        // Peripheral batteries (pads, mice) report scope "Device": not the machine's.
        if std::fs::read_to_string(d.join("scope")).is_ok_and(|s| s.trim() == "Device") {
            return None;
        }
        let pct: u8 = std::fs::read_to_string(d.join("capacity")).ok()?.trim().parse().ok()?;
        let status = std::fs::read_to_string(d.join("status")).unwrap_or_default();
        Some((pct.min(100), matches!(status.trim(), "Charging" | "Full")))
    })
}

/// Achievements per game ("unlocked of total") for the `{game.achievements…}` bindings, as the
/// UI last published them (`ui::achievements::publish_binding`): the renderer never waits for
/// a lookup, a game not looked up yet reads empty.
static ACHIEVEMENTS: std::sync::LazyLock<parking_lot::RwLock<std::collections::HashMap<GameId, (u32, u32)>>> =
    std::sync::LazyLock::new(Default::default);

/// Publish (or forget, `None`) a game's achievement count for theme bindings.
pub fn set_achievements(id: GameId, v: Option<(u32, u32)>) {
    if ACHIEVEMENTS.read().get(&id).copied() == v {
        return;
    }
    let mut m = ACHIEVEMENTS.write();
    match v {
        Some(v) => {
            m.insert(id, v);
        }
        None => {
            m.remove(&id);
        }
    }
}

/// The library's counts (`achievements::Hub::counts`: only what the settings let the library
/// show), for game lists and every view but a game's page.
type Counts = Arc<HashMap<GameId, (u32, u32)>>;
static LIBRARY: std::sync::LazyLock<parking_lot::RwLock<Counts>> = std::sync::LazyLock::new(Default::default);

/// Publish the library's counts (when they or the settings change).
pub fn set_library_achievements(counts: Counts) {
    *LIBRARY.write() = counts;
}

/// On a game's page its own answer (published by the page), else the library's count; in lists
/// only the library's.
fn achievements_of(id: GameId, page: bool) -> Option<(u32, u32)> {
    let own = page.then(|| ACHIEVEMENTS.read().get(&id).copied()).flatten();
    own.or_else(|| LIBRARY.read().get(&id).copied()).filter(|&(_, t)| t > 0)
}

/// One `{game.achievements…}` binding: "12 / 40", "12", "40", "0.3", "12 of 40 achievements".
fn achievements_value(key: &str, unlocked: u32, total: u32, out: &mut String) {
    let n = unlocked.min(total);
    let _ = match key {
        "game.achievements" => write!(out, "{n} / {total}"),
        "game.achievementsunlocked" => write!(out, "{n}"),
        "game.achievementstotal" => write!(out, "{total}"),
        "game.achievementspercent" => write!(out, "{}", n as f32 / total.max(1) as f32),
        _ => write!(out, "{n} of {total} achievements"),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Collection, CollectionKind};

    #[test]
    fn emulated_games_show_their_console_as_source() {
        let mut g = Game { source: Source::Emulator, ..Game::blank("Okami") };
        assert_eq!(source_text(&g), "Emulators", "no console known");
        for (id, want) in [
            ("sony_playstation2", "PlayStation 2"),
            ("nintendo_gamecube", "GameCube"),
            ("nintendo_64", "Nintendo 64"),
            ("nintendo_nes", "NES"),
        ] {
            if crate::emulation::platforms::by_id(id).is_none() {
                continue;
            }
            g.emu = Some(EmuRef { platform: id.into(), ..Default::default() });
            assert_eq!(source_text(&g), want);
        }
        assert_eq!(source_text(&game()), "Steam");
    }

    fn game() -> Game {
        Game {
            id: 7,
            source: Source::Steam,
            source_id: "1".into(),
            art: Art { cover: Some("/c/cover.jpg".into()), ..Default::default() },
            meta: Metadata {
                genres: vec!["Puzzle".into(), "FPS".into()],
                release_year: Some(2011),
                ..Default::default()
            },
            playtime_secs: 3600 * 3,
            last_played: Some(1_700_000_000),
            play_count: 2,
            size_bytes: Some(12 * 1024 * 1024 * 1024),
            favorite: true,
            user_rating: Some(4),
            ..Game::blank("Portal 2")
        }
    }

    #[test]
    fn players_like_es_de() {
        use crate::model::{players_from_features, players_from_steam_categories, players_text};
        assert_eq!(players_text(true, false, 0).as_deref(), Some("1"));
        assert_eq!(players_text(true, true, 4).as_deref(), Some("1-4"));
        assert_eq!(players_text(true, true, 0).as_deref(), Some("1+"));
        assert_eq!(players_text(false, true, 0).as_deref(), Some("2+"));
        assert_eq!(players_text(false, true, 8).as_deref(), Some("2-8"));
        assert_eq!(players_text(false, false, 0), None);
        // Steam: 2 single-player; 1 multi-player, 9 co-op, 24 split screen, 36-39 / 47-49 online / LAN.
        assert_eq!(players_from_steam_categories(&[2, 22, 28]).as_deref(), Some("1"));
        assert_eq!(players_from_steam_categories(&[2, 1, 9]).as_deref(), Some("1+"));
        assert_eq!(players_from_steam_categories(&[49]).as_deref(), Some("2+"));
        assert_eq!(players_from_steam_categories(&[22, 28]), None);
        // IGDB modes already stored as features.
        let f = |v: &[&str]| players_from_features(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(f(&["Single player", "Side view"]).as_deref(), Some("1"));
        assert_eq!(f(&["Single player", "Co-operative", "Split screen"]).as_deref(), Some("1+"));
        assert_eq!(f(&["Massively Multiplayer Online (MMO)"]).as_deref(), Some("2+"));
        assert_eq!(f(&["Third person"]), None);
        // The binding: stored value first, else the features; empty when unknown.
        let mut g = game();
        let b = |g: &Game| Binder::default().with_game(Some(g)).resolve("{game.players}", false).into_owned();
        assert_eq!(b(&g), "");
        g.meta.features = vec!["Single player".into()];
        assert_eq!(b(&g), "1");
        g.meta.players = Some("1-4".into());
        assert_eq!(b(&g), "1-4");
    }

    /// The "Desktop" source is shown as "Shortcuts" now; a theme written for the old name keeps
    /// its `Desktop.png` logo and its `== Desktop` rules.
    #[test]
    fn a_renamed_source_still_finds_what_its_old_name_found() {
        let g = Game { source: Source::Desktop, ..Game::blank("Doom") };
        let b = Binder::default().with_game(Some(&g));
        assert_eq!(b.resolve("{game.source}", false), "Shortcuts");
        let mut out = vec![];
        b.path_candidates("logos/{game.source}.png", &mut out);
        assert_eq!(out, ["logos/Shortcuts.png", "logos/Desktop.png"]);
        assert!(b.condition("{game.source} == Desktop") && b.condition("{game.source} == shortcuts"));
        assert!(!b.condition("{game.source} != Desktop"));
        assert_eq!(b.resolve("{game.sourcekey}", false), "desktop", "the key never changes");
        let item = CollectionItem {
            collection: Collection {
                kind: crate::library::CollectionKind::Source(Source::Desktop),
                name: "Shortcuts".into(),
            },
            name: "Shortcuts".into(),
            count: 1,
            art: Art::default(),
        };
        let mut out = vec![];
        Binder::default().with_collection(Some(&item)).path_candidates("art/{collection.name}.png", &mut out);
        assert_eq!(out, ["art/Shortcuts.png", "art/Desktop.png"]);
        // Other sources get no second guess.
        let steam = Game { source: Source::Steam, ..Game::blank("Doom") };
        let mut out = vec![];
        Binder::default().with_game(Some(&steam)).path_candidates("logos/{game.source}.png", &mut out);
        assert_eq!(out, ["logos/Steam.png"]);
        assert!(!Binder::default().with_game(Some(&steam)).condition("{game.source} == Desktop"));
    }

    #[test]
    fn hero_bound_images_get_the_background_placement() {
        let mut g = game();
        g.art.hero = Some("/c/hero.jpg".into());
        let b = Binder::default().with_game(Some(&g));
        let hero = Path::new("/c/hero.jpg");
        // No placement: the element's own crop.
        assert_eq!(b.image_crop("{art.hero}", &[], hero), None);
        let pl = BackgroundPlacement { x: 0.2, y: 0.9, zoom: 1.5 };
        g.edits.background_placement = Some(pl);
        let b = Binder::default().with_game(Some(&g));
        assert_eq!(b.image_crop("{art.hero}", &[], hero), Some(pl));
        // Through a fallback too; never for other bindings or another picture.
        assert_eq!(b.image_crop("{collection.hero}", &["{art.hero}".into()], hero), Some(pl));
        assert_eq!(b.image_crop("{art.cover}", &[], Path::new("/c/cover.jpg")), None);
        assert_eq!(b.image_crop("{art.hero}", &["{art.cover}".into()], Path::new("/c/cover.jpg")), None);
        // A game without a hero: the cover standing in for its background is cropped.
        g.art.hero = None;
        let b = Binder::default().with_game(Some(&g));
        assert_eq!(b.image_crop("{art.hero}", &["{art.cover}".into()], Path::new("/c/cover.jpg")), Some(pl));
    }

    #[test]
    fn resolves_bindings() {
        let g = game();
        let c = CollectionItem {
            collection: Collection { kind: CollectionKind::Source(Source::Steam), name: "Steam".into() },
            name: "Steam".into(),
            count: 12,
            art: Art::default(),
        };
        let b = Binder {
            game: Some(&g),
            collection: Some(&c),
            theme_dir: None,
            list: Some((2, 12)),
            section: Some("Sources"),
            focused: false,
            page: false,
            collection_art: None,
        };
        assert_eq!(b.resolve("{list.countlabel} · {section.name}", false), "12 games · Sources");
        assert_eq!(b.resolve("{game.genrelist}", false), "Puzzle|FPS");
        assert_eq!(b.resolve("{game.name} ({game.year})", false), "Portal 2 (2011)");
        assert_eq!(b.resolve("{game.genre}", false), "Puzzle, FPS");
        assert_eq!(b.resolve("{game.playtime}", false), "3h 0m");
        assert_eq!(b.resolve("{game.rating}", false), "0.8");
        assert_eq!(b.resolve("{game.favorite}", false), "true");
        assert_eq!(b.resolve("{game.lastplayed}", true), "1700000000");
        assert_eq!(b.resolve("{collection.countlabel}", false), "12 games");
        assert_eq!(b.resolve("{list.index}/{list.count}", false), "3/12");
        assert_eq!(b.resolve("{game.size}", false), "12.0 GB");
        assert_eq!(b.resolve("{game.playtimelabel}", false), "3h 0m played");
        assert!(b.resolve("{game.lastplayedlabel}", false).starts_with("Last played "));
        assert_eq!(b.resolve("{unknown.key}x", false), "x");
        assert_eq!(b.resolve("plain", false), "plain");
        assert_eq!(b.resolve("broken {game.name", false), "broken {game.name");
        let empty = Binder::default();
        assert_eq!(empty.resolve("{game.name}", false), "");
    }

    #[test]
    fn the_platform_reads_as_a_name_but_names_files_as_stored() {
        let mut g = game();
        g.platform = Some("windows".into());
        let b = Binder { game: Some(&g), ..Default::default() };
        assert_eq!(b.resolve("{game.platform}", false), "Windows");
        assert_eq!(b.resolve("{game.platform}", true), "windows");
        // Theme pictures are named after the stored value (`platform/windows.png`).
        let mut v = vec![];
        b.path_candidates("images/platform/{game.platform}.png", &mut v);
        assert_eq!(v, ["images/platform/windows.png"]);
        g.platform = Some("PS2".into());
        let b = Binder { game: Some(&g), ..Default::default() };
        assert_eq!(b.resolve("{game.platform}", false), "Sony PlayStation 2");
    }

    #[test]
    fn achievement_bindings() {
        let mut g = game();
        g.id = 987_654;
        let b = |g: &Game, t: &str| {
            Binder { game: Some(g), page: true, ..Default::default() }.resolve(t, false).into_owned()
        };
        let all = "{game.achievements}|{game.achievementsunlocked}|{game.achievementstotal}|\
                   {game.achievementspercent}|{game.achievementslabel}";
        // Not looked up yet: everything empty (`visible_if` hides the element).
        assert_eq!(b(&g, all), "||||");
        set_achievements(g.id, Some((12, 40)));
        assert_eq!(b(&g, all), "12 / 40|12|40|0.3|12 of 40 achievements");
        // Never more than all of them.
        set_achievements(g.id, Some((41, 40)));
        assert_eq!(b(&g, "{game.achievements} {game.achievementspercent}"), "40 / 40 1");
        // A game without any reads empty, as does one forgotten again.
        set_achievements(g.id, Some((0, 0)));
        assert_eq!(b(&g, all), "||||");
        set_achievements(g.id, None);
        assert_eq!(b(&g, "{game.achievementslabel}"), "");
    }

    #[test]
    fn lists_show_the_library_count_and_a_page_its_own_answer() {
        let mut g = game();
        g.id = 987_655;
        let list = |g: &Game| {
            Binder { game: Some(g), ..Default::default() }.resolve("{game.achievements}", false).into_owned()
        };
        let page = |g: &Game| {
            Binder { game: Some(g), page: true, ..Default::default() }
                .resolve("{game.achievements}", false)
                .into_owned()
        };
        set_library_achievements(Arc::new(HashMap::from([(g.id, (31, 54))])));
        assert_eq!(list(&g), "31 / 54");
        // The page falls back to the library's count until its own answer is there.
        assert_eq!(page(&g), "31 / 54");
        // A title-only match shows on the page, never in the list.
        set_achievements(g.id, Some((3, 10)));
        assert_eq!(page(&g), "3 / 10");
        assert_eq!(list(&g), "31 / 54");
        // Library counts switched off: the list shows nothing.
        set_library_achievements(Arc::new(HashMap::new()));
        assert_eq!(list(&g), "");
        set_achievements(g.id, None);
    }

    #[test]
    fn controller_and_deck_bindings() {
        let mut g = game();
        let b = |g: &Game, t: &str| Binder { game: Some(g), ..Default::default() }.resolve(t, false).into_owned();
        // Unknown: every binding is empty (chips and `visible_if` drop out).
        assert_eq!(b(&g, "{game.controller}{game.controllerlabel}{game.deck}{game.decklabel}"), "");
        g.meta.controller = Some(ControllerSupport::Partial);
        g.meta.deck = Some(DeckRating::Verified);
        assert_eq!(b(&g, "{game.controller}"), "partial");
        assert_eq!(b(&g, "{game.controllerlabel}"), "Partial controller support");
        assert_eq!(b(&g, "{game.controllershort}"), "Partial support");
        assert_eq!(b(&g, "{game.deck}|{game.decklabel}|{game.deckshort}"), "verified|Deck Verified|Verified");
        // ProtonDB: empty when unknown or not a tier.
        assert_eq!(b(&g, "{game.protondb}{game.protondblabel}"), "");
        g.meta.protondb = Some("Gold".into());
        assert_eq!(b(&g, "{game.protondb}|{game.protondblabel}|{game.protondbshort}"), "gold|ProtonDB Gold|Gold");
        g.meta.protondb = Some("shiny".into());
        assert_eq!(b(&g, "{game.protondb}{game.protondbshort}"), "");
        g.meta.protondb = None;
        // The user's setting wins over Steam's.
        g.controller_override = Some(ControllerSupport::None);
        assert_eq!(b(&g, "{game.controller}"), "none");
        assert_eq!(b(&g, "{game.controllerlabel}"), "No controller support");
    }

    #[test]
    fn completion_and_score_bindings() {
        let mut g = game();
        g.user_rating = None;
        let b = |g: &Game, t: &str| Binder { game: Some(g), ..Default::default() }.resolve(t, false).into_owned();
        // Unset: empty, so chips / `visible_if` drop out.
        assert_eq!(b(&g, "{game.completion}{game.completionkey}{game.score}{game.rating}"), "");
        g.edits.completion = Some(CompletionStatus::PlanToPlay);
        g.edits.user_score = Some(85);
        assert_eq!(b(&g, "{game.completion}"), "Plan to play");
        assert_eq!(b(&g, "{game.completionkey}"), "plan_to_play");
        assert_eq!(b(&g, "{game.score}"), "85");
        assert_eq!(b(&g, "{game.rating}"), "0.85");
        // A star rating still wins.
        g.user_rating = Some(4);
        assert_eq!(b(&g, "{game.rating}"), "0.8");
        // Keys match the serde names (stable across both).
        for c in CompletionStatus::ALL {
            let json = serde_json::to_string(&c).unwrap();
            assert_eq!(json.trim_matches('"'), completion_key(c));
        }
    }

    #[test]
    fn score_and_battery_bindings() {
        let mut g = game();
        let b = |g: &Game, t: &str| Binder { game: Some(g), ..Default::default() }.resolve(t, false).into_owned();
        assert_eq!(b(&g, "{game.scorelabel}"), "");
        g.edits.user_score = Some(70);
        assert_eq!(b(&g, "{game.scorelabel}"), "Your score 70");
        g.meta.community_score = Some(81);
        assert_eq!(b(&g, "{game.scorelabel}|{game.communityscore}"), "Players 81|81");
        g.meta.critic_score = Some(90);
        assert_eq!(b(&g, "{game.scorelabel}"), "Critics 90");
        g.meta.genres = vec!["A".into(), " ".into(), "B".into(), "C".into(), "D".into()];
        assert_eq!(b(&g, "{game.genrelist}"), "A|B|C");
        assert_eq!(Binder::default().resolve("{list.countlabel}", false), "");
        let one = Binder { list: Some((0, 1)), ..Default::default() };
        assert_eq!(one.resolve("{list.countlabel}", false), "1 game");

        let dir = tempfile::tempdir().unwrap();
        let mk = |name: &str, files: &[(&str, &str)]| {
            let d = dir.path().join(name);
            std::fs::create_dir_all(&d).unwrap();
            for (f, v) in files {
                std::fs::write(d.join(f), v).unwrap();
            }
        };
        assert_eq!(read_battery(dir.path()), None);
        mk("AC", &[("type", "Mains\n")]);
        mk("hidpp_battery_0", &[("type", "Battery"), ("scope", "Device"), ("capacity", "40")]);
        assert_eq!(read_battery(dir.path()), None);
        mk("BAT0", &[("type", "Battery\n"), ("capacity", "82\n"), ("status", "Discharging\n")]);
        assert_eq!(read_battery(dir.path()), Some((82, false)));
    }

    #[test]
    fn dates() {
        let s = format_date(1_700_000_000, "%Y-%m-%d");
        assert!(s.starts_with("2023-11-1"), "{s}");
        assert_eq!(format_date(0, "100%%"), "100%");
        assert_eq!(format_size(500), "500 B");
    }
}
