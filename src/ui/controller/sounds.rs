//! Which navigation sound a press makes: decided from what the press changed (nothing changed
//! at the end of a list → no sound), so the navigation code itself stays untouched.

use super::*;
use crate::media::audio::Sound;

/// What a press can change.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Snap {
    screen: Screen,
    coll: usize,
    game: usize,
    menus: usize,
    /// Selected row, tab, and the row's value (a setting changed with ← / →).
    menu: Option<(usize, usize, Option<String>)>,
    overlays: usize,
    home: (usize, Option<usize>, bool, usize, bool),
    details_focus: Option<usize>,
    library_gen: u64,
    running: Option<GameId>,
}

impl Snap {
    pub(super) fn of(ui: &ControllerUi, st: &AppState) -> Snap {
        let h = &ui.home;
        Snap {
            screen: ui.screen,
            coll: ui.coll_sel,
            game: ui.game_sel,
            menus: ui.menus.len(),
            menu: ui.menus.last().map(|m| (m.sel, m.tab, m.items.get(m.sel).and_then(|i| i.value.clone()))),
            overlays: ui.osk.is_some() as usize + ui.viewer.is_some() as usize + ui.sys.menus.len(),
            home: (h.row, h.cols.get(h.row).copied(), h.rail_focus, h.rail_sel, h.group.is_some()),
            details_focus: ui.details.as_ref().map(|d| d.focus),
            library_gen: st.library.generation,
            running: crate::ui::launching::running(st),
        }
    }

    /// How deep the screen is: opening goes deeper, going back less deep.
    fn depth(&self) -> usize {
        let screen = match self.screen {
            Screen::Home | Screen::Collections => 0,
            Screen::Games => 1,
            Screen::Details => 2,
        };
        screen + self.home.4 as usize + self.menus + self.overlays
    }
}

/// The sound for `nav`, which changed `a` into `b`.
pub(super) fn sound_for(nav: Nav, a: &Snap, b: &Snap) -> Option<Sound> {
    if b.running.is_some() && b.running != a.running {
        return Some(Sound::Launch);
    }
    if nav == Nav::Favorite && b.library_gen != a.library_gen && a.menus == 0 {
        return Some(Sound::Favorite);
    }
    let moved = Snap { library_gen: a.library_gen, running: a.running, ..b.clone() };
    if moved == *a {
        return None; // the end of a list, a press that did nothing
    }
    match b.depth().cmp(&a.depth()) {
        std::cmp::Ordering::Greater => return Some(Sound::Select),
        std::cmp::Ordering::Less => return Some(Sound::Back),
        std::cmp::Ordering::Equal => {}
    }
    Some(match nav {
        Nav::PageUp | Nav::PageDown | Nav::JumpUp | Nav::JumpDown => Sound::QuickSysSelect,
        Nav::Accept => Sound::Select,
        Nav::Back => Sound::Back,
        _ if a.menus == 0 && a.overlays == 0 && a.screen == Screen::Collections => Sound::SystemBrowse,
        _ => Sound::Scroll,
    })
}

/// After `nav` was handled: play its sound (nothing when sounds are off).
pub(super) fn after_nav(ui: &ControllerUi, st: &AppState, nav: Nav, before: &Snap) {
    if !st.config.controller.sounds_enabled(&st.controller_theme.file) {
        return;
    }
    if let Some(s) = sound_for(nav, before, &Snap::of(ui, st)) {
        crate::media::audio::global().play(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap() -> Snap {
        Snap {
            screen: Screen::Games,
            coll: 0,
            game: 3,
            menus: 0,
            menu: None,
            overlays: 0,
            home: (0, None, false, 0, false),
            details_focus: None,
            library_gen: 1,
            running: None,
        }
    }

    #[test]
    fn sounds_follow_what_changed() {
        let a = snap();
        assert_eq!(sound_for(Nav::Down, &a, &a), None, "end of the list: silent");
        let b = Snap { game: 4, ..a.clone() };
        assert_eq!(sound_for(Nav::Down, &a, &b), Some(Sound::Scroll));
        assert_eq!(sound_for(Nav::PageDown, &a, &b), Some(Sound::QuickSysSelect));
        let d = Snap { screen: Screen::Details, details_focus: Some(0), ..a.clone() };
        assert_eq!(sound_for(Nav::Accept, &a, &d), Some(Sound::Select));
        assert_eq!(sound_for(Nav::Back, &d, &a), Some(Sound::Back));
        let m = Snap { menus: 1, menu: Some((0, 0, None)), ..a.clone() };
        assert_eq!(sound_for(Nav::Menu, &a, &m), Some(Sound::Select));
        assert_eq!(sound_for(Nav::Back, &m, &a), Some(Sound::Back));
        let f = Snap { library_gen: 2, ..a.clone() };
        assert_eq!(sound_for(Nav::Favorite, &a, &f), Some(Sound::Favorite));
        assert_eq!(sound_for(Nav::Favorite, &a, &a), None);
        let l = Snap { running: Some(7), ..d.clone() };
        assert_eq!(sound_for(Nav::Accept, &d, &l), Some(Sound::Launch));
        // Collections: browsing systems.
        let c = Snap { screen: Screen::Collections, ..a.clone() };
        let c2 = Snap { coll: 1, ..c.clone() };
        assert_eq!(sound_for(Nav::Right, &c, &c2), Some(Sound::SystemBrowse));
        // A settings value changed with ← / →.
        let s = Snap { menus: 1, menu: Some((2, 4, Some("Off".into()))), ..a.clone() };
        let s2 = Snap { library_gen: 5, ..s.clone() };
        assert_eq!(sound_for(Nav::Right, &s, &s2), None, "only the library moved: no sound");
        let s3 = Snap { menu: Some((2, 4, Some("On".into()))), ..s.clone() };
        assert_eq!(sound_for(Nav::Right, &s, &s3), Some(Sound::Scroll), "a value changed");
        let s4 = Snap { menu: Some((3, 4, None)), ..s.clone() };
        assert_eq!(sound_for(Nav::Down, &s, &s4), Some(Sound::Scroll));
    }
}
