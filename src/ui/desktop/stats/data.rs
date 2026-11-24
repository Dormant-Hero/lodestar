//! The numbers behind the Statistics page (stats.rs): totals, completion, most played, sources
//! and platforms for a set of games. Pure: no egui, unit-tested below.

use super::super::{Facet, FilterValue, SrcKey};
use crate::library::Library;
use crate::model::{CompletionStatus, GameId};

/// How many games "Most played" lists.
pub(crate) const TOP_N: usize = 10;

/// One bar: what it is, how many games and their play time. `filter`: what clicking it shows
/// (None: nothing to filter by, e.g. "No status").
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    pub label: String,
    pub filter: Option<FilterValue>,
    pub games: usize,
    pub secs: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Stats {
    /// Games counted (hidden ones only when included).
    pub games: usize,
    pub installed: usize,
    /// Hidden games in the scope (also when they aren't counted, so the tile can say so).
    pub hidden: usize,
    pub favorites: usize,
    /// Games with any play time.
    pub played: usize,
    pub total_secs: u64,
    /// Install size of the installed games whose size is known.
    pub size_bytes: u64,
    /// Installed games without a known size.
    pub no_size: usize,
    /// Every status in its natural order, then "No status" (rows without games left out).
    pub completion: Vec<Row>,
    /// Most played first (only games with play time), at most [`TOP_N`].
    pub top: Vec<(GameId, u64)>,
    /// Most games first.
    pub sources: Vec<Row>,
    pub platforms: Vec<Row>,
}

impl Stats {
    /// Average play time of the games that were played (0 when none were).
    pub fn average_secs(&self) -> u64 {
        if self.played == 0 { 0 } else { self.total_secs / self.played as u64 }
    }

    /// Statistics of the games `scope` (indices into `lib.games`, hidden ones included);
    /// hidden games count only with `with_hidden`.
    pub fn compute(lib: &Library, scope: &[usize], with_hidden: bool) -> Stats {
        let mut s = Stats::default();
        let mut completion = [(0usize, 0u64); CompletionStatus::ALL.len() + 1];
        let mut sources: Vec<(SrcKey, usize, u64)> = vec![];
        // display label -> (games, secs)
        let mut platforms: Vec<(String, usize, u64)> = vec![];
        let mut unknown_platform = (0usize, 0u64);
        let mut top: Vec<(GameId, u64)> = vec![];
        for &i in scope {
            let Some(g) = lib.games.get(i) else { continue };
            if g.hidden {
                s.hidden += 1;
                if !with_hidden {
                    continue;
                }
            }
            let secs = g.playtime_secs;
            s.games += 1;
            s.installed += g.installed as usize;
            s.favorites += g.favorite as usize;
            s.played += (secs > 0) as usize;
            s.total_secs += secs;
            if g.installed {
                match g.size_bytes.filter(|&b| b > 0) {
                    Some(b) => s.size_bytes += b,
                    None => s.no_size += 1,
                }
            }
            let ci = g
                .completion()
                .and_then(|c| CompletionStatus::ALL.iter().position(|x| *x == c))
                .unwrap_or(CompletionStatus::ALL.len());
            completion[ci].0 += 1;
            completion[ci].1 += secs;
            let src = SrcKey::of(g);
            match sources.iter_mut().find(|r| r.0 == src) {
                Some(r) => {
                    r.1 += 1;
                    r.2 += secs;
                }
                None => sources.push((src, 1, secs)),
            }
            let mut plat: Option<String> = None;
            Facet::Platform.for_each(g, |v| {
                if plat.is_none() && !v.is_empty() {
                    plat = Some(v.to_string());
                }
            });
            match plat {
                Some(p) => match platforms.iter_mut().find(|r| r.0 == p) {
                    Some(r) => {
                        r.1 += 1;
                        r.2 += secs;
                    }
                    None => platforms.push((p, 1, secs)),
                },
                None => {
                    unknown_platform.0 += 1;
                    unknown_platform.1 += secs;
                }
            }
            if secs > 0 {
                top.push((g.id, secs));
            }
        }
        s.completion = completion
            .iter()
            .enumerate()
            .filter(|(_, (n, _))| *n > 0)
            .map(|(i, &(games, secs))| match CompletionStatus::ALL.get(i) {
                Some(c) => Row {
                    label: c.label().to_string(),
                    filter: Some(FilterValue::Facet(Facet::Completion, c.label().to_string())),
                    games,
                    secs,
                },
                None => Row { label: "No status".into(), filter: None, games, secs },
            })
            .collect();
        // Most played first; ties by name so the list doesn't jump around.
        let name = |id: GameId| lib.get(id).map_or("", |g| g.sort_name.as_str());
        top.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| name(a.0).cmp(name(b.0))));
        top.truncate(TOP_N);
        s.top = top;
        s.sources = sources
            .into_iter()
            .map(|(src, games, secs)| Row {
                label: src.label().to_string(),
                filter: Some(FilterValue::Source(src)),
                games,
                secs,
            })
            .collect();
        s.platforms = platforms
            .into_iter()
            .map(|(p, games, secs)| Row {
                label: crate::ui::platform_icons::short_label(&p).to_string(),
                filter: Some(FilterValue::Facet(Facet::Platform, p)),
                games,
                secs,
            })
            .collect();
        if unknown_platform.0 > 0 {
            s.platforms.push(Row {
                label: "Not set".into(),
                filter: None,
                games: unknown_platform.0,
                secs: unknown_platform.1,
            });
        }
        sort_rows(&mut s.sources, false);
        sort_rows(&mut s.platforms, false);
        s
    }
}

/// Biggest first, by games or by play time (ties: the other measure, then the label).
pub(crate) fn sort_rows(rows: &mut [Row], by_time: bool) {
    rows.sort_by(|a, b| {
        let (x, y) = if by_time {
            ((b.secs, b.games as u64), (a.secs, a.games as u64))
        } else {
            ((b.games as u64, b.secs), (a.games as u64, a.secs))
        };
        x.cmp(&y).then_with(|| a.label.to_lowercase().cmp(&b.label.to_lowercase()))
    })
}

/// "1,234" (plain thousands separators).
pub(crate) fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Play time in plain words: "45 min", "12 h 30 min", "1,234 h"; "0 h" for nothing.
pub(crate) fn hours(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    match (h, m) {
        (0, 0) if secs == 0 => "0 h".into(),
        (0, 0) => "Under 1 min".into(),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{} h", thousands(h)),
        (h, m) if h < 100 => format!("{h} h {m} min"),
        (h, _) => format!("{} h", thousands(h)),
    }
}

/// "1 game" / "12 games".
pub(crate) fn games(n: usize) -> String {
    if n == 1 { "1 game".into() } else { format!("{} games", thousands(n as u64)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Game, Source};

    fn game(id: GameId, name: &str, source: Source, secs: u64) -> Game {
        let mut g = Game::blank(name);
        g.id = id;
        g.source = source;
        g.playtime_secs = secs;
        g
    }

    fn lib() -> Library {
        let h = 3600;
        let mut a = game(1, "Alpha", Source::Steam, 30 * h);
        a.platform = Some("Windows".into());
        a.size_bytes = Some(10 << 30);
        a.edits.completion = Some(CompletionStatus::Beaten);
        a.favorite = true;
        let mut b = game(2, "Beta", Source::Steam, 2 * h);
        b.platform = Some("linux".into());
        b.edits.completion = Some(CompletionStatus::Playing);
        let mut c = game(3, "Gamma", Source::Gog, 0);
        c.installed = false;
        c.size_bytes = Some(5 << 30); // not installed: doesn't count towards the size
        let mut d = game(4, "Delta", Source::Gog, 50 * h);
        d.hidden = true;
        d.platform = Some("Windows".into());
        let mut e = game(5, "Epsilon", Source::Emulator, 2 * h);
        e.platform = Some("PlayStation 2".into());
        Library::new(vec![a, b, c, d, e])
    }

    #[test]
    fn totals_and_breakdowns() {
        let l = lib();
        let all: Vec<usize> = (0..l.games.len()).collect();
        let s = Stats::compute(&l, &all, false);
        assert_eq!((s.games, s.installed, s.hidden, s.favorites, s.played), (4, 3, 1, 1, 3));
        assert_eq!(s.total_secs, 34 * 3600);
        assert_eq!(s.average_secs(), 34 * 3600 / 3);
        assert_eq!((s.size_bytes, s.no_size), (10 << 30, 2));
        let comp: Vec<(&str, usize)> = s.completion.iter().map(|r| (r.label.as_str(), r.games)).collect();
        assert_eq!(comp, [("Playing", 1), ("Beaten", 1), ("No status", 2)]);
        assert!(s.completion[2].filter.is_none());
        // Most played: hidden Delta left out; ties by name.
        assert_eq!(s.top, [(1, 30 * 3600), (2, 2 * 3600), (5, 2 * 3600)]);
        let src: Vec<(&str, usize)> = s.sources.iter().map(|r| (r.label.as_str(), r.games)).collect();
        assert_eq!(src, [("Steam", 2), ("Emulators", 1), ("GOG", 1)]);
        assert_eq!(s.sources[0].filter, Some(FilterValue::Source(SrcKey::source(Source::Steam))));
        let plats: Vec<&str> = s.platforms.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(plats.last(), Some(&"Not set"));
        assert!(plats.contains(&"PS2"), "{plats:?}");

        let s = Stats::compute(&l, &all, true);
        assert_eq!((s.games, s.hidden), (5, 1));
        assert_eq!(s.top[0], (4, 50 * 3600));
        let mut rows = s.sources.clone();
        sort_rows(&mut rows, true);
        assert_eq!(rows[0].label, "GOG", "by play time");
        // A narrower scope.
        let s = Stats::compute(&l, &[1, 2], false);
        assert_eq!((s.games, s.total_secs, s.average_secs()), (2, 2 * 3600, 2 * 3600));
        assert_eq!(Stats::compute(&l, &[], false), Stats::default());
    }

    #[test]
    fn plain_numbers() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1000), "1,000");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(hours(0), "0 h");
        assert_eq!(hours(20), "Under 1 min");
        assert_eq!(hours(45 * 60), "45 min");
        assert_eq!(hours(3600), "1 h");
        assert_eq!(hours(12 * 3600 + 30 * 60), "12 h 30 min");
        assert_eq!(hours(1234 * 3600 + 59), "1,234 h");
        assert_eq!(games(1), "1 game");
        assert_eq!(games(1200), "1,200 games");
    }
}
