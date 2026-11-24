//! "Not a game" cleanup: entries the user marked as not a game, and name rules
//! (the user's own plus conservative built-in ones) that keep junk out of every view.
//!
//! Excluded games keep their DB rows (so "Show again" is instant and a rescan never re-adds
//! them); they are split off when the library is loaded (`split`). What the user marked is
//! remembered by source + source id in the `exclusions` table (db.rs), so a renamed entry stays
//! out; the same table remembers "keep" exceptions: a game a rule caught that the user brought
//! back ("Show again") without switching the whole rule off.
//!
//! Patterns are whole names with `*` as the only wildcard, case-insensitive, ™/®/© and repeated
//! spaces ignored: `Proton *`, `*Redistributable*`, `Steam Linux Runtime*`.

use crate::config::{ExcludeConfig, ExcludeRule};
use crate::model::{Game, Source};
use std::collections::HashSet;

/// A built-in rule: on by default, switchable off in Settings → Libraries → Hidden & excluded.
pub struct BuiltinRule {
    /// Stable id stored in `ExcludeConfig::builtin_off`. Never change these strings.
    pub id: &'static str,
    pub label: &'static str,
    /// Short form shown under the label.
    pub shown: &'static str,
    pub patterns: &'static [&'static str],
}

/// Kept conservative: whole-word patterns only, so real games ("Proton Pulse", "Setup Wars",
/// "Redistribution", "Uninstalled") stay. Steam's own tools are already dropped by the Steam
/// scanner by app id / type; these catch the same things from Lutris, Heroic, Bottles, desktop
/// entries, Wine prefixes and add-ons.
pub const BUILTIN: &[BuiltinRule] = &[
    BuiltinRule {
        id: "proton",
        label: "Proton versions",
        shown: "Proton 9.0, Proton Experimental, GE-Proton …",
        patterns: &[
            "proton 0*",
            "proton 1*",
            "proton 2*",
            "proton 3*",
            "proton 4*",
            "proton 5*",
            "proton 6*",
            "proton 7*",
            "proton 8*",
            "proton 9*",
            "proton experimental*",
            "proton hotfix*",
            "proton next*",
            "proton * runtime",
            "proton-*",
            "ge-proton*",
            "proton ge",
            "proton ge *",
        ],
    },
    BuiltinRule {
        id: "steam-runtime",
        label: "Steam runtimes",
        shown: "Steam Linux Runtime*, Steamworks Common Redistributables",
        patterns: &["steam linux runtime*", "steamworks common redistributables", "steamworks shared*"],
    },
    BuiltinRule {
        id: "redist",
        label: "Redistributables",
        shown: "*Redistributable*, vcredist, DirectX runtime",
        patterns: &[
            "*redistributable*",
            "*redistributables*",
            "* redist",
            "*vcredist*",
            "directx * runtime*",
            "directx runtime*",
        ],
    },
    BuiltinRule {
        id: "uninstall",
        label: "Uninstallers",
        shown: "Uninstall …, … Uninstaller",
        patterns: &["uninstall", "uninstall *", "* uninstall", "*uninstaller", "*uninstaller *", "unins000*"],
    },
    BuiltinRule {
        id: "setup",
        label: "Setup programs and installers",
        shown: "… Setup, … Installer, setup.exe",
        patterns: &["setup", "* setup", "*setup.exe", "installer", "* installer", "*installer.exe", "install.exe"],
    },
];

/// Name as rules see it: lowercase, ™/®/© dropped, whitespace collapsed.
pub fn norm(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for w in name.split_whitespace() {
        let w: String = w.chars().filter(|c| !matches!(c, '™' | '®' | '©')).flat_map(char::to_lowercase).collect();
        if w.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&w);
    }
    out
}

/// Does the (already [`norm`]ed) `name` match the (already normed) pattern? `*` matches any run
/// of characters, everything else itself; the whole name must match.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let n: Vec<char> = name.chars().collect();
    glob_chars(&p, &n)
}

/// [`glob_match`] on pre-split characters (the matcher splits each pattern once).
fn glob_chars(p: &[char], n: &[char]) -> bool {
    let (mut pi, mut ni) = (0usize, 0usize);
    // Where the last `*` was, and the name position it currently swallows up to.
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ni));
            pi += 1;
        } else if pi < p.len() && p[pi] == n[ni] {
            pi += 1;
            ni += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

/// A user pattern as matched (normed; an empty or `*`-only pattern matches nothing, so a slip
/// can never hide the whole library).
pub fn pattern_usable(pattern: &str) -> bool {
    norm(pattern).chars().any(|c| c != '*' && c != ' ')
}

/// The library a rule is limited to: `Some(None)` = every library, `Some(Some(s))` = only `s`,
/// None = a library this build doesn't know (a typo, or a newer build's): the rule matches
/// nothing rather than every library.
pub fn rule_scope(r: &ExcludeRule) -> Option<Option<Source>> {
    match r.source.as_deref() {
        None => Some(None),
        Some(k) => Source::from_key(k).map(Some),
    }
}

/// Why a game is out of the library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The user said "Not a game".
    Marked,
    /// A user rule (index into `ExcludeConfig::rules`).
    Rule(usize),
    /// A built-in rule (index into [`BUILTIN`]).
    Builtin(usize),
}

/// Kinds of rows in the `exclusions` table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// "Not a game": out of every view.
    NotGame,
    /// Brought back from a rule: no rule excludes it.
    Keep,
}

impl Mark {
    pub fn key(self) -> &'static str {
        match self {
            Mark::NotGame => "not_game",
            Mark::Keep => "keep",
        }
    }
    pub fn from_key(s: &str) -> Option<Mark> {
        match s {
            "not_game" => Some(Mark::NotGame),
            "keep" => Some(Mark::Keep),
            _ => None,
        }
    }
}

/// One row of the `exclusions` table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exclusion {
    pub source: Source,
    pub source_id: String,
    /// The name when it was marked (shown when the game's row is gone).
    pub name: String,
    pub mark: Mark,
    pub at: i64,
}

pub type Key = (Source, String);

/// Everything that decides whether a game is shown, compiled once per change.
#[derive(Clone, Debug, Default)]
pub struct Matcher {
    marked: HashSet<Key>,
    keep: HashSet<Key>,
    /// (normed pattern, library, index into the config's rules)
    user: Vec<(Vec<char>, Option<Source>, usize)>,
    /// (index into BUILTIN, its patterns)
    builtin: Vec<(usize, Vec<Vec<char>>)>,
}

impl Matcher {
    pub fn new(cfg: &ExcludeConfig, marks: &[Exclusion]) -> Matcher {
        let mut m = Matcher::default();
        for e in marks {
            let k = (e.source, e.source_id.clone());
            match e.mark {
                Mark::NotGame => m.marked.insert(k),
                Mark::Keep => m.keep.insert(k),
            };
        }
        for (i, r) in cfg.rules.iter().enumerate() {
            if r.enabled
                && pattern_usable(&r.pattern)
                && let Some(src) = rule_scope(r)
            {
                m.user.push((norm(&r.pattern).chars().collect(), src, i));
            }
        }
        for (i, b) in BUILTIN.iter().enumerate() {
            if !cfg.builtin_off.iter().any(|o| o == b.id) {
                m.builtin.push((i, b.patterns.iter().map(|p| p.chars().collect()).collect()));
            }
        }
        m
    }

    /// Why `g` is excluded (None = it is shown). Hand-added games are never caught by rules.
    pub fn reason(&self, g: &Game) -> Option<Reason> {
        self.reason_for(g.source, &g.source_id, &g.name)
    }

    pub fn reason_for(&self, source: Source, source_id: &str, name: &str) -> Option<Reason> {
        // Most libraries have no marks: no key to build then.
        let key = || (source, source_id.to_string());
        if !self.marked.is_empty() && self.marked.contains(&key()) {
            return Some(Reason::Marked);
        }
        if source == Source::Manual
            || (self.user.is_empty() && self.builtin.is_empty())
            || (!self.keep.is_empty() && self.keep.contains(&key()))
        {
            return None;
        }
        let n: Vec<char> = norm(name).chars().collect();
        for (p, src, i) in &self.user {
            if src.is_none_or(|s| s == source) && glob_chars(p, &n) {
                return Some(Reason::Rule(*i));
            }
        }
        for (i, pats) in &self.builtin {
            if pats.iter().any(|p| glob_chars(p, &n)) {
                return Some(Reason::Builtin(*i));
            }
        }
        None
    }
}

/// Split loaded games into (shown, excluded), keeping their order.
pub fn split(games: Vec<Game>, m: &Matcher) -> (Vec<Game>, Vec<Game>) {
    games.into_iter().partition(|g| m.reason(g).is_none())
}

/// Games a rule would catch, for the pattern dialog's preview (by the same matching as `split`;
/// marks and keeps are not consulted: the rule is being written for exactly those).
pub fn rule_matches<'a>(rule: &ExcludeRule, games: impl Iterator<Item = &'a Game>) -> Vec<&'a Game> {
    let Some(src) = rule_scope(rule).filter(|_| pattern_usable(&rule.pattern)) else {
        return vec![];
    };
    let p = norm(&rule.pattern);
    games
        .filter(|g| g.source != Source::Manual && src.is_none_or(|s| s == g.source) && glob_match(&p, &norm(&g.name)))
        .collect()
}

/// The pattern "…and everything named like this" starts with: the name with a trailing version
/// / build / bitness ("Proton 9.0-4", "Foo Redist 2019 (x64)") replaced by `*`, else the name
/// itself (an exact rule).
pub fn suggest_pattern(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let versionish = |w: &str| {
        let w = w.trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | ','));
        let lw = w.to_ascii_lowercase();
        !w.is_empty()
            && (w.chars().next().is_some_and(|c| c.is_ascii_digit())
                || (lw.starts_with('v') && lw[1..].chars().next().is_some_and(|c| c.is_ascii_digit()))
                || matches!(lw.as_str(), "x64" | "x86" | "64-bit" | "32-bit" | "beta" | "alpha" | "rc" | "build"))
    };
    let mut keep = words.len();
    while keep > 1 && versionish(words[keep - 1]) {
        keep -= 1;
    }
    if keep == words.len() {
        return name.trim().to_string();
    }
    format!("{} *", words[..keep].join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> ExcludeConfig {
        ExcludeConfig::default()
    }

    fn builtin_hits(name: &str) -> bool {
        Matcher::new(&cfg(), &[]).reason_for(Source::Lutris, "x", name).is_some()
    }

    #[test]
    fn glob_basics() {
        let g = |p: &str, n: &str| glob_match(&norm(p), &norm(n));
        assert!(g("Proton *", "Proton 9.0"));
        assert!(!g("Proton *", "Protonmail"));
        assert!(!g("Proton *", "Proton"));
        assert!(g("*Redistributable*", "Microsoft Visual C++ 2015 Redistributable (x64)"));
        assert!(g("steam linux runtime*", "Steam Linux Runtime 3.0 (sniper)"));
        assert!(g("a*b*c", "aXXbYYc"));
        assert!(!g("a*b*c", "aXXbYY"));
        assert!(g("*", "anything"));
        assert!(g("exact name", "EXACT   Name™"));
        assert!(!g("exact name", "exact name 2"));
        assert!(g("**x", "x"));
        assert!(g("é*", "Été")); // non-ASCII lowercases too
    }

    #[test]
    fn builtin_rules_catch_junk() {
        for n in [
            "Proton 9.0",
            "Proton 10.0-1",
            "Proton Experimental",
            "Proton Hotfix",
            "Proton EasyAntiCheat Runtime",
            "Proton BattlEye Runtime",
            "GE-Proton9-20",
            "Proton-GE 8",
            "Steam Linux Runtime 3.0 (sniper)",
            "Steamworks Common Redistributables",
            "Microsoft Visual C++ 2015-2022 Redistributable (x64)",
            "DirectX Redist",
            "DirectX End-User Runtime",
            "vcredist_x64",
            "Uninstall",
            "Uninstall Cyberpunk 2077",
            "Witcher 3 Uninstaller",
            "unins000",
            "Setup",
            "GOG Galaxy Setup",
            "setup.exe",
            "Game Installer",
            "Installer",
            "UbisoftConnectInstaller.exe",
        ] {
            assert!(builtin_hits(n), "{n} should be excluded");
        }
    }

    #[test]
    fn builtin_rules_leave_real_games_alone() {
        for n in [
            "Proton Pulse",
            "Protonmail",
            "Prototype 2",
            "Portal 2",
            "Setup Wars",
            "Setups and Downs",
            "The Installation",
            "Install Base",
            "Uninstalled",
            "Redistribution",
            "Redout",
            "Steam Prison",
            "Steamworld Dig 2",
            "DirectX Hero",
            "Runtime Error",
            "Half-Life 2",
            "Baldur's Gate 3",
            "Cyberpunk 2077",
            "Steam Linux",
        ] {
            assert!(!builtin_hits(n), "{n} must not be excluded");
        }
    }

    #[test]
    fn rules_switch_off_scope_and_keep() {
        let mut c = cfg();
        c.builtin_off.push("proton".into());
        c.rules.push(ExcludeRule { pattern: "Foo*".into(), source: Some("desktop".into()), enabled: true });
        c.rules.push(ExcludeRule { pattern: "Bar".into(), source: None, enabled: false });
        let marks = vec![
            Exclusion { source: Source::Steam, source_id: "1".into(), name: "Real".into(), mark: Mark::NotGame, at: 0 },
            Exclusion {
                source: Source::Lutris,
                source_id: "kept".into(),
                name: "Setup".into(),
                mark: Mark::Keep,
                at: 0,
            },
        ];
        let m = Matcher::new(&c, &marks);
        assert_eq!(m.reason_for(Source::Lutris, "a", "Proton 9.0"), None, "built-in switched off");
        assert_eq!(m.reason_for(Source::Desktop, "a", "Foo Tool"), Some(Reason::Rule(0)));
        assert_eq!(m.reason_for(Source::Lutris, "a", "Foo Tool"), None, "rule is only for desktop entries");
        assert_eq!(m.reason_for(Source::Lutris, "a", "Bar"), None, "disabled rule");
        assert_eq!(m.reason_for(Source::Steam, "1", "Real (renamed)"), Some(Reason::Marked));
        assert_eq!(m.reason_for(Source::Lutris, "kept", "Setup"), None, "brought back from a rule");
        assert_eq!(m.reason_for(Source::Lutris, "other", "Setup"), Some(Reason::Builtin(4)));
        assert_eq!(m.reason_for(Source::Manual, "m", "Setup"), None, "hand-added games are never caught by rules");
    }

    #[test]
    fn a_rule_for_an_unknown_library_matches_nothing() {
        let mut c = cfg();
        c.builtin_off = BUILTIN.iter().map(|b| b.id.to_string()).collect();
        c.rules = vec![ExcludeRule { pattern: "Foo*".into(), source: Some("no-such-library".into()), enabled: true }];
        assert_eq!(Matcher::new(&c, &[]).reason_for(Source::Steam, "1", "Foo Bar"), None);
        assert_eq!(rule_scope(&c.rules[0]), None);
    }

    #[test]
    fn empty_and_star_only_patterns_match_nothing() {
        let mut c = cfg();
        c.builtin_off = BUILTIN.iter().map(|b| b.id.to_string()).collect();
        for p in ["", "*", " * ", "**"] {
            c.rules = vec![ExcludeRule { pattern: p.into(), source: None, enabled: true }];
            assert_eq!(Matcher::new(&c, &[]).reason_for(Source::Steam, "1", "Anything"), None, "{p:?}");
        }
    }

    #[test]
    fn matching_a_big_library_is_fast() {
        let mut c = cfg();
        c.rules.push(ExcludeRule { pattern: "*benchmark*".into(), source: None, enabled: true });
        let m = Matcher::new(&c, &[]);
        let names: Vec<String> =
            (0..10_000).map(|i| format!("Some Fairly Long Game Name Number {i}: The Sequel")).collect();
        let t = std::time::Instant::now();
        let hits = names.iter().filter(|n| m.reason_for(Source::Steam, "1", n).is_some()).count();
        assert_eq!(hits, 0);
        // ~2 ms in release; generous for debug builds and busy machines.
        assert!(t.elapsed() < std::time::Duration::from_millis(500), "{:?}", t.elapsed());
    }

    #[test]
    fn suggested_patterns() {
        assert_eq!(suggest_pattern("Proton 9.0"), "Proton *");
        assert_eq!(suggest_pattern("Microsoft Visual C++ Redist 2019 (x64)"), "Microsoft Visual C++ Redist *");
        assert_eq!(suggest_pattern("Foo Tool v1.2 beta"), "Foo Tool *");
        assert_eq!(suggest_pattern("Uninstall Foo"), "Uninstall Foo");
        assert_eq!(suggest_pattern("2048"), "2048");
        assert_eq!(suggest_pattern("Cyberpunk 2077"), "Cyberpunk *");
    }

    #[test]
    fn split_keeps_order() {
        fn g(id: i64, name: &str) -> Game {
            let mut db = crate::db::Db::open_memory().unwrap();
            db.merge_scan(&[crate::model::ScannedGame::new(Source::Lutris, id.to_string(), name)], &[Source::Lutris])
                .unwrap();
            db.load_all().unwrap().remove(0)
        }
        let games = vec![g(1, "A"), g(2, "Proton 8.0"), g(3, "B")];
        let (shown, out) = split(games, &Matcher::new(&cfg(), &[]));
        assert_eq!(shown.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(), ["A", "B"]);
        assert_eq!(out[0].name, "Proton 8.0");
    }
}
