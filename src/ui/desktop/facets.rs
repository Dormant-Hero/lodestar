//! The library facets (filter dimensions: platforms, genres, companies, features, …), their
//! values and the per-value counts the explorer shows.

use super::*;

/// A Sources entry: a library source ("steam"), or a cloud service ("cloud-gfn"). Cloud games are
/// hand-added games, but they are listed by service like the stores (`Game::source_group_key`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct SrcKey(pub &'static str);

impl SrcKey {
    pub fn of(g: &crate::model::Game) -> SrcKey {
        SrcKey(g.source_group_key())
    }

    #[cfg(test)]
    pub fn source(s: Source) -> SrcKey {
        SrcKey(s.key())
    }

    /// A known key (a saved explorer state, a smart collection's rule); None for others.
    pub fn from_key(k: &str) -> Option<SrcKey> {
        Source::from_key(k)
            .map(|s| SrcKey(s.key()))
            .or_else(|| crate::cloud::Service::ALL.into_iter().map(|s| s.group_key()).find(|g| *g == k).map(SrcKey))
    }

    /// The library source this is ("cloud-gfn": none).
    pub fn as_source(self) -> Option<Source> {
        Source::from_key(self.0)
    }

    /// "Steam", "GeForce NOW".
    pub fn label(self) -> &'static str {
        match self.as_source() {
            Some(s) => s.display(),
            None => {
                crate::cloud::Service::ALL.into_iter().find(|s| s.group_key() == self.0).map_or(self.0, |s| s.name())
            }
        }
    }

    pub fn matches(self, g: &crate::model::Game) -> bool {
        g.source_group_key() == self.0
    }
}

/// One filter value: a detail clicked on the game page, or a removable chip above the games.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FilterValue {
    /// A facet value (display spelling; matched case-insensitively).
    Facet(Facet, String),
    Source(SrcKey),
    Tag(String),
    /// One of the user's collections (by id).
    Collection(u64),
}

/// Metadata fields the explorer can filter by (Playnite's Genre / Platform / Features /
/// Developer / Publisher sections), in explorer order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Facet {
    Genre,
    Platform,
    /// Controller support and the Steam Deck rating (fixed values, see [`FEATURES`]).
    Feature,
    Developer,
    Publisher,
    /// The user's completion status ("Playing", "Beaten" …).
    Completion,
    /// Release year (no explorer section: set by clicking the year on a game page).
    Year,
    /// Time played, in buckets ([`PLAYTIME`]).
    Playtime,
    /// Install size, in buckets ([`SIZES`]).
    Size,
    /// The user's score in stars ([`SCORES`]).
    Score,
}

/// Number of [`Facet`]s.
pub(crate) const N_FACETS: usize = 10;

/// "Time played" values, in display order (see [`playtime_bucket`]).
pub(crate) const PLAYTIME: [&str; 5] = ["Never", "Under 1 h", "1–10 h", "10–50 h", "50 h or more"];

/// "Install size" values, in display order (see [`size_bucket`]).
pub(crate) const SIZES: [&str; 5] = ["Under 1 GB", "1–10 GB", "10–50 GB", "50–100 GB", "100 GB or more"];

/// "User score" values, in display order (see [`score_bucket`]).
pub(crate) const SCORES: [&str; 6] = ["★★★★★", "★★★★", "★★★", "★★", "★", "Not rated"];

/// The [`PLAYTIME`] bucket of a play time.
pub(crate) fn playtime_bucket(secs: u64) -> &'static str {
    const H: u64 = 3600;
    PLAYTIME[match secs {
        0 => 0,
        s if s < H => 1,
        s if s < 10 * H => 2,
        s if s < 50 * H => 3,
        _ => 4,
    }]
}

/// The [`SIZES`] bucket of an install size (None: size not known, or 0).
pub(crate) fn size_bucket(bytes: Option<u64>) -> Option<&'static str> {
    const GB: u64 = 1 << 30;
    let b = bytes.filter(|&b| b > 0)?;
    Some(
        SIZES[match b {
            b if b < GB => 0,
            b if b < 10 * GB => 1,
            b if b < 50 * GB => 2,
            b if b < 100 * GB => 3,
            _ => 4,
        }],
    )
}

/// The user's score in stars (1..=5): the star rating, else the 0–100 score from Edit Game
/// Details (1–20 → ★, 21–40 → ★★ … 81–100 → ★★★★★). None: not rated.
pub(crate) fn user_stars(g: &Game) -> Option<u8> {
    if let Some(r) = g.user_rating.filter(|&r| r > 0) {
        return Some(r.min(5));
    }
    g.edits.user_score.map(|s| s.min(100).div_ceil(20).max(1))
}

/// The [`SCORES`] bucket of a game.
pub(crate) fn score_bucket(g: &Game) -> &'static str {
    match user_stars(g) {
        Some(n) => SCORES[5 - n as usize],
        None => SCORES[5],
    }
}

/// The explorer's "Features" values, in display order.
pub(crate) const FEATURES: [&str; 4] =
    ["Full controller support", "Partial controller support", "Steam Deck Verified", "Steam Deck Playable"];

impl Facet {
    /// In explorer order.
    pub const ALL: [Facet; N_FACETS] = [
        Facet::Genre,
        Facet::Platform,
        Facet::Feature,
        Facet::Completion,
        Facet::Playtime,
        Facet::Score,
        Facet::Size,
        Facet::Developer,
        Facet::Publisher,
        Facet::Year,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// Has a section in the explorer (the release year is only a chip above the games).
    pub fn in_explorer(self) -> bool {
        self != Facet::Year
    }

    /// A game can have several values of this field (genres, features, companies), so
    /// "Match all" means something for it. The others hold one value per game: picking two of
    /// them always means either one.
    pub fn multi_valued(self) -> bool {
        matches!(self, Facet::Genre | Facet::Feature | Facet::Developer | Facet::Publisher)
    }

    /// Values in a fixed, natural order (not alphabetical) in the explorer.
    fn fixed_order(self) -> Option<&'static [&'static str]> {
        match self {
            Facet::Playtime => Some(&PLAYTIME),
            Facet::Size => Some(&SIZES),
            Facet::Score => Some(&SCORES),
            _ => None,
        }
    }

    /// Singular name for a filter chip ("Genre: RPG").
    pub fn noun(self) -> &'static str {
        match self {
            Facet::Genre => "Genre",
            Facet::Platform => "Platform",
            Facet::Feature => "Feature",
            Facet::Developer => "Developer",
            Facet::Publisher => "Publisher",
            Facet::Completion => "Status",
            Facet::Year => "Released",
            Facet::Playtime => "Played",
            Facet::Size => "Size",
            Facet::Score => "Score",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Facet::Genre => "GENRES",
            Facet::Platform => "PLATFORMS",
            Facet::Feature => "FEATURES",
            Facet::Developer => "DEVELOPERS",
            Facet::Publisher => "PUBLISHERS",
            Facet::Completion => "COMPLETION",
            Facet::Year => "RELEASE YEAR",
            Facet::Playtime => "PLAY TIME",
            Facet::Size => "INSTALL SIZE",
            Facet::Score => "USER SCORE",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Facet::Genre => "genre",
            Facet::Platform => "platform",
            Facet::Feature => "feature",
            Facet::Developer => "developer",
            Facet::Publisher => "publisher",
            Facet::Completion => "completion",
            Facet::Year => "year",
            Facet::Playtime => "playtime",
            Facet::Size => "size",
            Facet::Score => "score",
        }
    }

    /// Call `f` with each value of this field for `g` (display spelling).
    pub fn for_each(self, g: &Game, mut f: impl FnMut(&str)) {
        match self {
            Facet::Genre => g.meta.genres.iter().for_each(|s| f(s.trim())),
            Facet::Platform => {
                if let Some(p) = g.platform.as_deref() {
                    f(platform_label(p))
                }
            }
            Facet::Feature => {
                match g.controller() {
                    Some(ControllerSupport::Full) => f(FEATURES[0]),
                    Some(ControllerSupport::Partial) => f(FEATURES[1]),
                    _ => {}
                }
                match g.meta.deck {
                    Some(DeckRating::Verified) => f(FEATURES[2]),
                    Some(DeckRating::Playable) => f(FEATURES[3]),
                    _ => {}
                }
                // The user's own features (Edit Game Details → General), else IGDB's modes.
                g.edits.features.as_ref().unwrap_or(&g.meta.features).iter().for_each(|s| f(s.trim()));
            }
            Facet::Developer => split_companies(g.meta.developer.as_deref(), f),
            Facet::Publisher => split_companies(g.meta.publisher.as_deref(), f),
            Facet::Completion => {
                if let Some(c) = g.completion() {
                    f(c.label())
                }
            }
            Facet::Year => {
                if let Some(y) = g.meta.release_year {
                    f(&y.to_string())
                }
            }
            Facet::Playtime => f(playtime_bucket(g.playtime_secs)),
            Facet::Size => {
                if let Some(b) = size_bucket(g.size_bytes) {
                    f(b)
                }
            }
            Facet::Score => f(score_bucket(g)),
        }
    }

    /// [`Self::matches`], or with `all` (Match all) every one of `wanted` for the fields a game
    /// can have several values of ([`Self::multi_valued`]).
    pub fn matches_mode(self, g: &Game, wanted: &[String], all: bool) -> bool {
        if !all || !self.multi_valued() || wanted.len() < 2 {
            return self.matches(g, wanted);
        }
        wanted.iter().all(|w| self.matches(g, std::slice::from_ref(w)))
    }

    /// Does `g` have any of `wanted` (lowercase) for this field?
    pub fn matches(self, g: &Game, wanted: &[String]) -> bool {
        // PLATFORMS → PC: every PC game, whatever its own platform says (Linux, Windows …).
        if self == Facet::Platform && wanted.iter().any(|w| w == PC_KEY) && is_pc(g) {
            return true;
        }
        if self == Facet::Platform && wanted.iter().any(|w| w == PC_OTHER_KEY) && is_pc_other(g) {
            return true;
        }
        let mut hit = false;
        self.for_each(g, |v| {
            if hit {
                return;
            }
            hit = if v.is_ascii() {
                wanted.iter().any(|w| w.eq_ignore_ascii_case(v))
            } else {
                let lower = v.to_lowercase();
                wanted.contains(&lower)
            };
        });
        hit
    }
}

/// Key of the combined "PC" entry at the top of PLATFORMS (a game whose platform is set to
/// "PC" is in it too).
pub(crate) const PC_KEY: &str = "pc";

/// Key of PLATFORMS → PC → "Other": the PC games none of the entries under PC covers (no
/// platform set, or just "PC"; often store games that aren't installed). Without it PC's count
/// was more than Linux + Windows with no way to see the rest.
pub(crate) const PC_OTHER_KEY: &str = "pc:other";

/// A PC game whose platform names no PC platform of its own ([`PC_OTHER_KEY`]).
pub(crate) fn is_pc_other(g: &Game) -> bool {
    is_pc(g) && g.platform.as_deref().map(str::trim).is_none_or(|p| p.is_empty() || p.eq_ignore_ascii_case("pc"))
}

/// A game of the PLATFORMS → PC entry: a PC game (not emulated, not from a ROM server, not
/// streamed: [`crate::library::is_pc_game`], the controller's "PC Games") whose platform, when
/// set, is a PC one (Linux, Windows, macOS, PC, a browser) — not a console name.
pub(crate) fn is_pc(g: &Game) -> bool {
    crate::library::is_pc_game(g)
        && g.platform.as_deref().is_none_or(|p| crate::ui::platform_icons::is_pc_label(platform_label(p)))
}

pub(crate) use crate::emulation::platforms::display_name as platform_label;

/// A platform as running text names it: a console without its maker ("Sony PlayStation 2" →
/// "PlayStation 2", like the game page's Console fact and Settings → Emulation), anything
/// else as [`platform_label`] spells it. The explorer's short form ("PS2") is only for the
/// narrow sidebar.
pub(crate) fn platform_display(p: &str) -> String {
    let full = platform_label(p);
    match crate::emulation::platforms::by_name(full).filter(|pl| !pl.id.starts_with("pc_")) {
        Some(pl) => crate::emulation::console_label(pl).to_string(),
        None => full.to_string(),
    }
}

/// Steam lists several studios in one string ("Larian Studios, Feral Interactive"). Split on
/// commas, but keep company suffixes ("Square Enix, Inc.") with their name.
pub(crate) fn split_companies(s: Option<&str>, mut f: impl FnMut(&str)) {
    let Some(s) = s else { return };
    const SUFFIXES: [&str; 12] =
        ["inc", "inc.", "llc", "llc.", "ltd", "ltd.", "co.", "co", "s.a.", "gmbh", "corp.", "corp"];
    let mut start = 0usize;
    for (i, &b) in s.as_bytes().iter().enumerate() {
        if b != b',' {
            continue;
        }
        let next = s[i + 1..].split(',').next().unwrap_or("").trim();
        if SUFFIXES.iter().any(|x| next.eq_ignore_ascii_case(x)) {
            continue; // "Name, Inc." stays together
        }
        let part = s[start..i].trim();
        if !part.is_empty() {
            f(part);
        }
        start = i + 1;
    }
    let last = s[start..].trim();
    if !last.is_empty() {
        f(last);
    }
}

/// String-keyed map with FNV-1a hashing (short keys, not attacker-controlled): several times
/// faster than the default SipHash or a BTreeMap for the counting in [`Counts::compute`].
type FastMap<V> = std::collections::HashMap<String, V, std::hash::BuildHasherDefault<Fnv>>;

#[derive(Default)]
struct Fnv(u64);

impl std::hash::Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        let mut h = if self.0 == 0 { 0xcbf2_9ce4_8422_2325 } else { self.0 };
        for b in bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0100_0000_01b3);
        }
        self.0 = h;
    }
}

/// `v` in lower case into the reused buffer `key` (no allocation once it is big enough).
fn lower_into(key: &mut String, v: &str) {
    key.clear();
    if v.is_ascii() {
        key.push_str(v);
        key.make_ascii_lowercase();
    } else {
        key.extend(v.chars().flat_map(char::to_lowercase));
    }
}

/// (lowercase key, display spelling, count) for one explorer row.
pub(crate) type FacetCount = (String, String, usize);

/// One of the user's collections in the explorer: (id, name, games, smart).
pub(crate) type CollCount = (u64, String, usize, bool);

/// Per-library-generation counts for the explorer and status bar.
#[derive(Default)]
pub(crate) struct Counts {
    pub(super) generation: u64,
    pub all: usize,
    pub installed: usize,
    /// Visible games that aren't installed.
    pub not_installed: usize,
    pub favorites: usize,
    pub recent: usize,
    pub hidden: usize,
    /// Sources entries (cloud services apart from hand-added games), alphabetical.
    pub sources: Vec<(SrcKey, usize)>,
    pub tags: Vec<(String, usize)>,
    /// The user's collections, in their order (empty ones too).
    pub colls: Vec<CollCount>,
    /// Indexed by `Facet::index`, sorted case-insensitively.
    pub facets: [Vec<FacetCount>; N_FACETS],
}

impl Counts {
    /// Whether field `f` gets an explorer section: it has values, and for the user score at
    /// least one rated game (a lone "Not rated" row says nothing).
    pub(crate) fn shows(&self, f: Facet) -> bool {
        let list = &self.facets[f.index()];
        f.in_explorer() && !list.is_empty() && (f != Facet::Score || list.iter().any(|(_, d, _)| d != SCORES[5]))
    }

    pub(super) fn compute(lib: &crate::library::Library) -> Counts {
        let mut c = Counts { generation: lib.generation, ..Default::default() };
        let recent = Collection { kind: CollectionKind::Recent, name: String::new() };
        let mut per_source: Vec<(SrcKey, usize)> = vec![];
        // Hash maps with a cheap hash, sorted once at the end: this loop runs on every library
        // change (each metadata or artwork result), a dozen lookups per game.
        let mut tags: FastMap<(String, usize)> = Default::default();
        // lowercase value -> (display spelling, games, last game index counted)
        let mut facets: [FastMap<(String, usize, usize)>; N_FACETS] = Default::default();
        let mut key = String::new();
        let mut per_coll = vec![0usize; lib.user.len()];
        let (mut pc, mut pc_other) = (0usize, 0usize);
        for (gi, g) in lib.games.iter().enumerate() {
            if g.hidden {
                c.hidden += 1;
                continue;
            }
            pc += is_pc(g) as usize;
            pc_other += is_pc_other(g) as usize;
            for (n, u) in per_coll.iter_mut().zip(&lib.user) {
                *n += u.contains(g) as usize;
            }
            c.all += 1;
            c.installed += g.installed as usize;
            c.not_installed += !g.installed as usize;
            c.favorites += g.favorite as usize;
            // Same predicate as the "Recently played" view itself.
            c.recent += recent.contains(g) as usize;
            let k = SrcKey::of(g);
            match per_source.iter_mut().find(|(s, _)| *s == k) {
                Some(e) => e.1 += 1,
                None => per_source.push((k, 1)),
            }
            for t in &g.tags {
                // Tags compare like `tag_eq` (str::to_lowercase: a final Greek sigma differs
                // from the char-by-char lower case the field values use).
                if t.is_ascii() {
                    lower_into(&mut key, t);
                } else {
                    key.clear();
                    key.push_str(&t.to_lowercase());
                }
                match tags.get_mut(key.as_str()) {
                    Some(e) => e.1 += 1,
                    None => {
                        tags.insert(key.clone(), (t.clone(), 1));
                    }
                }
            }
            for f in Facet::ALL {
                let map = &mut facets[f.index()];
                f.for_each(g, |v| {
                    if v.is_empty() {
                        return;
                    }
                    // Reused key buffer: allocate only for values not seen before.
                    lower_into(&mut key, v);
                    let e = match map.get_mut(key.as_str()) {
                        Some(e) => e,
                        None => map.entry(key.clone()).or_insert((v.to_string(), 0, usize::MAX)),
                    };
                    // A game counts once per value even if the source repeats it.
                    if e.2 != gi {
                        e.2 = gi;
                        e.1 += 1;
                    }
                });
            }
        }
        c.sources = per_source;
        // Alphabetical, like Playnite (predictable however many stores there are).
        c.sources.sort_by_cached_key(|(s, _)| s.label().to_lowercase());
        // Same spelling as the controller's tag collections (the most common one).
        let canon = lib.all_tags();
        let mut tags: Vec<(String, (String, usize))> = tags.into_iter().collect();
        tags.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        c.tags = tags
            .into_iter()
            .map(|(_, (t, n))| {
                let lower = t.to_lowercase();
                let shown = canon.iter().find(|c| c.to_lowercase() == lower).cloned();
                (shown.unwrap_or(t), n)
            })
            .collect();
        for (out, map) in c.facets.iter_mut().zip(facets) {
            *out = map.into_iter().map(|(k, (d, n, _))| (k, d, n)).collect();
            // Keys are unique: the order a BTreeMap would give.
            out.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        }
        // Features in their natural order (controller first, then the Deck, then the user's own
        // alphabetically), not alphabetical; completion statuses in their natural order too.
        c.facets[Facet::Feature.index()]
            .sort_by_key(|(_, d, _)| FEATURES.iter().position(|x| x == d).unwrap_or(usize::MAX));
        c.facets[Facet::Completion.index()]
            .sort_by_key(|(_, d, _)| CompletionStatus::ALL.iter().position(|x| x.label() == d));
        for f in Facet::ALL {
            if let Some(order) = f.fixed_order() {
                c.facets[f.index()].sort_by_key(|(_, d, _)| order.iter().position(|x| x == d));
            }
        }
        c.colls = lib.user.iter().zip(per_coll).map(|(u, n)| (u.id, u.name.clone(), n, u.is_smart())).collect();
        // PCs first, then the consoles by full name (so each maker's consoles sit together).
        let plats = &mut c.facets[Facet::Platform.index()];
        plats.sort_by_cached_key(|(k, d, _)| (!crate::ui::platform_icons::is_pc_label(d), k.clone()));
        // "PC" (every PC game) at the very top, the PC platforms under it. Games whose platform
        // is literally "PC" are PC games: their own entry folds into it.
        plats.retain(|(k, _, _)| k != PC_KEY);
        if pc > 0 {
            plats.insert(0, (PC_KEY.to_string(), "PC".to_string(), pc));
            // Only when it isn't all of PC: an "Other" holding every PC game says nothing.
            if pc_other > 0 && pc_other < pc {
                let after_pcs =
                    plats.iter().skip(1).take_while(|(_, d, _)| crate::ui::platform_icons::is_pc_label(d)).count();
                plats.insert(1 + after_pcs, (PC_OTHER_KEY.to_string(), "Other".to_string(), pc_other));
            }
        }
        // Newest year first.
        c.facets[Facet::Year.index()].sort_by(|a, b| b.0.cmp(&a.0));
        c
    }
}

#[cfg(test)]
mod tests {
    use super::super::{DesktopUi, ExplorerState, LibSel};
    use super::*;
    use crate::library::Library;

    fn game(id: GameId, genres: &[&str], secs: u64, gb: Option<u64>) -> Game {
        let mut g = Game::blank(&format!("G{id}"));
        g.id = id;
        g.source = Source::Steam;
        g.meta.genres = genres.iter().map(|s| s.to_string()).collect();
        g.playtime_secs = secs;
        g.size_bytes = gb.map(|n| n << 30);
        g
    }

    fn lib() -> Library {
        let h = 3600;
        let mut games = vec![
            game(1, &["RPG", "Strategy"], 0, Some(20)),
            game(2, &["RPG"], 30 * 60, Some(2)),
            game(3, &["Strategy"], 5 * h, None),
            game(4, &["Action"], 60 * h, Some(120)),
        ];
        games[2].installed = false;
        games[1].user_rating = Some(4);
        games[3].edits.user_score = Some(95);
        games[0].tags = vec!["Co-op".into(), "Cozy".into()];
        games[1].tags = vec!["Co-op".into()];
        Library::new(games)
    }

    fn ids(ui: &DesktopUi, l: &Library) -> Vec<GameId> {
        let mut v: Vec<GameId> = ui.view.iter().map(|&i| l.games[i].id).collect();
        v.sort();
        v
    }

    /// Field values: one row per value whatever its case (spelled as first seen), in
    /// case-insensitive order; tags likewise, with the library's most common spelling.
    #[test]
    fn counts_order_and_spelling() {
        let mut games = vec![
            game(1, &["rpg", "Zombie"], 0, None),
            game(2, &["RPG", "action"], 0, None),
            game(3, &["Action", "Ärger"], 0, None),
        ];
        games[0].tags = vec!["co-op".into()];
        games[1].tags = vec!["Co-op".into(), "Backlog".into()];
        games[2].tags = vec!["Co-op".into()];
        let c = Counts::compute(&Library::new(games));
        let genres: Vec<(&str, &str, usize)> =
            c.facets[Facet::Genre.index()].iter().map(|(k, d, n)| (k.as_str(), d.as_str(), *n)).collect();
        assert_eq!(
            genres,
            vec![("action", "action", 2), ("rpg", "rpg", 2), ("zombie", "Zombie", 1), ("ärger", "Ärger", 1)]
        );
        assert_eq!(c.tags, vec![("Backlog".to_string(), 1), ("Co-op".to_string(), 3)]);
        // One row for a tag whose capitals end in a Greek sigma (as `tag_eq` sees it).
        let mut g = vec![game(1, &[], 0, None), game(2, &[], 0, None)];
        g[0].tags = vec!["ΟΔΟΣ".into()];
        g[1].tags = vec!["οδος".into()];
        let c = Counts::compute(&Library::new(g));
        assert_eq!(c.tags.len(), 1, "{:?}", c.tags);
        assert_eq!(c.tags[0].1, 2);
    }

    /// PLATFORMS → PC: every PC game (Linux, Windows, macOS, platform not set, literally
    /// "PC"), never emulated, streamed or console games; at the top with the PC platforms under
    /// it, and the same filter for the explorer, smart collections and the art menu.
    #[test]
    fn pc_entry_holds_every_pc_game() {
        let mut games: Vec<Game> = (1..=8).map(|i| game(i, &[], 0, None)).collect();
        games[0].platform = Some("linux".into());
        games[1].platform = Some("windows".into());
        games[2].platform = Some("mac".into());
        games[3].platform = None;
        games[4].platform = Some("PC".into());
        // Emulated, streamed, a console set by hand, hidden: not PC.
        games[5].source = Source::Emulator;
        games[5].emu = Some(Default::default());
        games[5].emu.as_mut().unwrap().platform = "sony_playstation2".into();
        games[6].platform = Some(crate::cloud::PLATFORM.into());
        games[7].platform = Some("Sony PlayStation 2".into());
        let mut hidden = game(9, &[], 0, None);
        hidden.platform = Some("linux".into());
        hidden.hidden = true;
        games.push(hidden);
        let l = Library::new(games);
        let c = Counts::compute(&l);
        let plats: Vec<(&str, usize)> =
            c.facets[Facet::Platform.index()].iter().map(|(k, _, n)| (k.as_str(), *n)).collect();
        assert_eq!(plats[0], ("pc", 5), "{plats:?}");
        assert_eq!(plats.iter().filter(|(k, _)| *k == "pc").count(), 1, "the literal \"PC\" folds in: {plats:?}");
        let pcs: Vec<&str> = plats.iter().skip(1).take(3).map(|p| p.0).collect();
        assert_eq!(pcs, ["linux", "macos", "windows"], "the PC platforms right under it");
        assert_eq!(plats[4], (PC_OTHER_KEY, 2), "then the PC games none of them covers: {plats:?}");
        // The explorer filter.
        let mut ui = DesktopUi::new();
        ui.facet_sel[Facet::Platform.index()] = vec![PC_KEY.into()];
        ui.refresh_view_for(&l);
        assert_eq!(ids(&ui, &l), [1, 2, 3, 4, 5]);
        // PC and Linux together: either (one platform per game).
        ui.facet_sel[Facet::Platform.index()].push("linux".into());
        ui.refresh_view_for(&l);
        assert_eq!(ids(&ui, &l), [1, 2, 3, 4, 5]);
        ui.facet_sel[Facet::Platform.index()] = vec!["linux".into()];
        ui.refresh_view_for(&l);
        assert_eq!(ids(&ui, &l), [1], "Linux alone is still Linux only");
        ui.facet_sel[Facet::Platform.index()] = vec![PC_OTHER_KEY.into()];
        ui.refresh_view_for(&l);
        assert_eq!(ids(&ui, &l), [4, 5], "Other: no platform, or just \"PC\"");
        // A smart collection made from it.
        let mut rule = crate::library::user::SmartRule::default();
        rule.fields.insert(Facet::Platform.key().into(), vec![PC_KEY.into()]);
        let hits: Vec<GameId> = l.games.iter().filter(|g| !g.hidden && rule.matches(g)).map(|g| g.id).collect();
        assert_eq!(hits, [1, 2, 3, 4, 5]);
        rule.match_all = Some(true);
        assert!(rule.matches(&l.games[0]), "Match all: one platform per game, so still any");
        // No PC games: no PC entry.
        let c = Counts::compute(&Library::new(vec![l.games[5].clone()]));
        assert!(!c.facets[Facet::Platform.index()].iter().any(|(k, _, _)| k == PC_KEY));
    }

    /// PLATFORMS → PC → Other appears only when it adds something: never when every PC game
    /// is in it (it would repeat PC), nor when Linux / Windows … cover them all.
    #[test]
    fn pc_other_entry_only_when_pc_is_more_than_its_platforms() {
        let keys = |games: Vec<Game>| -> Vec<String> {
            Counts::compute(&Library::new(games)).facets[Facet::Platform.index()]
                .iter()
                .map(|(k, _, _)| k.clone())
                .collect()
        };
        let mut covered: Vec<Game> = (1..=2).map(|i| game(i, &[], 0, None)).collect();
        covered[0].platform = Some("linux".into());
        covered[1].platform = Some("windows".into());
        assert_eq!(keys(covered.clone()), ["pc", "linux", "windows"]);
        let all_other: Vec<Game> = (1..=2).map(|i| game(i, &[], 0, None)).collect();
        assert_eq!(keys(all_other), ["pc"]);
        covered.push(game(3, &[], 0, None));
        assert_eq!(keys(covered), ["pc", "linux", "windows", PC_OTHER_KEY]);
    }

    /// Running text names a console without its maker; PCs and unknown names stay as they are.
    #[test]
    fn platforms_read_the_same_everywhere() {
        assert_eq!(platform_display("Sony PlayStation 2"), "PlayStation 2");
        assert_eq!(platform_display("PS2"), "PlayStation 2");
        assert_eq!(platform_display("Nintendo 64"), "Nintendo 64");
        assert_eq!(platform_display("windows"), "Windows");
        assert_eq!(platform_display("Amiga 1200 something"), "Amiga 1200 something");
    }

    #[test]
    fn buckets() {
        assert_eq!(playtime_bucket(0), "Never");
        assert_eq!(playtime_bucket(59 * 60), "Under 1 h");
        assert_eq!(playtime_bucket(3600), "1–10 h");
        assert_eq!(playtime_bucket(49 * 3600), "10–50 h");
        assert_eq!(playtime_bucket(50 * 3600), "50 h or more");
        assert_eq!(size_bucket(None), None);
        assert_eq!(size_bucket(Some(0)), None);
        assert_eq!(size_bucket(Some(500 << 20)), Some("Under 1 GB"));
        assert_eq!(size_bucket(Some(100 << 30)), Some("100 GB or more"));
        let mut g = Game::blank("x");
        assert_eq!(score_bucket(&g), "Not rated");
        g.edits.user_score = Some(0);
        assert_eq!(score_bucket(&g), "★");
        g.edits.user_score = Some(81);
        assert_eq!(score_bucket(&g), "★★★★★");
        g.edits.user_score = Some(60);
        assert_eq!(score_bucket(&g), "★★★");
        g.user_rating = Some(2); // the star rating wins
        assert_eq!(score_bucket(&g), "★★");
    }

    #[test]
    fn new_sections_count_in_order() {
        let l = lib();
        let c = Counts::compute(&l);
        assert_eq!(c.not_installed, 1);
        let shown = |f: Facet| -> Vec<(String, usize)> {
            c.facets[f.index()].iter().map(|(_, d, n)| (d.clone(), *n)).collect()
        };
        let pt = shown(Facet::Playtime);
        assert_eq!(
            pt.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(),
            ["Never", "Under 1 h", "1–10 h", "50 h or more"]
        );
        let sz = shown(Facet::Size);
        assert_eq!(sz.iter().map(|x| x.0.as_str()).collect::<Vec<_>>(), ["1–10 GB", "10–50 GB", "100 GB or more"]);
        let sc = shown(Facet::Score);
        assert_eq!(sc, [("★★★★★".into(), 1), ("★★★★".into(), 1), ("Not rated".into(), 2)]);
        assert!(c.shows(Facet::Score) && c.shows(Facet::Size) && !c.shows(Facet::Year));
        // Nobody rated anything: no score section.
        let c = Counts::compute(&Library::new(vec![game(1, &[], 0, None)]));
        assert!(!c.shows(Facet::Score));
    }

    #[test]
    fn not_installed_and_match_all() {
        let l = lib();
        let mut ui = DesktopUi::new();
        ui.refresh_view_for(&l);
        assert_eq!(ids(&ui, &l), [1, 2, 3, 4]);
        ui.lib_sel = LibSel::NotInstalled;
        ui.refresh_view_for(&l);
        assert_eq!(ids(&ui, &l), [3]);
        ui.set_lib_sel(LibSel::All);
        ui.facet_sel[Facet::Genre.index()] = vec!["rpg".into(), "strategy".into()];
        ui.refresh_view_for(&l);
        assert_eq!(ids(&ui, &l), [1, 2, 3], "match any");
        assert!(ui.can_match_all());
        ui.match_all = true;
        ui.refresh_view_for(&l);
        assert_eq!(ids(&ui, &l), [1], "match all");
        // Single-valued fields stay "any" (a game has one time-played bucket).
        ui.facet_sel[Facet::Genre.index()].clear();
        ui.facet_sel[Facet::Playtime.index()] = vec!["never".into(), "50 h or more".into()];
        ui.refresh_view_for(&l);
        assert_eq!(ids(&ui, &l), [1, 4]);
        assert!(!ui.can_match_all());
        ui.facet_sel[Facet::Playtime.index()].clear();
        ui.facet_sel[Facet::Score.index()] = vec!["★★★★".into()];
        ui.refresh_view_for(&l);
        assert_eq!(ids(&ui, &l), [2]);
        ui.facet_sel[Facet::Score.index()].clear();
        // Tags follow the switch too.
        ui.filter.tags = vec!["Co-op".into(), "Cozy".into()];
        ui.refresh_view_for(&l);
        assert_eq!(ids(&ui, &l), [1]);
        ui.match_all = false;
        ui.refresh_view_for(&l);
        assert_eq!(ids(&ui, &l), [1, 2]);
        // The stats page's scope: the same games, hidden ones on request.
        assert_eq!(ui.filtered_indices(&l, true).len(), 2);
    }

    #[test]
    fn new_filters_persist_and_go_into_smart_rules() {
        let mut ui = DesktopUi::new();
        ui.lib_sel = LibSel::NotInstalled;
        ui.match_all = true;
        ui.facet_sel[Facet::Playtime.index()] = vec!["never".into()];
        ui.facet_sel[Facet::Size.index()] = vec!["1–10 gb".into()];
        ui.facet_sel[Facet::Score.index()] = vec!["★★★".into()];
        let json = serde_json::to_string(&ui.explorer_state()).unwrap();
        let mut back = DesktopUi::new();
        back.apply_explorer_state(&serde_json::from_str::<ExplorerState>(&json).unwrap());
        assert_eq!(back.lib_sel, LibSel::NotInstalled);
        assert!(back.match_all);
        assert_eq!(back.facet_sel, ui.facet_sel);
        assert_eq!(back.explorer_hash(), ui.explorer_hash());
        let dir = tempfile::tempdir().unwrap();
        let st = crate::app::AppState::for_test(dir.path());
        let r = ui.current_rule(&st);
        assert!(r.not_installed && !r.installed);
        assert_eq!(r.fields.get("playtime"), Some(&vec!["never".to_string()]));
        assert_eq!(r.fields.get("score"), Some(&vec!["★★★".to_string()]));
        assert_eq!(r.match_all, None, "nothing where the switch matters");
    }
}
