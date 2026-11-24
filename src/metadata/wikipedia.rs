//! Wikipedia descriptions (no key): the lead section of the game's English Wikipedia article.
//!
//! One request per game: the MediaWiki API's full-text search as a generator, with each hit's
//! short description, disambiguation flag, URL and plain-text intro in the same answer:
//! `GET https://en.wikipedia.org/w/api.php?action=query&generator=search&gsrsearch=<name> video game
//! &prop=description|pageprops|info|extracts&exintro=1&explaintext=1&inprop=url …`
//! (`formatversion=2`: `query.pages[{index, title, description, pageprops.disambiguation,
//! fullurl, extract}]`).
//!
//! A hit counts only when it is a video game article whose title (without its "(2016 video
//! game)" part) is the game's name: films, novels, series, characters and soundtracks of the
//! same name are skipped by the title's bracket and the short description. Several games of the
//! same name ("Doom (1993 video game)" / "(2016 video game)") are told apart by the release
//! year, then by the platform the intro names; still ambiguous = no match (descriptions are
//! filled automatically, so only confident matches count).
//!
//! Text is CC BY-SA: the game page says "From Wikipedia" with a link to the article
//! (`Metadata::wikipedia`). Requests follow Wikimedia's API etiquette: a descriptive
//! User-Agent and one request at a time on the metadata thread (its throttle spaces them).

use crate::metadata::parse::{encode_path_segment, search_name};
use serde_json::Value;

pub const API: &str = "https://en.wikipedia.org/w/api.php";
/// Licence of Wikipedia's text (the credit links to it).
pub const LICENCE_URL: &str = "https://creativecommons.org/licenses/by-sa/4.0/";
/// The credit appended where a description can only be plain text (themes, controller mode).
pub const CREDIT_LINE: &str = "\n\n(From Wikipedia, text under CC BY-SA 4.0)";
/// Set for screenshot runs: Wikipedia is never asked (they must not reach the network).
pub static OFF: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Wikipedia may be asked in this process.
pub fn allowed() -> bool {
    !OFF.load(std::sync::atomic::Ordering::Relaxed)
}

/// Hits looked at per search.
const HITS: usize = 10;
/// Longest description kept (characters; the lead of a long article can run to a few pages).
const MAX_CHARS: usize = 2400;

/// The search for `name` (one request; everything needed comes back with the hits).
pub fn search_url(name: &str) -> String {
    let q = format!("{} video game", search_name(name));
    format!(
        "{API}?action=query&format=json&formatversion=2&redirects=1&generator=search&gsrsearch={}\
         &gsrlimit={HITS}&gsrnamespace=0&prop=description%7Cpageprops%7Cinfo%7Cextracts\
         &ppprop=disambiguation&inprop=url&exintro=1&explaintext=1&exlimit={HITS}",
        encode_path_segment(q.trim())
    )
}

/// One search hit.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub title: String,
    /// Short description ("2017 video game", "1985 American film").
    pub description: String,
    pub url: String,
    /// Plain-text lead section.
    pub extract: String,
    pub disambiguation: bool,
    /// Search rank (1 = best).
    pub index: u32,
}

/// The search answer → hits by rank. None = not an answer of this API; an empty list = nothing
/// found (no `query`).
pub fn parse_search(body: &[u8]) -> Option<Vec<Page>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    if v.get("error").is_some() {
        return None;
    }
    let Some(q) = v.get("query") else {
        // No hits: the API leaves `query` out.
        return v.get("batchcomplete").map(|_| vec![]);
    };
    let pages = q.get("pages")?.as_array()?;
    let mut out: Vec<Page> = pages
        .iter()
        .filter(|p| p.get("missing").is_none() && p.get("invalid").is_none())
        .filter_map(|p| {
            let s = |k: &str| p.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
            let title = s("title");
            if title.is_empty() {
                return None;
            }
            let url = Some(s("fullurl"))
                .filter(|u| u.starts_with("https://en.wikipedia.org/"))
                .unwrap_or_else(|| article_url(&title));
            Some(Page {
                description: s("description"),
                url,
                extract: s("extract"),
                disambiguation: p.get("pageprops").and_then(|pp| pp.get("disambiguation")).is_some(),
                index: p.get("index").and_then(Value::as_u64).unwrap_or(u64::from(u32::MAX)) as u32,
                title,
            })
        })
        .collect();
    out.sort_by_key(|p| p.index);
    Some(out)
}

/// `https://en.wikipedia.org/wiki/<Title_with_underscores>`.
pub fn article_url(title: &str) -> String {
    format!("https://en.wikipedia.org/wiki/{}", encode_path_segment(&title.replace(' ', "_")))
}

/// "Doom (1993 video game)" → ("Doom", Some("1993 video game")).
pub fn split_title(title: &str) -> (&str, Option<&str>) {
    let t = title.trim();
    if t.ends_with(')')
        && let Some(o) = t.rfind(" (")
    {
        return (t[..o].trim_end(), Some(&t[o + 2..t.len() - 1]));
    }
    (t, None)
}

fn has_word(hay: &str, word: &str) -> bool {
    hay.match_indices(word).any(|(i, _)| {
        let before = hay[..i].chars().next_back();
        let after = hay[i + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Not the game itself: a series, a character, a film … of the same name.
const NOT_A_GAME: &[&str] = &[
    "series",
    "franchise",
    "character",
    "soundtrack",
    "album",
    "film",
    "novel",
    "book",
    "comic",
    "manga",
    "anime",
    "television",
    "tv series",
    "song",
    "single",
    "board game",
    "card game",
    "tabletop",
    "game show",
    "developer",
    "company",
    "publisher",
    "studio",
    "list of",
    "magazine",
    "engine",
    "console",
    "genre",
    "musician",
    "band",
    "actor",
    "actress",
    "politician",
    "footballer",
    "player",
    "programmer",
    "designer",
    "disambiguation",
    "topics referred to",
    "website",
    "award",
    "convention",
    "tournament",
    "esports",
    "fictional",
    "modding",
    "toy",
    "theme park",
    "attraction",
];
/// Words that, with "game", make a short description a video game ("First-person shooter game").
const GAME_WORDS: &[&str] = &[
    "shooter",
    "role-playing",
    "platform",
    "platformer",
    "puzzle",
    "adventure",
    "racing",
    "fighting",
    "strategy",
    "simulation",
    "sports",
    "rhythm",
    "survival",
    "horror",
    "stealth",
    "roguelike",
    "roguelite",
    "sandbox",
    "action",
    "metroidvania",
    "party",
    "arcade",
    "indie",
    "mmorpg",
    "open world",
    "hack and slash",
];

/// A video game article (not a series / film / character … of the same name).
pub fn is_video_game(p: &Page) -> bool {
    if p.disambiguation {
        return false;
    }
    let mut d = p.description.to_lowercase();
    // "Single-player" / "multiplayer" describe a game, not a song single or a sports player.
    for (bad, good) in [
        ("single-player", "singleplayer"),
        ("single player", "singleplayer"),
        ("multi-player", "multiplayer"),
        ("two-player", "twoplayer"),
    ] {
        d = d.replace(bad, good);
    }
    let paren = split_title(&p.title).1.map(str::to_lowercase);
    // "visual novel" is a game even though "novel" isn't.
    let d_clean = d.replace("visual novel", "visualnovel game");
    // What the article is: the description before "in the …" / "based on …" ("2004 video game
    // based on the film", "Video game in the Mario series" are games; "Character in the 2018
    // video game Celeste", "Film based on the video game" aren't).
    let head = [" in the ", " based on ", " from the ", " by ", " for ", " of the ", " set in ", " featuring "]
        .iter()
        .filter_map(|c| d_clean.find(c))
        .min()
        .map_or(d_clean.as_str(), |i| &d_clean[..i]);
    let rejected = |s: &str| NOT_A_GAME.iter().any(|w| has_word(s, w));
    if let Some(par) = &paren {
        if rejected(par) {
            return false;
        }
        if par.contains("video game") || par == "game" {
            // "(video game)" settles it unless the short description says otherwise.
            return !rejected(head);
        }
    }
    if d_clean.is_empty() || rejected(head) {
        return false;
    }
    d_clean.contains("video game")
        || d_clean.contains("visualnovel game")
        || (has_word(&d_clean, "game") && GAME_WORDS.iter().any(|w| has_word(&d_clean, w)))
        || (has_word(&d_clean, "game") && year_in(&d_clean).is_some())
}

/// The first plausible release year (1950..=2100) in `s`.
fn year_in(s: &str) -> Option<i32> {
    let b = s.as_bytes();
    (0..b.len().saturating_sub(3)).find_map(|i| {
        let digits = b[i..i + 4].iter().all(u8::is_ascii_digit);
        let edge_l = i == 0 || !b[i - 1].is_ascii_alphanumeric();
        let edge_r = i + 4 == b.len() || !b[i + 4].is_ascii_alphanumeric();
        if !(digits && edge_l && edge_r) {
            return None;
        }
        let y: i32 = s[i..i + 4].parse().ok()?;
        (1950..=2100).contains(&y).then_some(y)
    })
}

/// The year the article gives: in its title's bracket, its short description, or the first
/// sentence of the intro ("… is a 2007 platform game …").
pub fn page_year(p: &Page) -> Option<i32> {
    if let Some(y) = split_title(&p.title).1.and_then(year_in) {
        return Some(y);
    }
    if let Some(y) = year_in(&p.description) {
        return Some(y);
    }
    let first = p.extract.split(". ").next().unwrap_or("");
    year_in(first)
}

/// Words of a platform name the intro would use ("Nintendo Wii" → "Wii", "Sony PlayStation 2"
/// → "PlayStation 2", "PC (Windows)" → "Windows").
pub fn platform_words(platform: &str) -> Vec<String> {
    let p = platform.trim();
    if p.is_empty() {
        return vec![];
    }
    let lower = p.to_lowercase();
    if lower.starts_with("pc") || lower.contains("windows") {
        return vec!["Windows".into(), "Microsoft Windows".into()];
    }
    if lower.contains("linux") {
        return vec!["Linux".into()];
    }
    let mut out = vec![p.to_string()];
    for brand in ["Nintendo ", "Sony ", "Sega ", "Microsoft ", "SNK ", "NEC ", "Atari ", "Bandai "] {
        if let Some(rest) = p.strip_prefix(brand).filter(|r| !r.is_empty()) {
            out.push(rest.to_string());
        }
    }
    out
}

fn mentions_platform(p: &Page, words: &[String]) -> bool {
    words.iter().any(|w| {
        let hay = &p.extract;
        hay.match_indices(w.as_str()).any(|(i, _)| {
            let before = hay[..i].chars().next_back();
            let rest = &hay[i + w.len()..];
            // "Wii" must not match "Wii U".
            let after_ok = !rest.chars().next().is_some_and(char::is_alphanumeric)
                && !(w.ends_with("Wii") && rest.starts_with(" U"));
            !before.is_some_and(char::is_alphanumeric) && after_ok
        })
    })
}

/// What the game is known as.
#[derive(Clone, Debug, Default)]
pub struct Want<'a> {
    pub name: &'a str,
    pub year: Option<i32>,
    pub platform: Option<&'a str>,
}

/// The game's article among search hits (confident matches only).
pub fn pick<'a>(pages: &'a [Page], want: &Want) -> Option<&'a Page> {
    let mut c: Vec<&Page> = pages
        .iter()
        .filter(|p| is_video_game(p) && crate::art::sources::same_name(split_title(&p.title).0, want.name))
        .collect();
    if c.len() <= 1 {
        // One article of that name: a year that says otherwise (off by more than a regional
        // release gap) means it's another game.
        return c.into_iter().next().filter(|p| match (want.year, page_year(p)) {
            (Some(w), Some(y)) => (w - y).abs() <= 1,
            _ => true,
        });
    }
    if let Some(w) = want.year {
        let exact: Vec<&Page> = c.iter().copied().filter(|p| page_year(p) == Some(w)).collect();
        c = if exact.is_empty() {
            c.into_iter().filter(|p| page_year(p).is_some_and(|y| (w - y).abs() <= 1)).collect()
        } else {
            exact
        };
        if c.len() <= 1 {
            return c.into_iter().next();
        }
    }
    if let Some(words) = want.platform.map(platform_words).filter(|w| !w.is_empty()) {
        let on: Vec<&Page> = c.iter().copied().filter(|p| mentions_platform(p, &words)).collect();
        if on.len() == 1 {
            return on.into_iter().next();
        }
        if !on.is_empty() {
            c = on;
        }
    }
    // The plain title is Wikipedia's primary topic of that name (the original game) — only
    // when the year can't say which one is meant.
    let plain: Vec<&Page> = c.iter().copied().filter(|p| split_title(&p.title).1.is_none()).collect();
    if want.year.is_none() && plain.len() == 1 {
        return plain.into_iter().next();
    }
    None
}

/// A letter of a script other than Latin / Greek / Cyrillic (a native name or reading in
/// brackets: "Zeruda no Densetsu" asides in Japanese, Korean, Chinese …).
fn foreign_script(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xAC00..=0xD7AF | 0xF900..=0xFAFF
        | 0xFF66..=0xFF9F | 0x0600..=0x06FF | 0x0590..=0x05FF | 0x0E00..=0x0E7F)
}

/// The intro as a tidy description: native-name asides and the empty brackets left by removed
/// pronunciations dropped, spacing fixed, paragraphs kept (blank line between), and cut at a
/// sentence end when very long.
pub fn clean_extract(s: &str) -> String {
    // Brackets: drop the ones with foreign script or nothing but punctuation left.
    let mut out = String::with_capacity(s.len());
    let mut stack: Vec<usize> = vec![];
    for c in s.chars() {
        match c {
            '(' => {
                stack.push(out.len());
                out.push(c);
            }
            ')' => {
                if let Some(start) = stack.pop() {
                    let inner = &out[start + 1..];
                    let empty = !inner.chars().any(char::is_alphanumeric);
                    if empty || inner.chars().any(foreign_script) {
                        out.truncate(start);
                        continue;
                    }
                }
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    let paras: Vec<String> = out
        .split('\n')
        .map(|l| {
            let mut l = l.split_whitespace().collect::<Vec<_>>().join(" ");
            for (bad, good) in [(" ,", ","), (" .", "."), (" ;", ";"), (" :", ":"), ("( ", "("), (" )", ")")] {
                while l.contains(bad) {
                    l = l.replace(bad, good);
                }
            }
            l
        })
        .filter(|l| !l.is_empty() && !l.starts_with("=="))
        .collect();
    let mut text = paras.join("\n\n");
    if text.chars().count() > MAX_CHARS {
        let cut: String = text.chars().take(MAX_CHARS).collect();
        text = match cut.rfind(". ").or_else(|| cut.rfind(".\n")) {
            Some(i) if i > MAX_CHARS / 3 => cut[..=i].to_string(),
            _ => format!("{}…", cut.trim_end()),
        };
    }
    text.trim().to_string()
}

/// The description for the game from a search answer: (text, article URL). None = no
/// confident match (or an article without an intro).
pub fn description_from(body: &[u8], want: &Want) -> Option<Option<(String, String)>> {
    let pages = parse_search(body)?;
    Some(pick(&pages, want).and_then(text_of))
}

/// A page's intro as a description and its URL (None for a stub intro).
pub fn text_of(p: &Page) -> Option<(String, String)> {
    let text = clean_extract(&p.extract);
    // A stub intro ("X is a video game.") says nothing the page doesn't.
    (text.len() >= 40).then(|| (text, p.url.clone()))
}

/// The video game articles a lookup the user asked for offers when `pick` can't tell: the
/// same-named ones, else any game article among the hits (search order).
pub fn candidates<'a>(pages: &'a [Page], want: &Want) -> Vec<&'a Page> {
    let games: Vec<&Page> = pages.iter().filter(|p| is_video_game(p) && text_of(p).is_some()).collect();
    let same: Vec<&Page> =
        games.iter().copied().filter(|p| crate::art::sources::same_name(split_title(&p.title).0, want.name)).collect();
    if same.is_empty() { games } else { same }
}

// ---------------------------------------------------------------------------------------------
// Games looked up before Wikipedia was a source (AppState::queue_wikipedia_backfill)

/// Which games had their Wikipedia-only background lookup (DB `kv`, so each game is asked once;
/// switching "Wikipedia first" on starts the list over).
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct BackfillDone {
    /// `prefer_wikipedia` the list is for.
    pub prefer: bool,
    pub done: std::collections::HashSet<crate::model::GameId>,
}

/// The app side of the backfill: off in screenshot / test runs.
#[derive(Debug, Default)]
pub struct Backfill {
    pub enabled: bool,
    pub state: Option<BackfillDone>,
    /// `prefer_wikipedia` the games were queued for this session.
    pub queued_for: Option<bool>,
}

pub const BACKFILL_KV: &str = "wikipedia_backfill";

impl crate::app::AppState {
    /// Give every game looked up before (that Wikipedia could add a description to) one
    /// Wikipedia-only background lookup; never-looked-up games go through `queue_metadata`.
    pub(crate) fn queue_wikipedia_backfill(&mut self) {
        let m = &self.config.metadata;
        if !self.wiki_backfill.enabled || !m.wikipedia || self.library.games.is_empty() {
            return;
        }
        let prefer = m.prefer_wikipedia;
        if self.wiki_backfill.queued_for == Some(prefer) {
            return;
        }
        self.wiki_backfill.queued_for = Some(prefer);
        let db = &self.db;
        let st = self.wiki_backfill.state.get_or_insert_with(|| {
            db.kv_get(BACKFILL_KV).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
        });
        if st.prefer != prefer {
            // "Wikipedia first" switched on: every game gets asked again (switched off: the
            // games already asked stay asked).
            if prefer {
                st.done.clear();
            }
            st.prefer = prefer;
        }
        for g in &self.library.games {
            if !g.hidden && !st.done.contains(&g.id) && matches!(self.db.fetched_at(g.id), Ok(Some(_))) {
                self.metadata.request_wikipedia_backfill(g);
            }
        }
    }

    pub(crate) fn wikipedia_backfill_done(&mut self, id: crate::model::GameId) {
        let Some(st) = self.wiki_backfill.state.as_mut() else {
            return;
        };
        if st.done.insert(id) {
            match serde_json::to_string(&*st) {
                Ok(s) => {
                    if let Err(e) = self.db.kv_set(BACKFILL_KV, &s) {
                        log::warn!("Wikipedia: saving lookups: {e:#}");
                    }
                }
                Err(e) => log::warn!("Wikipedia: saving lookups: {e}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(title: &str, desc: &str, extract: &str, index: u32) -> Page {
        Page {
            title: title.into(),
            description: desc.into(),
            url: article_url(title),
            extract: extract.into(),
            disambiguation: false,
            index,
        }
    }

    fn fixture() -> &'static [u8] {
        include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/metadata/wikipedia_search_doom.json"))
    }

    #[test]
    fn search_url_is_one_encoded_request() {
        let u = search_url("DOOM™ (2016)");
        assert!(u.starts_with("https://en.wikipedia.org/w/api.php?action=query"), "{u}");
        assert!(u.contains("gsrsearch=DOOM%20video%20game"), "{u}");
        assert!(u.contains("exintro=1") && u.contains("explaintext=1") && u.contains("inprop=url"), "{u}");
        assert!(!u.contains(' '));
    }

    #[test]
    fn parses_hits_in_rank_order() {
        let pages = parse_search(fixture()).unwrap();
        assert_eq!(pages[0].title, "Doom (franchise)");
        assert_eq!(pages[1].title, "Doom (1993 video game)");
        assert_eq!(pages[1].url, "https://en.wikipedia.org/wiki/Doom_(1993_video_game)");
        assert!(pages.iter().any(|p| p.disambiguation && p.title == "Doom"));
        // Nothing found: no `query`; not this API: None.
        assert_eq!(parse_search(br#"{"batchcomplete":true}"#), Some(vec![]));
        assert_eq!(parse_search(br#"{"error":{"code":"x"}}"#), None);
        assert_eq!(parse_search(b"<html>"), None);
    }

    #[test]
    fn titles_split_off_their_bracket() {
        assert_eq!(split_title("Doom (1993 video game)"), ("Doom", Some("1993 video game")));
        assert_eq!(split_title("Doom II"), ("Doom II", None));
        assert_eq!(split_title("Madeline (Celeste)"), ("Madeline", Some("Celeste")));
    }

    #[test]
    fn only_video_game_articles_count() {
        let yes = [
            page("Control (video game)", "2019 video game", "", 1),
            page("Doom (1993 video game)", "First-person shooter game", "", 1),
            page("Doom (2016 video game)", "First-person shooter", "", 1),
            page("Super Mario Galaxy", "2007 video game", "", 1),
            page("Steins;Gate", "2009 visual novel", "", 1),
            page("Tetris (Game Boy video game)", "", "", 1),
            page("Hades (video game)", "2020 roguelike action game", "", 1),
            page("Spider-Man 2 (video game)", "2004 video game based on the film", "", 1),
            page("Prince of Persia: The Sands of Time", "2003 video game in the Prince of Persia series", "", 1),
            page("Portal (video game)", "2007 single-player puzzle-platform video game", "", 1),
            page("Tetris DS", "2006 video game for the Nintendo DS", "", 1),
        ];
        for p in &yes {
            assert!(is_video_game(p), "{}", p.title);
        }
        let no = [
            page("Doom (franchise)", "American media franchise", "", 1),
            page("Doom (film)", "2005 film by Andrzej Bartkowiak", "", 1),
            page("The Super Mario Galaxy Movie", "2026 American animated film", "", 1),
            page("Super Mario", "Video game series", "", 1),
            page("Madeline (Celeste)", "Character in the 2018 video game Celeste", "", 1),
            page("Super Mario Galaxy Original Soundtrack", "2008 soundtrack album", "", 1),
            page("Remote Control (game show)", "Television series", "", 1),
            page("Maddy Thorson", "Canadian video game developer (born 1988)", "", 1),
            page("Catan", "Board game", "", 1),
            page("Prey (novel)", "1980s science fiction novel", "", 1),
            page("Celeste", "Topics referred to by the same term", "", 1),
            page("Glossary of video game terms", "", "", 1),
            page("Doom (2005 film)", "Film based on the video game series", "", 1),
            page("Max Payne (film)", "2008 film based on the video game", "", 1),
            page("Professor Layton", "Video game series by Level-5", "", 1),
            page("Dawn (character)", "Fictional character from the Pokémon video game series", "", 1),
            page("Daigo Umehara", "Japanese professional video game player", "", 1),
        ];
        for p in &no {
            assert!(!is_video_game(p), "{}", p.title);
        }
        let mut d = page("Doom", "", "", 1);
        d.disambiguation = true;
        assert!(!is_video_game(&d));
    }

    #[test]
    fn same_named_games_are_told_apart_by_year_then_platform() {
        let pages = parse_search(fixture()).unwrap();
        let pick_for = |year, platform| pick(&pages, &Want { name: "DOOM", year, platform }).map(|p| p.title.as_str());
        assert_eq!(pick_for(Some(1993), None), Some("Doom (1993 video game)"));
        assert_eq!(pick_for(Some(2016), None), Some("Doom (2016 video game)"));
        // A regional release a year later still counts.
        assert_eq!(pick_for(Some(1994), None), Some("Doom (1993 video game)"));
        // Two games of that name and nothing to choose by: no match.
        assert_eq!(pick_for(None, None), None);
        assert_eq!(pick_for(Some(2001), None), None);
        // The intro's platform decides when the year can't.
        assert_eq!(pick_for(None, Some("PC (Windows)")), Some("Doom (2016 video game)"));
        assert_eq!(pick_for(None, Some("MS-DOS")), Some("Doom (1993 video game)"));
        // "Doom II" is another game; the franchise / film aren't games.
        assert_eq!(
            pick(&pages, &Want { name: "Doom II", ..Default::default() }).map(|p| p.title.as_str()),
            Some("Doom II")
        );
        assert_eq!(pick(&pages, &Want { name: "Doom 3", ..Default::default() }), None);
    }

    #[test]
    fn a_single_article_must_not_contradict_the_year() {
        let pages = vec![
            page("Super Mario Galaxy", "2007 video game", "Super Mario Galaxy is a 2007 platform game for the Wii.", 2),
            page("The Super Mario Galaxy Movie", "2026 American animated film", "", 1),
        ];
        let w = |year| Want { name: "Super Mario Galaxy", year, platform: Some("Nintendo Wii") };
        assert_eq!(pick(&pages, &w(Some(2007))).map(|p| p.index), Some(2));
        assert_eq!(pick(&pages, &w(None)).map(|p| p.index), Some(2));
        assert_eq!(pick(&pages, &w(Some(2015))), None);
    }

    #[test]
    fn the_plain_title_wins_only_without_a_year() {
        let pages = vec![
            page("Prince of Persia", "1989 video game", "Prince of Persia is a 1989 game for the Apple II.", 1),
            page("Prince of Persia (2008 video game)", "2008 video game", "Prince of Persia is a 2008 game.", 2),
        ];
        let w = |year| Want { name: "Prince of Persia", year, platform: None };
        assert_eq!(pick(&pages, &w(None)).map(|p| p.index), Some(1));
        assert_eq!(pick(&pages, &w(Some(2008))).map(|p| p.index), Some(2));
    }

    #[test]
    fn platform_words_and_wii_u() {
        assert_eq!(platform_words("Nintendo Wii"), vec!["Nintendo Wii".to_string(), "Wii".to_string()]);
        assert!(platform_words("Sony PlayStation 2").contains(&"PlayStation 2".to_string()));
        let wiiu = page("X", "", "X is a 2013 game for the Wii U.", 1);
        let wii = page("X", "", "X is a 2008 game for the Wii.", 1);
        let words = platform_words("Nintendo Wii");
        assert!(!mentions_platform(&wiiu, &words));
        assert!(mentions_platform(&wii, &words));
    }

    #[test]
    fn extracts_are_tidied() {
        let raw = "Doom  is a 1993 first-person shooter game ( ; ) developed by id Software .\n\
                   It was released (Japanese: ドゥーム, Hepburn: Dūmu) in Japan later.\n\n\n\
                   Third paragraph (the sequel) follows.";
        assert_eq!(
            clean_extract(raw),
            "Doom is a 1993 first-person shooter game developed by id Software.\n\n\
             It was released in Japan later.\n\nThird paragraph (the sequel) follows."
        );
        // Very long intros are cut at a sentence end.
        let long = "This is a sentence about the game. ".repeat(200);
        let c = clean_extract(&long);
        assert!(c.chars().count() <= MAX_CHARS && c.ends_with('.'), "{}", c.len());
        assert_eq!(clean_extract("  "), "");
    }

    #[test]
    fn description_from_the_answer() {
        let (text, url) =
            description_from(fixture(), &Want { name: "Doom", year: Some(1993), platform: None }).unwrap().unwrap();
        assert!(text.starts_with("Doom is a 1993 first-person shooter game"), "{text}");
        assert!(!text.contains("  "));
        assert_eq!(url, "https://en.wikipedia.org/wiki/Doom_(1993_video_game)");
        assert_eq!(description_from(fixture(), &Want { name: "Quake", ..Default::default() }), Some(None));
        assert_eq!(description_from(b"nope", &Want::default()), None);
    }
}
