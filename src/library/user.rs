//! The user's own collections (Steam-style): manual ones (a list of games) and smart ones (a
//! saved filter that updates by itself). Kept in the DB kv table (`collections`, JSON), read
//! leniently: an entry that doesn't parse is skipped, unknown fields are ignored. They show in
//! the desktop sidebar and as controller-mode collections.

use crate::db::Db;
use crate::model::{Game, GameId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub(crate) const KV: &str = "collections";

/// A smart collection's rule: the explorer filter it was made from. Within a field any value
/// matches (with `match_all`, every value of a field a game can have several of); all fields
/// must match (like the explorer). New fields default to "no narrowing", so older rules read
/// as they were.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct SmartRule {
    pub installed: bool,
    /// Only games that aren't installed (explorer "Not installed").
    pub not_installed: bool,
    pub favorites: bool,
    /// Played at least once ("Recently played").
    pub played: bool,
    /// Source keys ("steam", "gog").
    pub sources: Vec<String>,
    /// All of these tags (case-insensitive).
    pub tags: Vec<String>,
    /// Explorer field key ("genre", "platform", "playtime", "size", "score", …) → lowercase
    /// values.
    pub fields: BTreeMap<String, Vec<String>>,
    /// Match all / Match any of several picked values. None (rules made before the switch):
    /// all of the tags, any value within a field.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub match_all: Option<bool>,
    /// Search text (the same fuzzy match as the search box).
    pub search: String,
}

impl SmartRule {
    pub fn is_empty(&self) -> bool {
        !self.installed
            && !self.not_installed
            && !self.favorites
            && !self.played
            && self.sources.is_empty()
            && self.tags.is_empty()
            && self.fields.values().all(Vec::is_empty)
            && self.search.trim().is_empty()
    }

    pub fn matches(&self, g: &Game) -> bool {
        use crate::ui::desktop::Facet;
        if (self.installed && !g.installed)
            || (self.not_installed && g.installed)
            || (self.favorites && !g.favorite)
            || (self.played && g.last_played.is_none())
        {
            return false;
        }
        // Sources by `source_group_key`: a cloud service ("cloud-gfn") is a source of its own.
        if !self.sources.is_empty() && !self.sources.iter().any(|s| s == g.source_group_key()) {
            return false;
        }
        let has = |t: &String| crate::library::has_tag(g, t);
        let tags_ok = match self.match_all {
            Some(false) => self.tags.is_empty() || self.tags.iter().any(has),
            _ => self.tags.iter().all(has),
        };
        if !tags_ok {
            return false;
        }
        let all = self.match_all == Some(true);
        for (k, want) in &self.fields {
            if want.is_empty() {
                continue;
            }
            // A field this version doesn't know matches nothing (never "everything").
            match Facet::ALL.iter().find(|f| f.key() == k) {
                Some(f) if f.matches_mode(g, want, all) => {}
                _ => return false,
            }
        }
        let q = self.search.trim();
        q.is_empty() || crate::search::score(q, g.display_name()).is_some()
    }
}

/// One collection of the user's.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct UserCollection {
    /// Stable id (`coll:<id>` keys, the explorer's remembered selection).
    pub id: u64,
    pub name: String,
    /// Manual collections: the games, in the order they were added.
    pub games: Vec<GameId>,
    /// Smart collections: the rule (`games` is unused then).
    pub rule: Option<SmartRule>,
}

impl UserCollection {
    pub fn is_smart(&self) -> bool {
        self.rule.is_some()
    }

    pub fn contains(&self, g: &Game) -> bool {
        match &self.rule {
            Some(r) => r.matches(g),
            None => self.games.contains(&g.id),
        }
    }
}

/// Read the collections (empty when there are none or the stored value is unreadable).
pub fn load(db: &Db) -> Vec<UserCollection> {
    db.kv_get(KV).map(|s| parse(&s)).unwrap_or_default()
}

/// Lenient parse: entries that don't parse, have no name or repeat an id are dropped.
pub fn parse(s: &str) -> Vec<UserCollection> {
    let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(s) else {
        log::warn!("collections: stored value isn't a list; ignoring it");
        return vec![];
    };
    let mut out: Vec<UserCollection> = vec![];
    for v in values {
        match serde_json::from_value::<UserCollection>(v) {
            Ok(c) if !c.name.trim().is_empty() && !out.iter().any(|o| o.id == c.id) => out.push(c),
            Ok(_) => {}
            Err(e) => log::warn!("collections: skipping an entry: {e}"),
        }
    }
    out
}

pub fn save(db: &Db, colls: &[UserCollection]) {
    match serde_json::to_string(colls) {
        Ok(json) => {
            if let Err(e) = db.kv_set(KV, &json) {
                log::warn!("saving collections: {e:#}");
            }
        }
        Err(e) => log::warn!("saving collections: {e}"),
    }
}

/// Every game in a manual collection (from the stored JSON).
pub fn member_ids(s: &str) -> std::collections::HashSet<GameId> {
    parse(s).into_iter().flat_map(|c| c.games).collect()
}

/// Game `old` is gone from the library (deleted, or merged into `new`): take it out of the
/// manual collections, `new` taking its place. Game ids can be reused by SQLite after a delete,
/// so a stale id would otherwise pull an unrelated game into a collection. True when changed.
pub fn replace_in(colls: &mut [UserCollection], old: GameId, new: Option<GameId>) -> bool {
    let mut changed = false;
    for c in colls.iter_mut() {
        let Some(i) = c.games.iter().position(|g| *g == old) else { continue };
        changed = true;
        match new {
            Some(n) if !c.games.contains(&n) => c.games[i] = n,
            _ => {
                c.games.remove(i);
            }
        }
    }
    changed
}

/// [`replace_in`] on the stored collections (the caller reloads the library afterwards).
pub fn replace_game(db: &Db, old: GameId, new: Option<GameId>) {
    let mut colls = load(db);
    if replace_in(&mut colls, old, new) {
        save(db, &colls);
    }
}

/// An id no collection uses.
pub fn next_id(colls: &[UserCollection]) -> u64 {
    colls.iter().map(|c| c.id).max().unwrap_or(0) + 1
}

/// `name` trimmed, made unique among `colls` ("Co-op" → "Co-op 2"), ignoring collection `skip`.
pub fn unique_name(colls: &[UserCollection], name: &str, skip: Option<u64>) -> String {
    let base = name.trim();
    let base = if base.is_empty() { "Collection" } else { base };
    let taken = |n: &str| colls.iter().any(|c| Some(c.id) != skip && c.name.trim().eq_ignore_ascii_case(n));
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

/// Add `ids` to manual collection `id` (games already in it stay where they are). Returns
/// how many were added.
pub fn add_games(colls: &mut [UserCollection], id: u64, ids: &[GameId]) -> usize {
    let Some(c) = colls.iter_mut().find(|c| c.id == id && !c.is_smart()) else {
        return 0;
    };
    let mut n = 0;
    for g in ids {
        if !c.games.contains(g) {
            c.games.push(*g);
            n += 1;
        }
    }
    n
}

/// Remove `ids` from manual collection `id`; returns how many were in it.
pub fn remove_games(colls: &mut [UserCollection], id: u64, ids: &[GameId]) -> usize {
    let Some(c) = colls.iter_mut().find(|c| c.id == id) else {
        return 0;
    };
    let before = c.games.len();
    c.games.retain(|g| !ids.contains(g));
    before - c.games.len()
}

/// Move collection `id` one place up (`delta` < 0) or down; false when it can't move.
pub fn move_by(colls: &mut [UserCollection], id: u64, delta: i32) -> bool {
    let Some(i) = colls.iter().position(|c| c.id == id) else {
        return false;
    };
    let j = i as i64 + delta as i64;
    if j < 0 || j >= colls.len() as i64 {
        return false;
    }
    colls.swap(i, j as usize);
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    fn game(id: GameId, name: &str) -> Game {
        let mut g = Game::blank(name);
        g.id = id;
        g.source = Source::Steam;
        g
    }

    #[test]
    fn lenient_parse_and_round_trip() {
        let s = r#"[{"id":1,"name":"Co-op","games":[3,4]},{"id":"bad"},{"id":2,"name":""},
            {"id":1,"name":"dup"},{"id":5,"name":"RPGs","rule":{"fields":{"genre":["rpg"]},"x":1}}]"#;
        let v = parse(s);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].games, [3, 4]);
        assert!(v[1].is_smart());
        let back = parse(&serde_json::to_string(&v).unwrap());
        assert_eq!(back, v);
        assert!(parse("{}").is_empty());
        assert!(parse("garbage").is_empty());
    }

    #[test]
    fn manual_membership_and_edits() {
        let mut v = vec![
            UserCollection { id: 1, name: "A".into(), ..Default::default() },
            UserCollection { id: 2, name: "B".into(), rule: Some(SmartRule::default()), ..Default::default() },
        ];
        assert_eq!(add_games(&mut v, 1, &[5, 6, 5]), 2);
        assert_eq!(add_games(&mut v, 1, &[6, 7]), 1);
        assert_eq!(v[0].games, [5, 6, 7]);
        assert_eq!(add_games(&mut v, 2, &[5]), 0, "smart collections take no games");
        assert!(v[0].contains(&game(6, "x")));
        assert_eq!(remove_games(&mut v, 1, &[6, 9]), 1);
        assert_eq!(v[0].games, [5, 7]);
        assert!(move_by(&mut v, 2, -1));
        assert_eq!(v[0].id, 2);
        assert!(!move_by(&mut v, 2, -1));
        assert_eq!(next_id(&v), 3);
        assert_eq!(unique_name(&v, " a ", None), "a 2");
        assert_eq!(unique_name(&v, "A", Some(1)), "A");
        assert_eq!(unique_name(&v, "  ", None), "Collection");
        // A deleted / merged game.
        let mut w = vec![
            UserCollection { id: 1, name: "A".into(), games: vec![5, 6, 7], rule: None },
            UserCollection { id: 2, name: "B".into(), games: vec![6, 9], rule: None },
        ];
        assert!(replace_in(&mut w, 6, Some(9)));
        assert_eq!(w[0].games, [5, 9, 7]);
        assert_eq!(w[1].games, [9], "already in it: not twice");
        assert!(replace_in(&mut w, 9, None));
        assert_eq!((w[0].games.as_slice(), w[1].games.as_slice()), (&[5, 7][..], &[][..]));
        assert!(!replace_in(&mut w, 42, None));
        assert_eq!(member_ids(&serde_json::to_string(&w).unwrap()).len(), 2);
    }

    #[test]
    fn rescans_keep_games_that_are_in_a_collection() {
        use crate::model::ScannedGame;
        let dir = tempfile::tempdir().unwrap();
        let mut db = Db::open(&dir.path().join("l.db")).unwrap();
        let scan = |names: &[&str]| -> Vec<ScannedGame> {
            names.iter().map(|n| ScannedGame::new(Source::Desktop, format!("{n}.desktop"), *n)).collect()
        };
        db.merge_scan(&scan(&["A", "B", "C"]), &[Source::Desktop]).unwrap();
        let id = |db: &Db, n: &str| db.load_all().unwrap().into_iter().find(|g| g.name == n).map(|g| g.id);
        let b = id(&db, "B").unwrap();
        save(&db, &[UserCollection { id: 1, name: "Picks".into(), games: vec![b], rule: None }]);
        db.merge_scan(&scan(&["A"]), &[Source::Desktop]).unwrap();
        assert_eq!(id(&db, "B"), Some(b), "kept (not installed) for its collection");
        assert_eq!(id(&db, "C"), None, "not in a collection: removed as before");
    }

    #[test]
    fn smart_rules_match_like_the_explorer() {
        let mut rpg = game(1, "Baldur's Gate 3");
        rpg.meta.genres = vec!["RPG".into()];
        rpg.installed = true;
        rpg.tags = vec!["Co-op".into()];
        let mut shooter = game(2, "DOOM");
        shooter.meta.genres = vec!["Shooter".into()];
        shooter.installed = false;
        let mut r = SmartRule::default();
        assert!(r.is_empty() && r.matches(&rpg) && r.matches(&shooter));
        r.fields.insert("genre".into(), vec!["rpg".into(), "strategy".into()]);
        assert!(r.matches(&rpg) && !r.matches(&shooter));
        r.tags = vec!["co-op".into()];
        assert!(r.matches(&rpg));
        r.installed = true;
        r.search = "baldur".into();
        assert!(r.matches(&rpg) && !r.is_empty());
        r.sources = vec!["gog".into()];
        assert!(!r.matches(&rpg));
        // Newer filters: not installed, time played / size / score buckets, Match any / all.
        let mut r = SmartRule { not_installed: true, ..Default::default() };
        assert!(!r.is_empty() && !r.matches(&rpg) && r.matches(&shooter));
        r.fields.insert("playtime".into(), vec!["never".into()]);
        assert!(r.matches(&shooter));
        shooter.playtime_secs = 3 * 3600;
        assert!(!r.matches(&shooter));
        r.fields.insert("playtime".into(), vec!["never".into(), "1–10 h".into()]);
        assert!(r.matches(&shooter));
        let mut multi = SmartRule::default();
        rpg.meta.genres = vec!["RPG".into(), "Strategy".into()];
        rpg.tags = vec!["Co-op".into(), "Cozy".into()];
        multi.fields.insert("genre".into(), vec!["rpg".into(), "shooter".into()]);
        assert!(multi.matches(&rpg), "old rules: any value within a field");
        multi.match_all = Some(true);
        assert!(!multi.matches(&rpg));
        multi.fields.insert("genre".into(), vec!["rpg".into(), "strategy".into()]);
        assert!(multi.matches(&rpg) && !multi.matches(&shooter));
        let mut tags = SmartRule { tags: vec!["co-op".into(), "solo".into()], ..Default::default() };
        assert!(!tags.matches(&rpg), "old rules: all of the tags");
        tags.match_all = Some(false);
        assert!(tags.matches(&rpg) && !tags.matches(&shooter));
        // Stored and read back; an old stored rule without the new fields still parses.
        let c = UserCollection { id: 1, name: "x".into(), rule: Some(tags.clone()), ..Default::default() };
        assert_eq!(parse(&serde_json::to_string(&[c]).unwrap())[0].rule.as_ref(), Some(&tags));
        let old = parse(r#"[{"id":2,"name":"o","rule":{"installed":true,"tags":["a"]}}]"#);
        let old_rule = old[0].rule.as_ref().unwrap();
        assert!(old_rule.match_all.is_none() && !old_rule.not_installed);
        let mut odd = SmartRule::default();
        odd.fields.insert("from-the-future".into(), vec!["x".into()]);
        assert!(!odd.matches(&rpg));
    }
}
