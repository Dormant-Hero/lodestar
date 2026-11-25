//! In-memory library: the games, collections (for controller mode / sidebar) and filtered views.
//!
//! CONTRACT: UI code reads games only through `Library`. Views are `Vec<usize>` indices into
//! `Library::games`, recomputed only when inputs change (see `ViewCache`).

use crate::config::SortBy;
use crate::model::*;
use crate::search;
use std::collections::HashMap;

mod categories;
pub use categories::Category;
pub mod user;
pub use user::UserCollection;

#[derive(Default)]
pub struct Library {
    pub games: Vec<Game>,
    by_id: HashMap<GameId, usize>,
    /// The user's own collections (manual and smart), in their order (DB kv, see user.rs).
    pub user: Vec<UserCollection>,
    /// Bumped on every change; views compare it to know when to recompute.
    pub generation: u64,
    /// Per game (same index as `games`): the console its free-text `platform` names, if any.
    /// Resolving the name is a normalise-and-look-up, and the collection lists ask it for
    /// every game of every console collection after each change: at 20,000 games that alone
    /// took a whole frame. Kept in step by `replace` and `update`, the only ways games change.
    named_platform: Vec<Option<&'static str>>,
    /// Achievement counts the library shows (unlocked, total), for Sort → Achievements
    /// (`achievements::Hub::counts`; empty when switched off).
    pub achievements: std::sync::Arc<HashMap<GameId, (u32, u32)>>,
    /// Bumped when `achievements` changes (not `generation`: only lists sorted by achievements
    /// are sorted again, see `view_generation`).
    pub achievements_generation: u64,
    /// The account's owned Steam games (None: no list, nothing is marked; see steam_owned.rs).
    steam_owned: Option<crate::steam_owned::OwnedSet>,
}

fn named_platform(g: &Game) -> Option<&'static str> {
    g.platform.as_deref().and_then(platform_named).map(|p| p.id)
}

/// The platform a game's platform text names. A bare "PC" names none of the PC platforms:
/// stores say it of Linux and Windows games alike.
fn platform_named(name: &str) -> Option<&'static crate::emulation::platforms::Platform> {
    if name.trim().eq_ignore_ascii_case("pc") { None } else { crate::emulation::platforms::by_name(name) }
}

impl Library {
    #[cfg(test)]
    pub fn new(games: Vec<Game>) -> Library {
        let mut l = Library::default();
        l.replace(games);
        l
    }

    pub fn replace(&mut self, games: Vec<Game>) {
        self.by_id = games.iter().enumerate().map(|(i, g)| (g.id, i)).collect();
        self.named_platform = games.iter().map(named_platform).collect();
        self.games = games;
        self.generation += 1;
    }

    pub fn get(&self, id: GameId) -> Option<&Game> {
        self.by_id.get(&id).map(|&i| &self.games[i])
    }

    pub fn index_of(&self, id: GameId) -> Option<usize> {
        self.by_id.get(&id).copied()
    }

    /// New achievement counts (Sort → Achievements).
    pub fn set_achievement_counts(&mut self, counts: std::sync::Arc<HashMap<GameId, (u32, u32)>>) {
        self.achievements = counts;
        self.achievements_generation += 1;
    }

    /// A new owned-games list. Bumps the generation when it changes what is marked.
    pub fn set_steam_owned(&mut self, owned: Option<crate::steam_owned::OwnedSet>) {
        if self.steam_owned != owned {
            self.steam_owned = owned;
            self.generation += 1;
        }
    }

    /// `g` played here but is not in the account's Steam library any more (steam_owned.rs).
    pub fn not_in_steam_library(&self, g: &Game) -> bool {
        crate::steam_owned::not_in_library(g, self.steam_owned.as_deref())
    }

    /// What a list sorted by `sort` depends on: the generation, and for Sort → Achievements also
    /// the counts (new counts don't re-sort a list sorted by name: 20,000 games take a frame).
    pub fn view_generation(&self, sort: SortBy) -> u64 {
        if sort == SortBy::Achievements {
            self.generation ^ self.achievements_generation.rotate_left(32)
        } else {
            self.generation
        }
    }

    /// Replace the user's collections (the caller saves them). Bumps the generation.
    pub fn set_user_collections(&mut self, colls: Vec<UserCollection>) {
        self.user = colls;
        self.generation += 1;
    }

    /// Games no longer in their library and not kept by the user (Settings → Libraries →
    /// Missing games), in library order.
    pub fn missing(&self) -> impl Iterator<Item = &Game> {
        self.games.iter().filter(|g| g.missing.is_some_and(|m| !m.kept))
    }

    pub fn user_collection(&self, id: u64) -> Option<&UserCollection> {
        self.user.iter().find(|c| c.id == id)
    }

    /// Mutate one game in memory (the caller also writes the DB). Bumps the generation.
    pub fn update(&mut self, id: GameId, f: impl FnOnce(&mut Game)) {
        if let Some(&i) = self.by_id.get(&id) {
            let g = &mut self.games[i];
            let before = UserData::of(g);
            f(g);
            // Like the DB trigger `games_modified` (Sort → Recently modified).
            if before != UserData::of(g) {
                g.modified = Some(now_unix());
            }
            self.named_platform[i] = named_platform(g);
            self.generation += 1;
        }
    }

    /// Every tag once, in case-insensitive order. Tags that differ only in case ("RPG", "rpg")
    /// are one tag everywhere (filters, collections, the explorer); the spelling shown is the
    /// one used by most games (ties: the first in sort order).
    pub fn all_tags(&self) -> Vec<String> {
        // Spelling -> games, grouped by the lower-case tag; a few distinct tags over thousands
        // of games, so count first and sort only the distinct spellings.
        let mut groups: HashMap<String, std::collections::BTreeMap<&str, usize>> = HashMap::new();
        let mut key = String::new();
        for t in self.games.iter().flat_map(|g| g.tags.iter()) {
            key.clear();
            if t.is_ascii() {
                key.push_str(t);
                key.make_ascii_lowercase();
            } else {
                key.push_str(&t.to_lowercase());
            }
            let group = match groups.get_mut(key.as_str()) {
                Some(g) => g,
                None => groups.entry(key.clone()).or_default(),
            };
            *group.entry(t.as_str()).or_default() += 1;
        }
        let mut groups: Vec<_> = groups.into_iter().collect();
        groups.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        groups
            .into_iter()
            .map(|(_, spellings)| {
                // Most common spelling; ties: the first in sort order (BTreeMap order).
                let mut best: (&str, usize) = ("", 0);
                for (sp, n) in spellings {
                    if n > best.1 {
                        best = (sp, n);
                    }
                }
                best.0.to_string()
            })
            .collect()
    }

    /// Sources that have at least one visible game.
    pub fn sources_present(&self) -> Vec<Source> {
        let mut v: Vec<Source> =
            Source::ALL.into_iter().filter(|s| self.games.iter().any(|g| g.source == *s && !g.hidden)).collect();
        v.sort();
        v
    }

    /// Consoles (Playnite platform ids) of the visible emulated games, sorted by display name.
    pub fn platforms_present(&self) -> Vec<String> {
        let mut v: Vec<String> = vec![];
        for g in self.games.iter().filter(|g| !g.hidden) {
            if let Some(e) = g.emu.as_ref().filter(|e| !e.platform.is_empty())
                && !v.contains(&e.platform)
            {
                v.push(e.platform.clone());
            }
        }
        v.sort_by_cached_key(|id| platform_label(id).to_lowercase());
        v
    }

    /// `c.contains(&self.games[i])`, using the resolved platform names for console collections.
    pub fn in_collection(&self, c: &Collection, i: usize) -> bool {
        let g = &self.games[i];
        match &c.kind {
            CollectionKind::Platform(id) => {
                g.emu.as_ref().is_some_and(|e| e.platform == *id)
                    || match self.named_platform.get(i) {
                        Some(p) => *p == Some(id.as_str()),
                        None => named_platform(g) == Some(id.as_str()),
                    }
            }
            _ => c.contains(g),
        }
    }

    /// Collections for controller mode, in display order, as listed when the player chose
    /// nothing (see [`Library::categories`]).
    pub fn collections(&self) -> Vec<Collection> {
        self.categories().into_iter().filter(|c| c.default).map(|c| c.collection).collect()
    }

    /// Indices of games in a collection (respecting hidden), sorted for that collection.
    pub fn collection_games(&self, c: &Collection) -> Vec<usize> {
        let mut v: Vec<usize> =
            (0..self.games.len()).filter(|&i| !self.games[i].hidden && self.in_collection(c, i)).collect();
        match c.kind {
            CollectionKind::Recent => {
                v.sort_by_key(|&i| std::cmp::Reverse(self.games[i].last_played.unwrap_or(0)));
                v.truncate(30);
            }
            _ => self.sort(&mut v, SortBy::Name, false),
        }
        v
    }

    pub fn sort(&self, v: &mut [usize], by: SortBy, descending: bool) {
        let g = &self.games;
        let name = |i: &usize| g[*i].sort_name.as_str();
        // Games without the value (no size, no score …): at the end in both directions.
        let mut missing: Option<fn(&Game) -> bool> = None;
        if by == SortBy::Achievements {
            self.sort_by_achievements(v, descending);
            return;
        }
        match by {
            SortBy::Name => v.sort_by(|a, b| name(a).cmp(name(b))),
            SortBy::LastPlayed => v.sort_by(|a, b| {
                g[*b].last_played.unwrap_or(0).cmp(&g[*a].last_played.unwrap_or(0)).then(name(a).cmp(name(b)))
            }),
            SortBy::Playtime => {
                v.sort_by(|a, b| g[*b].playtime_secs.cmp(&g[*a].playtime_secs).then(name(a).cmp(name(b))))
            }
            SortBy::PlayCount => v.sort_by(|a, b| g[*b].play_count.cmp(&g[*a].play_count).then(name(a).cmp(name(b)))),
            SortBy::Added => v.sort_by(|a, b| g[*b].added.cmp(&g[*a].added).then(name(a).cmp(name(b)))),
            SortBy::Modified => {
                let m = |i: &usize| g[*i].modified.unwrap_or(g[*i].added);
                v.sort_by(|a, b| m(b).cmp(&m(a)).then(name(a).cmp(name(b))))
            }
            SortBy::ReleaseYear => {
                missing = Some(|g| release_key(g).is_none());
                v.sort_by(|a, b| release_key(&g[*b]).cmp(&release_key(&g[*a])).then(name(a).cmp(name(b))))
            }
            SortBy::Source => v.sort_by(|a, b| g[*a].source.cmp(&g[*b].source).then(name(a).cmp(name(b)))),
            SortBy::InstallSize => {
                missing = Some(|g| g.size_bytes.is_none());
                v.sort_by(|a, b| g[*b].size_bytes.cmp(&g[*a].size_bytes).then(name(a).cmp(name(b))))
            }
            SortBy::UserScore => {
                missing = Some(|g| g.edits.user_score.is_none());
                v.sort_by(|a, b| g[*b].edits.user_score.cmp(&g[*a].edits.user_score).then(name(a).cmp(name(b))))
            }
            SortBy::Completion => {
                missing = Some(|g| g.completion().is_none());
                let rank = |i: &usize| completion_rank(g[*i].completion());
                v.sort_by(|a, b| rank(a).cmp(&rank(b)).then(name(a).cmp(name(b))))
            }
            SortBy::InstallFolder => {
                missing = Some(|g| g.install_dir.is_none());
                v.sort_by_cached_key(|&i| {
                    let p = g[i].install_dir.as_deref().map(|p| p.to_string_lossy().to_lowercase());
                    (p.is_none(), p, g[i].sort_name.clone())
                })
            }
            SortBy::Achievements => {}
            SortBy::RomPath => {
                missing = Some(|g| g.emu.is_none());
                v.sort_by_cached_key(|&i| {
                    let p = g[i].emu.as_ref().map(|e| e.rom.to_string_lossy().to_lowercase());
                    (p.is_none(), p, g[i].sort_name.clone())
                })
            }
        }
        if descending {
            v.reverse();
            if let Some(m) = missing {
                // Stable: keeps the reversed order within both parts.
                v.sort_by_key(|&i| m(&g[i]));
            }
        }
    }

    /// Most complete first (share unlocked, then more unlocked, then the name); games without a
    /// count stay at the end in both directions.
    fn sort_by_achievements(&self, v: &mut [usize], descending: bool) {
        let g = &self.games;
        let count = |i: usize| self.achievements.get(&g[i].id).copied().filter(|c| c.1 > 0);
        v.sort_by(|&a, &b| match (count(a), count(b)) {
            (Some((ua, ta)), Some((ub, tb))) => (ub as u64 * ta as u64)
                .cmp(&(ua as u64 * tb as u64))
                .then(ub.cmp(&ua))
                .then(g[a].sort_name.cmp(&g[b].sort_name)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => g[a].sort_name.cmp(&g[b].sort_name),
        });
        if descending {
            v.reverse();
            v.sort_by_key(|&i| count(i).is_none());
        }
    }

    /// Apply a filter + sort. With a search query, results are ranked by match quality instead.
    pub fn filter(&self, f: &Filter) -> Vec<usize> {
        let q = f.search.trim();
        let mut scored: Vec<(i32, usize)> = vec![];
        let mut v: Vec<usize> = vec![];
        for (i, g) in self.games.iter().enumerate() {
            if g.hidden && !f.show_hidden {
                continue;
            }
            if f.installed_only && !g.installed {
                continue;
            }
            if f.favorites_only && !g.favorite {
                continue;
            }
            if !f.sources.is_empty() && !f.sources.contains(&g.source) {
                continue;
            }
            if !f.tags.is_empty() && !f.tags.iter().all(|t| has_tag(g, t)) {
                continue;
            }
            if let Some(c) = &f.collection
                && !self.in_collection(c, i)
            {
                continue;
            }
            if q.is_empty() {
                v.push(i);
            } else if let Some(s) = search::score(q, g.display_name()) {
                scored.push((s, i));
            }
        }
        if q.is_empty() {
            self.sort(&mut v, f.sort, f.descending);
            v
        } else {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(self.games[a.1].sort_name.cmp(&self.games[b.1].sort_name)));
            scored.into_iter().map(|(_, i)| i).collect()
        }
    }
}

/// The parts of a game whose change counts as "modified" (same as the DB trigger
/// `games_modified`; play statistics and scans don't count).
#[derive(PartialEq)]
struct UserData {
    user_name: Option<String>,
    flags: (bool, bool, Option<u8>, Option<ControllerSupport>),
    tags: Vec<String>,
    art: Art,
    meta: Metadata,
    launch_override: Option<LaunchOverride>,
    edits: UserEdits,
}

impl UserData {
    fn of(g: &Game) -> UserData {
        UserData {
            user_name: g.user_name.clone(),
            flags: (g.favorite, g.hidden, g.user_rating, g.controller_override),
            tags: g.tags.clone(),
            art: g.art.clone(),
            meta: g.meta.clone(),
            launch_override: g.launch_override.clone(),
            edits: g.edits.clone(),
        }
    }
}

/// A console's display name ("Sony PlayStation 2") by Playnite platform id; the id itself when
/// it isn't known.
pub fn platform_label(id: &str) -> String {
    crate::emulation::platforms::by_id(id).map(|p| p.name.to_string()).unwrap_or_else(|| id.to_string())
}

/// Release date for sorting: (year, month, day) from the full date when known, else
/// (year, 0, 0); None without either.
pub fn release_key(g: &Game) -> Option<(i32, u32, u32)> {
    if let Some(d) = g.meta.release_date.as_deref() {
        let mut it = d.trim().splitn(3, '-').map(|p| p.trim().parse::<i64>().ok());
        if let (Some(Some(y)), m, day) = (it.next(), it.next().flatten(), it.next().flatten()) {
            let clamp = |v: Option<i64>, hi: i64| v.map_or(0, |v| v.clamp(0, hi) as u32);
            return Some((y as i32, clamp(m, 12), clamp(day, 31)));
        }
    }
    g.meta.release_year.map(|y| (y, 0, 0))
}

/// Order of completion statuses (as listed in menus); not set comes last.
pub fn completion_rank(c: Option<CompletionStatus>) -> usize {
    c.and_then(|c| CompletionStatus::ALL.iter().position(|x| *x == c)).unwrap_or(CompletionStatus::ALL.len())
}

/// Tags are compared case-insensitively ("RPG" == "rpg"), like the desktop explorer groups them.
pub fn tag_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || (!(a.is_ascii() && b.is_ascii()) && a.to_lowercase() == b.to_lowercase())
}

/// Whether `g` carries tag `t` (case-insensitive).
pub fn has_tag(g: &Game, t: &str) -> bool {
    g.tags.iter().any(|x| tag_eq(x, t))
}

/// Remove empty tags and duplicates that differ only in case, keeping the first spelling and order.
pub fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(tags.len());
    for t in tags {
        let t = t.trim().to_string();
        if !t.is_empty() && !out.iter().any(|o| tag_eq(o, &t)) {
            out.push(t);
        }
    }
    out
}

/// Stores with fewer games than this get no controller-mode collection of their own.
pub const MIN_STORE_GAMES: usize = 3;

/// A PC game: not emulated, not from a ROM server, not streamed.
pub fn is_pc_game(g: &Game) -> bool {
    g.emu.is_none()
        && !matches!(g.source, Source::Emulator | Source::Romm)
        && g.platform.as_deref() != Some(crate::cloud::PLATFORM)
}

/// A game of the PC category (desktop PLATFORMS → PC, controller "PC Games"): a PC game whose
/// platform, when set, is a PC one (Linux, Windows, macOS, PC, a browser), not a console name.
pub fn in_pc_category(g: &Game) -> bool {
    is_pc_game(g)
        && g.platform
            .as_deref()
            .is_none_or(|p| crate::ui::platform_icons::is_pc_label(crate::emulation::platforms::display_name(p)))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum CollectionKind {
    All,
    Installed,
    /// Games with full or partial controller support (Steam data or the user's setting).
    Controller,
    /// Every PC game ([`in_pc_category`]), whatever store it came from: one home for a
    /// theme's "pc" art instead of a dozen store collections all borrowing it.
    Pc,
    Favorites,
    Recent,
    Source(Source),
    Tag(String),
    /// One console's games (Playnite platform id, e.g. "sony_playstation2"): emulated games of
    /// that platform and games whose platform is set to its name.
    Platform(String),
    /// One of the user's own collections (a snapshot: compare by value).
    User(UserCollection),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Collection {
    pub kind: CollectionKind,
    pub name: String,
}

impl Collection {
    pub fn contains(&self, g: &Game) -> bool {
        match &self.kind {
            CollectionKind::All => true,
            CollectionKind::Installed => g.installed,
            CollectionKind::Controller => g.controller().is_some_and(|c| c.is_playable()),
            CollectionKind::Pc => in_pc_category(g),
            CollectionKind::Favorites => g.favorite,
            // `last_played` is already None for games that don't count as played (see
            // `db::trusted_last_played`).
            CollectionKind::Recent => g.last_played.is_some(),
            CollectionKind::Source(s) => g.source == *s,
            CollectionKind::Tag(t) => has_tag(g, t),
            CollectionKind::Platform(id) => {
                g.emu.as_ref().is_some_and(|e| e.platform == *id) || named_platform(g) == Some(id.as_str())
            }
            CollectionKind::User(u) => u.contains(g),
        }
    }

    /// Stable short key (theme art lookup, remembering selection).
    pub fn key(&self) -> String {
        match &self.kind {
            CollectionKind::All => "all".into(),
            CollectionKind::Installed => "installed".into(),
            CollectionKind::Controller => "controller".into(),
            CollectionKind::Pc => "pc".into(),
            CollectionKind::Favorites => "favorites".into(),
            CollectionKind::Recent => "recent".into(),
            CollectionKind::Source(s) => s.key().into(),
            CollectionKind::Tag(t) => format!("tag:{t}"),
            CollectionKind::Platform(id) => format!("platform:{id}"),
            CollectionKind::User(u) => format!("coll:{}", u.id),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Filter {
    pub search: String,
    pub sources: Vec<Source>,
    pub tags: Vec<String>,
    pub installed_only: bool,
    pub favorites_only: bool,
    pub show_hidden: bool,
    pub collection: Option<Collection>,
    pub sort: SortBy,
    pub descending: bool,
}

/// Memoises `Library::filter` so the UI can call it every frame for free.
#[derive(Default)]
pub struct ViewCache {
    key: Option<(u64, Filter)>,
    pub indices: Vec<usize>,
}

impl ViewCache {
    pub fn get<'a>(&'a mut self, lib: &Library, f: &Filter) -> &'a [usize] {
        let gen_ = lib.view_generation(f.sort);
        let fresh = matches!(&self.key, Some((g, kf)) if *g == gen_ && kf == f);
        if !fresh {
            self.indices = lib.filter(f);
            self.key = Some((gen_, f.clone()));
        }
        &self.indices
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    /// Names from a real library (Steam + Faugus + desktop).
    const REAL: &[&str] = &[
        "Baldur's Gate 3",
        "Baldur's Gate II: Enhanced Edition",
        "Baldur's Gate: Enhanced Edition",
        "Beyond: Two Souls",
        "Black Myth: Wukong",
        "Bully: Scholarship Edition",
        "CHRONO TRIGGER",
        "Clair Obscur: Expedition 33",
        "CODE VEIN",
        "Control Ultimate Edition",
        "Crash Bandicoot™ N. Sane Trilogy",
        "Cyberpunk 2077",
        "Dead Island Definitive Edition",
        "Dead Island Riptide Definitive Edition",
        "Detroit: Become Human",
        "Devil May Cry 4 Special Edition",
        "Divinity: Original Sin 2",
        "Divinity: Original Sin Enhanced Edition",
        "DOOM",
        "DOOM Eternal",
        "Dragon's Dogma 2",
        "Dragon's Dogma: Dark Arisen",
        "Dying Light",
        "Echoes of the End: Enhanced Edition",
        "ELDEN RING",
        "The Elder Scrolls IV: Oblivion Remastered",
        "The Elder Scrolls V: Skyrim Special Edition",
        "EvoCreo",
        "Fallout: New Vegas",
        "FINAL FANTASY VII",
        "FINAL FANTASY VII (2013)",
        "Football Life 2026",
        "Football Life 2026 Settings",
        "Football Life 2027",
        "Football Life 2027 Settings",
        "Halo: The Master Chief Collection",
        "Heavy Rain",
        "Hellblade: Senua's Sacrifice",
        "HITMAN World of Assassination",
        "Hogwarts Legacy",
        "Icewind Dale: Enhanced Edition",
        "Kingdom Rush 6: Genesis TD",
        "Metaphor: ReFantazio",
        "mgo2.exe",
        "Middle-earth™: Shadow of Mordor™",
        "Mirror's Edge™ Catalyst",
        "Neverwinter Nights: Enhanced Edition",
        "Nioh 3",
        "Old School RuneScape",
        "ONE PIECE ODYSSEY",
        "Pathfinder: Wrath of the Righteous - Enhanced Edition",
        "Planescape: Torment: Enhanced Edition",
        "Rise of the Tomb Raider",
        "RuneLite",
        "RuneScape",
        "Sacred 2 Remaster",
        "SCARLET NEXUS",
        "Sekiro™: Shadows Die Twice",
        "Shadow of the Tomb Raider",
        "Solasta: Crown of the Magister",
        "STAR WARS Jedi: Fallen Order™",
        "STAR WARS Jedi: Survivor™",
        "Tales of ARISE",
        "Tomb Raider",
        "Warhammer 40,000: Rogue Trader",
        "Where Winds Meet",
        "The Witcher 3: Wild Hunt — Remastered",
    ];

    fn lib(names: &[&str]) -> (Db, Library) {
        let mut db = Db::open_memory().unwrap();
        let scanned: Vec<ScannedGame> =
            names.iter().enumerate().map(|(i, n)| ScannedGame::new(Source::Steam, i.to_string(), *n)).collect();
        db.merge_scan(&scanned, &[Source::Steam]).unwrap();
        let l = Library::new(db.load_all().unwrap());
        (db, l)
    }

    fn search<'a>(l: &'a Library, q: &str) -> Vec<&'a str> {
        let f = Filter { search: q.into(), ..Default::default() };
        l.filter(&f).into_iter().map(|i| l.games[i].display_name()).collect()
    }

    #[test]
    fn search_ranking_on_real_names() {
        let (_db, l) = lib(REAL);
        let top = |q: &str| search(&l, q).first().copied().unwrap_or("<none>").to_string();
        assert_eq!(top("doom"), "DOOM");
        assert_eq!(top("tomb"), "Tomb Raider");
        assert_eq!(top("witcher"), "The Witcher 3: Wild Hunt — Remastered");
        assert_eq!(top("skyrim"), "The Elder Scrolls V: Skyrim Special Edition");
        assert_eq!(top("bg3"), "Baldur's Gate 3");
        assert_eq!(top("dos2"), "Divinity: Original Sin 2");
        assert_eq!(top("ff7"), "<none>"); // VII, not 7: no loose match into unrelated titles
        assert_eq!(top("elden"), "ELDEN RING");
        assert_eq!(top("runescape"), "RuneScape");
        assert_eq!(top("osrs"), "Old School RuneScape");
        assert_eq!(top("jedi survivor"), "STAR WARS Jedi: Survivor™");
        assert_eq!(top("baldurs"), "Baldur's Gate 3");
        assert_eq!(top("football 2027"), "Football Life 2027");
        assert_eq!(top("  Hogwarts  "), "Hogwarts Legacy");
        let tr = search(&l, "tomb raider");
        assert_eq!(tr, vec!["Tomb Raider", "Rise of the Tomb Raider", "Shadow of the Tomb Raider"]);
        // Prefix matches outrank word matches.
        let dead = search(&l, "dead");
        assert_eq!(dead.len(), 2, "{dead:?}");
        assert!(search(&l, "zzzz").is_empty());
        assert!(search(&l, "e").len() > 20);
    }

    #[test]
    fn platform_collections_follow_sources() {
        let mut games = vec![];
        for (i, (name, platform)) in
            [("Gran", "sony_playstation2"), ("Zelda", "nintendo_gamecube"), ("Kart", "nintendo_gamecube")]
                .iter()
                .enumerate()
        {
            let mut g = Game::blank(name);
            g.id = i as GameId + 1;
            g.source = Source::Emulator;
            g.emu = Some(EmuRef { platform: platform.to_string(), ..Default::default() });
            games.push(g);
        }
        // A game whose platform was set by hand to a console name counts too.
        let mut hand = Game::blank("Hand");
        hand.id = 10;
        hand.platform = Some("Sony PlayStation 2".into());
        games.push(hand);
        let l = Library::new(games);
        assert_eq!(l.platforms_present(), vec!["nintendo_gamecube".to_string(), "sony_playstation2".to_string()]);
        let cs = l.collections();
        let names: Vec<&str> = cs.iter().map(|c| c.name.as_str()).collect();
        let at = |n: &str| names.iter().position(|x| *x == n).unwrap();
        assert!(
            at("Emulators") < at("Nintendo GameCube") && at("Nintendo GameCube") < at("Sony PlayStation 2"),
            "{names:?}"
        );
        let ps2 = &cs[at("Sony PlayStation 2")];
        assert_eq!(ps2.key(), "platform:sony_playstation2");
        let in_ps2: Vec<&str> = l.collection_games(ps2).into_iter().map(|i| l.games[i].name.as_str()).collect();
        assert_eq!(in_ps2, vec!["Gran", "Hand"]);
    }

    #[test]
    fn user_collections_are_controller_collections() {
        let (_db, mut l) = lib(&["A", "B", "C"]);
        let ids: Vec<GameId> = l.games.iter().map(|g| g.id).collect();
        let g0 = l.generation;
        l.set_user_collections(vec![
            UserCollection { id: 4, name: "Picks".into(), games: vec![ids[2], ids[0]], rule: None },
            UserCollection { id: 5, name: "Empty".into(), ..Default::default() },
            UserCollection {
                id: 6,
                name: "Bs".into(),
                rule: Some(user::SmartRule { search: "b".into(), ..Default::default() }),
                ..Default::default()
            },
        ]);
        assert!(l.generation > g0);
        let cs = l.collections();
        let picks = cs.iter().find(|c| c.name == "Picks").unwrap();
        assert_eq!(picks.key(), "coll:4");
        let names: Vec<&str> = l.collection_games(picks).into_iter().map(|i| l.games[i].name.as_str()).collect();
        assert_eq!(names, ["A", "C"]);
        assert!(!cs.iter().any(|c| c.name == "Empty"), "empty collections are left out");
        let bs = cs.iter().find(|c| c.name == "Bs").unwrap();
        assert_eq!(l.collection_games(bs).len(), 1);
        // A filter on a collection value is just a filter.
        let f = Filter { collection: Some(picks.clone()), ..Default::default() };
        assert_eq!(l.filter(&f).len(), 2);
    }

    #[test]
    fn tags_are_deduped_and_collections_unique() {
        let (db, mut l) = lib(&["A", "B", "C"]);
        let ids: Vec<GameId> = l.games.iter().map(|g| g.id).collect();
        for (id, tags) in ids.iter().zip([vec!["RPG"], vec!["rpg"], vec!["RPG", "Co-op"]]) {
            let tags: Vec<String> = tags.into_iter().map(String::from).collect();
            db.set_tags(*id, &tags).unwrap();
            l.update(*id, |g| g.tags = tags.clone());
        }
        // "RPG" and "rpg" are one tag (the spelling most games use), like the explorer shows it.
        let tags = l.all_tags();
        assert_eq!(tags, vec!["Co-op", "RPG"]);
        // Filtering by either spelling finds all three RPGs, and so does the tag collection.
        for t in ["RPG", "rpg", "Rpg"] {
            let f = Filter { tags: vec![t.into()], ..Default::default() };
            assert_eq!(l.filter(&f).len(), 3, "{t}");
        }
        let f = Filter { tags: vec!["rpg".into(), "CO-OP".into()], ..Default::default() };
        assert_eq!(l.filter(&f).len(), 1);
        let rpg = l.collections().into_iter().find(|c| c.name == "RPG").unwrap();
        assert_eq!(l.collection_games(&rpg).len(), 3);
        assert_eq!(normalize_tags(vec![" RPG ".into(), "rpg".into(), "".into(), "Co-op".into()]), vec!["RPG", "Co-op"]);
        assert!(tag_eq("Ärger", "äRGER"));
        let keys: Vec<String> = l.collections().iter().map(|c| c.key()).collect();
        let mut uniq = keys.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(keys.len(), uniq.len(), "{keys:?}");
    }

    #[test]
    fn sorting_by_achievements_puts_the_most_complete_first_and_games_without_last() {
        let mut games = vec![];
        for (i, name) in ["Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta"].into_iter().enumerate() {
            let mut g = Game::blank(name);
            g.id = i as GameId + 1;
            g.sort_name = name.to_lowercase();
            games.push(g);
        }
        let mut l = Library::new(games);
        // Beta 50 %, Gamma 100 %, Delta 50 % with more unlocked, Epsilon 0 %; Alpha and Zeta: none.
        let counts: HashMap<GameId, (u32, u32)> = [(2, (5, 10)), (3, (4, 4)), (4, (20, 40)), (5, (0, 30))].into();
        let g0 = l.generation;
        let v0 = l.view_generation(SortBy::Achievements);
        l.set_achievement_counts(std::sync::Arc::new(counts));
        assert_ne!(l.view_generation(SortBy::Achievements), v0, "views sorted by achievements recompute");
        assert_eq!((l.generation, l.view_generation(SortBy::Name)), (g0, g0), "others don't");
        let names = |v: &[usize]| v.iter().map(|&i| l.games[i].name.clone()).collect::<Vec<_>>();
        let mut v: Vec<usize> = (0..l.games.len()).collect();
        l.sort(&mut v, SortBy::Achievements, false);
        assert_eq!(names(&v), ["Gamma", "Delta", "Beta", "Epsilon", "Alpha", "Zeta"]);
        l.sort(&mut v, SortBy::Achievements, true);
        assert_eq!(names(&v), ["Epsilon", "Beta", "Delta", "Gamma", "Zeta", "Alpha"], "without a count: still last");
    }

    /// `all_tags`: one entry per tag whatever its case, in case-insensitive order; the spelling
    /// most games use, ties going to the first spelling in sort order.
    #[test]
    fn all_tags_order_and_spelling_ties() {
        let mut games = vec![];
        for (i, tags) in
            [vec!["zed", "Beta"], vec!["Zed"], vec!["beta"], vec!["ärger", "Alpha"], vec!["Ärger"], vec!["Beta"]]
                .into_iter()
                .enumerate()
        {
            let mut g = Game::blank(&format!("G{i}"));
            g.id = i as GameId + 1;
            g.tags = tags.into_iter().map(String::from).collect();
            games.push(g);
        }
        let l = Library::new(games);
        // "Beta" ×2 beats "beta"; "Zed"/"zed" and "Ärger"/"ärger" tie: the first in byte order.
        assert_eq!(l.all_tags(), vec!["Alpha", "Beta", "Zed", "Ärger"]);
    }

    #[test]
    fn new_sorts_keep_missing_values_last() {
        let (_db, mut l) = lib(&["A", "B", "C", "D"]);
        let ids: Vec<GameId> = l.games.iter().map(|g| g.id).collect();
        l.update(ids[0], |g| g.size_bytes = Some(10));
        l.update(ids[1], |g| g.size_bytes = Some(30));
        l.update(ids[2], |g| g.edits.user_score = Some(90));
        l.update(ids[3], |g| g.edits.user_score = Some(40));
        l.update(ids[0], |g| g.meta.release_date = Some("2020-05-02".into()));
        l.update(ids[1], |g| g.meta.release_year = Some(2020));
        l.update(ids[2], |g| g.meta.release_date = Some("2021-01-01".into()));
        l.update(ids[3], |g| g.edits.completion = Some(CompletionStatus::Beaten));
        l.update(ids[2], |g| g.edits.completion = Some(CompletionStatus::Playing));
        let order = |l: &Library, by: SortBy, desc: bool| {
            let mut v: Vec<usize> = (0..l.games.len()).collect();
            l.sort(&mut v, by, desc);
            v.into_iter().map(|i| l.games[i].name.clone()).collect::<Vec<_>>()
        };
        assert_eq!(order(&l, SortBy::InstallSize, false), ["B", "A", "C", "D"]);
        assert_eq!(order(&l, SortBy::InstallSize, true), ["A", "B", "D", "C"], "no size: last both ways");
        assert_eq!(order(&l, SortBy::UserScore, false), ["C", "D", "A", "B"]);
        assert_eq!(order(&l, SortBy::ReleaseYear, false), ["C", "A", "B", "D"], "full date beats the year");
        assert_eq!(order(&l, SortBy::Completion, false), ["C", "D", "A", "B"], "status order, not set last");
        assert_eq!(order(&l, SortBy::Completion, true), ["D", "C", "B", "A"]);
        // Every change above counts as modified; the last one touched C.
        assert!(l.games.iter().all(|g| g.modified.is_some()));
        l.update(ids[1], |g| g.playtime_secs = 99);
        let before = l.get(ids[1]).unwrap().modified;
        l.update(ids[1], |g| g.playtime_secs = 100);
        assert_eq!(l.get(ids[1]).unwrap().modified, before, "play time is not a modification");
    }

    #[test]
    fn the_db_remembers_when_a_game_changed() {
        let (db, _) = lib(&["A", "B"]);
        let all = db.load_all().unwrap();
        assert!(all.iter().all(|g| g.modified.is_none()));
        db.set_favorite(all[0].id, true).unwrap();
        db.set_play_stats(all[1].id, 60, 1, None).unwrap();
        let all = db.load_all().unwrap();
        assert!(all[0].modified.is_some_and(|t| t > 1_700_000_000));
        assert!(all[1].modified.is_none(), "play statistics don't count");
        // Writing the same value again isn't a change either.
        let t = all[0].modified;
        db.set_favorite(all[0].id, true).unwrap();
        assert_eq!(db.load_all().unwrap()[0].modified, t);
    }

    #[test]
    fn filters_and_sorts() {
        let (db, mut l) = lib(&["The Beta", "alpha", "Gamma"]);
        let id = |l: &Library, n: &str| l.games.iter().find(|g| g.name == n).unwrap().id;
        let beta = id(&l, "The Beta");
        let gamma = id(&l, "Gamma");
        db.set_hidden(gamma, true).unwrap();
        db.set_favorite(beta, true).unwrap();
        db.set_user_name(id(&l, "alpha"), Some("Zeta")).unwrap();
        l.replace(db.load_all().unwrap());
        let names = |v: Vec<usize>, l: &Library| {
            v.into_iter().map(|i| l.games[i].display_name().to_string()).collect::<Vec<_>>()
        };
        let f = Filter::default();
        assert_eq!(names(l.filter(&f), &l), vec!["The Beta", "Zeta"]);
        let f = Filter { show_hidden: true, ..Default::default() };
        assert_eq!(names(l.filter(&f), &l), vec!["The Beta", "Gamma", "Zeta"]);
        let f = Filter { show_hidden: true, descending: true, ..Default::default() };
        assert_eq!(names(l.filter(&f), &l), vec!["Zeta", "Gamma", "The Beta"]);
        let f = Filter { favorites_only: true, ..Default::default() };
        assert_eq!(names(l.filter(&f), &l), vec!["The Beta"]);
        // Searching finds games by their custom name, not the hidden one.
        let f = Filter { search: "zeta".into(), ..Default::default() };
        assert_eq!(names(l.filter(&f), &l), vec!["Zeta"]);
        let f = Filter { search: "gamma".into(), ..Default::default() };
        assert!(l.filter(&f).is_empty());
        // Collections skip hidden games; a store with fewer than MIN_STORE_GAMES games gets no
        // collection of its own (its games are in All Games).
        assert!(!l.collections().iter().any(|c| c.kind == CollectionKind::Source(Source::Steam)));
        let all = Collection { kind: CollectionKind::All, name: String::new() };
        assert_eq!(l.collection_games(&all).len(), 2);
    }

    /// Console collections match games whose platform is only a name ("PlayStation 2"), also
    /// after an edit changes it: the resolved name is kept per game and must follow `update`.
    #[test]
    fn console_collections_follow_a_platform_changed_by_update() {
        let mut a = Game::blank("A");
        a.id = 1;
        a.platform = Some("PlayStation 2".into());
        let mut b = Game::blank("B");
        b.id = 2;
        b.platform = Some("windows".into());
        let mut l = Library::new(vec![a, b]);
        let ps2 = Collection { kind: CollectionKind::Platform("sony_playstation2".into()), name: "PS2".into() };
        let ids = |l: &Library| l.collection_games(&ps2).into_iter().map(|i| l.games[i].id).collect::<Vec<_>>();
        assert_eq!(ids(&l), [1]);
        l.update(2, |g| g.platform = Some("Sony PlayStation 2".into()));
        assert_eq!(ids(&l), [1, 2]);
        l.update(1, |g| g.platform = None);
        assert_eq!(ids(&l), [2]);
        let f = Filter { collection: Some(ps2.clone()), ..Default::default() };
        assert_eq!(l.filter(&f).into_iter().map(|i| l.games[i].id).collect::<Vec<_>>(), [2], "the filtered view too");
    }
}
