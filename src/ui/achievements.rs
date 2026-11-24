//! Achievements on game pages, shared by the desktop and controller UIs: what to show for a game
//! (asking `AppState::achievements`, cheap enough for every frame), the words and numbers both
//! pages use, and the theme bindings' summary.
//!
//! Screenshot runs can show made-up progress instead (`LODESTAR_FAKE_ACHIEVEMENTS=steam|ra|hint|
//! loading|none`): only when the run can't go online (`art::net::local_only`), so it can never
//! show up in real use.

use crate::achievements::summary::Summary;
use crate::achievements::{Achievement, Progress, Service, State};
use crate::app::AppState;
use crate::model::{Game, GameId};
use std::sync::Arc;

/// What the game page shows for a game (None: no section at all). Asks the worker when the
/// answer is missing or old.
pub(crate) fn for_game(st: &mut AppState, id: GameId) -> Option<State> {
    if !st.config.achievements.show {
        return None;
    }
    let g = st.library.get(id)?;
    if let Some(kind) = fake_kind() {
        return fake(&st.library.games, g, &kind, st.achievements.summary(id).map(|s| (s.unlocked, s.total)));
    }
    st.achievements.request(g, &st.config.achievements);
    match st.achievements.state(id)? {
        State::None => None,
        s => Some(s.clone()),
    }
}

/// `for_game` for themes: publishes the game's "unlocked of total" for the `{game.achievements…}`
/// bindings (empty unless the answer is there).
pub(crate) fn publish_binding(st: &mut AppState, id: GameId) {
    let v = match for_game(st, id) {
        Some(State::Ready(p)) if p.total > 0 => Some((p.unlocked.min(p.total), p.total)),
        _ => None,
    };
    crate::theme::render::bind::set_achievements(id, v);
}

/// "12 of 40" ("40 achievements" while the player's own progress isn't known).
pub(crate) fn count_line(p: &Progress) -> String {
    if !p.synced {
        return if p.total == 1 { "1 achievement".into() } else { format!("{} achievements", p.total) };
    }
    format!("{} of {}", p.unlocked.min(p.total), p.total)
}

/// RetroAchievements: "8 in hardcore mode" when that differs from the count.
pub(crate) fn hardcore_line(p: &Progress) -> Option<String> {
    if p.service != Service::RetroAchievements || !p.synced {
        return None;
    }
    let hc = p.list.iter().filter(|a| a.unlocked_at.is_some() && a.hardcore).count() as u32;
    (hc != p.unlocked).then(|| format!("{hc} in hardcore mode"))
}

/// How far along, 0..=1 (0 without any achievements).
pub(crate) fn fraction(p: &Progress) -> f32 {
    if p.total == 0 || !p.synced { 0.0 } else { (p.unlocked.min(p.total) as f32 / p.total as f32).clamp(0.0, 1.0) }
}

/// "30%".
pub(crate) fn percent_label(p: &Progress) -> String {
    format!("{:.0}%", (fraction(p) * 100.0).floor())
}

/// The name shown: a hidden achievement keeps its secret until it is unlocked.
pub(crate) fn shown_name(a: &Achievement) -> &str {
    if masked(a) { "Hidden achievement" } else { &a.name }
}

/// The description shown ("" for a hidden one still locked).
pub(crate) fn shown_description(a: &Achievement) -> &str {
    if masked(a) { "" } else { a.description.trim() }
}

fn masked(a: &Achievement) -> bool {
    a.hidden && a.unlocked_at.is_none()
}

/// A row's right-hand line: when it was unlocked ("3 days ago"); for a locked one how rare it
/// is ("12% of players"), when known.
pub(crate) fn when_line(a: &Achievement) -> String {
    match (a.unlocked_at, a.rarity) {
        (Some(t), _) => crate::model::format_last_played(Some(t)),
        (None, Some(r)) if r < 1.0 => "Under 1% of players".into(),
        (None, Some(r)) => format!("{:.0}% of players", r.min(100.0)),
        (None, None) => String::new(),
    }
}

/// The page link's text.
pub(crate) fn link_label(s: Service) -> String {
    format!("View on {}", s.label())
}

// ------------------------------------------------------------------ screenshot runs

/// `LODESTAR_FAKE_ACHIEVEMENTS` in a run that can't go online (screenshots).
pub(crate) fn fake_kind() -> Option<String> {
    if !crate::art::net::local_only() {
        return None;
    }
    std::env::var("LODESTAR_FAKE_ACHIEVEMENTS").ok().filter(|v| !v.trim().is_empty()).map(|v| v.trim().to_lowercase())
}

const NAMES: [(&str, &str); 20] = [
    ("First Steps", "Finish the opening chapter"),
    ("Treasure Hunter", "Open 50 chests"),
    ("Sharpshooter", "Hit 25 targets in a row"),
    ("Explorer", "Discover every region of the map"),
    ("Collector", "Find all hidden relics"),
    ("No Time to Lose", "Finish a level in under two minutes"),
    ("Untouchable", "Beat a boss without taking damage"),
    ("Night Owl", "Play for an hour after midnight"),
    ("Big Spender", "Spend 10,000 coins in the shop"),
    ("Survivor", "Win a battle with 1 HP left"),
    ("Speed Demon", "Reach top speed on every track"),
    ("Friend of Beasts", "Tame every creature"),
    ("Master Chef", "Cook all 30 recipes"),
    ("Cartographer", "Fill in the whole map"),
    ("Pacifist", "Finish a chapter without fighting"),
    ("Old Habits", "Return to the first village"),
    ("Lucky Streak", "Win five card games in a row"),
    ("Iron Will", "Finish the game on Hard"),
    ("Completionist", "Unlock every other achievement"),
    ("Legend", "Finish the game"),
];

/// Made-up library counts for screenshot runs: most Steam and emulated games get one (the
/// same numbers every run), other stores' games none.
pub(crate) fn fake_counts(games: &[Game], kind: &str) -> std::collections::HashMap<GameId, Summary> {
    const COUNTS: [(u32, u32); 14] = [
        (31, 54),
        (6, 53),
        (13, 81),
        (7, 46),
        (28, 42),
        (19, 50),
        (44, 87),
        (3, 51),
        (12, 37),
        (9, 36),
        (2, 33),
        (54, 54),
        (0, 25),
        (17, 20),
    ];
    if matches!(kind, "none" | "loading" | "hint") {
        return Default::default();
    }
    games
        .iter()
        .filter(|g| crate::achievements::steam::applies(g) || g.emu.is_some())
        .filter(|g| g.id.rem_euclid(5) != 2)
        .map(|g| {
            let (unlocked, total) =
                COUNTS[(g.id.unsigned_abs().wrapping_mul(2_654_435_761) >> 7) as usize % COUNTS.len()];
            let service = if g.emu.is_some() { Service::RetroAchievements } else { Service::Steam };
            (g.id, Summary { unlocked, total, service })
        })
        .collect()
}

/// Made-up progress for the shown game (`kind`: steam (default), ra, hint, loading, none),
/// with the game's made-up library count when it has one. Pictures are other games' icons
/// from the library, so the rows look real.
fn fake(games: &[Game], g: &Game, kind: &str, count: Option<(u32, u32)>) -> Option<State> {
    match kind {
        "none" => return None,
        "loading" => return Some(State::Loading),
        "hint" => {
            return Some(State::Hint(if g.emu.is_some() {
                "Add your RetroAchievements username and Web API key in Settings → Achievements to see your progress"
                    .into()
            } else {
                "Play the game once through Steam to see your progress here".into()
            }));
        }
        _ => {}
    }
    let ra = kind == "ra";
    let pics: Vec<String> = games
        .iter()
        .filter_map(|o| o.art.icon.as_ref().or(o.art.cover.as_ref()))
        .filter(|p| p.is_file())
        .take(40)
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let (unlocked, total) = count.unwrap_or(if ra { (14, 30) } else { (12, 40) });
    let now = crate::model::now_unix();
    let ages = [0.1, 1.2, 3.4, 6.0, 9.5, 16.0, 40.0, 75.0, 120.0, 200.0, 260.0, 300.0, 330.0, 360.0];
    let list = (0..total)
        .map(|i| {
            let (name, desc) = NAMES[i as usize % NAMES.len()];
            let name = if i as usize >= NAMES.len() { format!("{name} II") } else { name.to_string() };
            let unlocked_at = (i < unlocked).then(|| now - (ages[i as usize % ages.len()] * 86400.0) as i64);
            let icon = (!pics.is_empty()).then(|| pics[i as usize % pics.len()].clone());
            Achievement {
                id: format!("ACH_{i}"),
                name,
                description: desc.to_string(),
                icon: icon.clone(),
                icon_locked: icon,
                unlocked_at,
                hardcore: ra && i % 3 != 1,
                hidden: !ra && (i == 2 || i == 30 || i == 35),
                rarity: Some(((i * 37) % 90 + 2) as f32),
            }
        })
        .collect();
    let p = Progress {
        service: if ra { Service::RetroAchievements } else { Service::Steam },
        unlocked,
        total,
        list,
        page_url: Some(if ra {
            "https://retroachievements.org/game/1".into()
        } else {
            "https://steamcommunity.com/stats/1/achievements".into()
        }),
        note: ra.then(|| "Matched by title — your ROM version may not be supported".into()),
        synced: true,
        probable: false,
    };
    Some(State::Ready(Arc::new(p)))
}

/// Made-up emulator sign-ins for Settings → Achievements in screenshot runs.
pub(crate) fn fake_signins() -> Vec<crate::achievements::ra::emulators::Signin> {
    use crate::achievements::ra::emulators::{Signin, SigninStatus};
    let s = |label: &str, status, where_to_sign_in| Signin {
        instance: format!("fake-{}", label.to_lowercase()),
        label: label.into(),
        status,
        where_to_sign_in,
    };
    vec![
        s("RetroArch", SigninStatus::SignedIn { username: "Player1".into() }, "Settings → Achievements"),
        s("RetroArch (Flatpak)", SigninStatus::SignedIn { username: String::new() }, "Settings → Achievements"),
        s("DuckStation (Flatpak)", SigninStatus::NotSignedIn, "Settings → Achievements"),
        s("PCSX2", SigninStatus::SignedIn { username: "SomeoneElse".into() }, "Settings → Achievements"),
        s("PPSSPP", SigninStatus::Disabled, "Settings → Tools → RetroAchievements"),
        s("Dolphin", SigninStatus::Unknown, "Tools → Achievements → Settings"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ach(name: &str, unlocked: Option<i64>, hidden: bool, hardcore: bool) -> Achievement {
        Achievement {
            id: name.into(),
            name: name.into(),
            description: format!("{name} desc"),
            unlocked_at: unlocked,
            hidden,
            hardcore,
            ..Default::default()
        }
    }

    fn progress(service: Service, list: Vec<Achievement>) -> Progress {
        let unlocked = list.iter().filter(|a| a.unlocked_at.is_some()).count() as u32;
        Progress {
            service,
            unlocked,
            total: list.len() as u32,
            list,
            page_url: None,
            note: None,
            synced: true,
            probable: false,
        }
    }

    #[test]
    fn counts_and_percentages() {
        let list = (0..40).map(|i| ach(&format!("a{i}"), (i < 12).then_some(1000 + i), false, false)).collect();
        let p = progress(Service::Steam, list);
        assert_eq!(count_line(&p), "12 of 40");
        assert!((fraction(&p) - 0.3).abs() < 1e-6);
        assert_eq!(percent_label(&p), "30%");
        assert_eq!(hardcore_line(&p), None, "Steam has no hardcore mode");
        let empty = progress(Service::Steam, vec![]);
        assert_eq!(fraction(&empty), 0.0);
        assert_eq!(percent_label(&empty), "0%");
        // Only the list known: no "0 of 40".
        let mut p2 = p.clone();
        p2.synced = false;
        assert_eq!(count_line(&p2), "40 achievements");
        assert_eq!(fraction(&p2), 0.0);
        // More unlocked than there are (a retired achievement): never over 100 %.
        let mut p3 = p.clone();
        p3.unlocked = 41;
        assert_eq!(count_line(&p3), "40 of 40");
        assert_eq!(fraction(&p3), 1.0);
    }

    #[test]
    fn hardcore_only_when_different() {
        let p = progress(
            Service::RetroAchievements,
            vec![ach("a", Some(1), false, true), ach("b", Some(2), false, false), ach("c", None, false, false)],
        );
        assert_eq!(hardcore_line(&p).as_deref(), Some("1 in hardcore mode"));
        let all_hc =
            progress(Service::RetroAchievements, vec![ach("a", Some(1), false, true), ach("c", None, false, false)]);
        assert_eq!(hardcore_line(&all_hc), None);
    }

    #[test]
    fn hidden_ones_keep_their_secret_until_unlocked() {
        let locked = ach("Secret Ending", None, true, false);
        assert_eq!(shown_name(&locked), "Hidden achievement");
        assert_eq!(shown_description(&locked), "");
        let open = ach("Secret Ending", Some(5), true, false);
        assert_eq!(shown_name(&open), "Secret Ending");
        assert_eq!(shown_description(&open), "Secret Ending desc");
        let plain = ach("Legend", None, false, false);
        assert_eq!(shown_name(&plain), "Legend");
        assert_eq!(when_line(&plain), "");
        let rare = Achievement { rarity: Some(0.4), ..plain.clone() };
        assert_eq!(when_line(&rare), "Under 1% of players");
        let common = Achievement { rarity: Some(42.3), ..plain };
        assert_eq!(when_line(&common), "42% of players");
    }

    #[test]
    fn labels() {
        assert_eq!(link_label(Service::RetroAchievements), "View on RetroAchievements");
    }

    #[test]
    fn fakes_need_a_screenshot_run() {
        // Tests never call set_local_only, so the environment variable alone does nothing.
        if !crate::art::net::local_only() {
            assert_eq!(fake_kind(), None);
        }
        let g = Game::blank("x");
        let Some(State::Ready(p)) = fake(&[], &g, "ra", None) else { panic!("ra fake") };
        assert_eq!((p.unlocked, p.total, p.service), (14, 30, Service::RetroAchievements));
        assert!(hardcore_line(&p).is_some() && p.note.is_some());
        let Some(State::Ready(p)) = fake(&[], &g, "steam", None) else { panic!("steam fake") };
        assert_eq!(count_line(&p), "12 of 40");
        assert!(p.list.iter().any(|a| a.hidden && a.unlocked_at.is_none()));
        assert!(matches!(fake(&[], &g, "hint", None), Some(State::Hint(_))));
        assert_eq!(fake(&[], &g, "none", None), None);
    }
}
