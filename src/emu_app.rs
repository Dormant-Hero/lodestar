//! Play ▾ → "Play with ▸ <emulator — kind · profile>" (desktop and controller mode): Play, this
//! once with another emulator / profile of the game's console; "Always use this for this game"
//! also makes it the game's own choice (Edit → Installation → Emulator).

use crate::app::AppState;
use crate::emulation::detect::{Emulator, Profile};
use crate::emulation::registry::{self, Registry};
use crate::model::{Game, GameId};

/// One "Play with" entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlayWith {
    pub emulator: String,
    pub profile: String,
    /// "PCSX2 — Flatpak · Default".
    pub label: String,
    /// What Play uses now.
    pub current: bool,
}

/// "PCSX2 — Flatpak · Default": the emulator, how it is installed, the profile.
pub fn choice_label(e: &Emulator, p: &Profile) -> String {
    format!("{} — {} · {}", e.name, e.kind_label(), p.name)
}

/// The emulators + profiles that can start this game, the one Play uses now first marked.
/// Empty for other games, and when there is nothing to choose between (one way only).
pub fn choices(reg: &Registry, g: &Game) -> Vec<PlayWith> {
    let Some(e) = g.emu.as_ref() else { return vec![] };
    if registry::unplayable_ref(e).is_some() {
        return vec![];
    }
    // What Play uses: an emulator Play action's choice, else the game's.
    let now = match g.play_action() {
        Some(a) if a.kind == crate::model::ActionKind::Emulator => registry::action_choice(reg, g, a).ok(),
        Some(_) => None,
        None => registry::choose_for(reg, Some(g), e),
    };
    let mut out = vec![];
    let mut found: Vec<(&Emulator, Profile)> = vec![];
    for emu in reg.emulators.iter().filter(|x| !x.blocked) {
        for p in crate::emulation::detect::profiles(emu, &reg.config).into_iter().filter(|p| p.supports(&e.platform)) {
            found.push((emu, p));
        }
    }
    let labels: Vec<String> = found.iter().map(|(emu, p)| choice_label(emu, p)).collect();
    for (i, (emu, p)) in found.iter().enumerate() {
        let current = now.as_ref().is_some_and(|(x, q)| x.id == emu.id && q.id == p.id);
        // Copies of one emulator (an old AppImage kept next to the new one) would read the same:
        // they say their folder instead ("RPCS3 — RPCS3_old · Default").
        let label = if labels.iter().filter(|l| **l == labels[i]).count() > 1 {
            format!("{} · {}", emu.copy_label(), p.name)
        } else {
            labels[i].clone()
        };
        out.push(PlayWith { emulator: emu.id.clone(), profile: p.id.clone(), label, current });
    }
    out.sort_by_key(|c| !c.current);
    if out.len() < 2 { vec![] } else { out }
}

/// The emulator Play starts this game with: an emulator Play action's, else the game's.
pub fn play_emulator(reg: &Registry, g: &Game) -> Option<Emulator> {
    let e = g.emu.as_ref()?;
    match g.play_action() {
        Some(a) if a.kind == crate::model::ActionKind::Emulator => {
            registry::action_choice(reg, g, a).ok().map(|(x, _)| x)
        }
        _ => registry::choose_for(reg, Some(g), e).map(|(x, _)| x),
    }
}

/// [`choices`] against the published registry.
pub fn choices_now(g: &Game) -> Vec<PlayWith> {
    registry::current().map(|r| choices(&r, g)).unwrap_or_default()
}

/// The lines of "Choose what to play" for `g` against the published registry (empty: Play has
/// one way only).
pub fn play_entries_now(g: &Game) -> Vec<registry_roms::Entry> {
    registry_roms::entries(registry::current().as_deref(), g)
}

use crate::emulation::roms as registry_roms;

/// What Play changes in (a copy of) the game before starting it: the line picked in "Choose
/// what to play", else the one remembered with "Always use this" when it is still offered.
pub fn apply_play_choice(game: &mut Game, picked: Option<&crate::model::PlayChoice>) {
    let c = match picked {
        Some(c) => Some(c.clone()),
        None if game.edits.play_default.is_some() => registry_roms::remembered(game, &play_entries_now(game)),
        None => None,
    };
    if let Some(c) = c {
        registry_roms::apply(game, &c);
    }
}

impl AppState {
    /// Per frame: the one-time check of a config from before found the folders named
    /// "Emulators" on the drives its emulators came from — they become emulator folders
    /// (Settings → Emulation), so nothing disappears. Saved once.
    pub(crate) fn poll_emulator_dirs(&mut self) {
        let Some(dirs) = registry::take_migration() else { return };
        let e = &mut self.config.emulation;
        if e.emulator_dirs_checked {
            return;
        }
        for d in dirs {
            if !e.emulator_dirs.contains(&d) {
                e.emulator_dirs.push(d);
            }
        }
        e.emulator_dirs_checked = true;
        self.save_config();
        self.refresh_emulation(false);
    }

    /// Play from a UI: with several ways to play and none remembered, the lines to ask about
    /// (the UI shows "Choose what to play"); else the game starts (None).
    pub fn play_or_choose(&mut self, id: GameId) -> Option<Vec<registry_roms::Entry>> {
        let g = self.library.get(id)?;
        let entries = play_entries_now(g);
        if entries.len() >= 2 && registry_roms::remembered(g, &entries).is_none() {
            return Some(entries);
        }
        self.launch(id);
        None
    }

    /// Start one line of "Choose what to play" (`always`: from now on without asking).
    pub fn launch_choice(&mut self, id: GameId, choice: crate::model::PlayChoice, always: bool) {
        let Some(g) = self.library.get(id) else { return };
        if always {
            let label = play_entries_now(g).into_iter().find(|e| e.choice == choice).map(|e| e.label);
            let game = g.display_name().to_string();
            let mut e = g.edits.clone();
            e.play_default = Some(choice.clone());
            self.set_edits(id, e);
            if let Some(l) = label {
                self.notify(format!("Play starts “{l}” for {game} from now on (Play ▾ → Ask every time)"));
            }
        }
        self.switch.windowed = None;
        self.switch.play_with = None;
        self.switch.play_choice = Some((id, choice));
        self.start_game(id, None);
    }

    /// Fold the library entries of files game `id` lists as its ROMs into it (emu_app.rs,
    /// db/emu_roms.rs).
    pub fn fold_claimed_roms(&mut self, id: GameId, announce: bool) {
        match self.db.fold_claimed_roms(id) {
            Ok(0) => {}
            Ok(_) if !announce => self.reload_library(),
            Ok(n) => {
                self.reload_library();
                let name = self.library.get(id).map(|g| g.display_name().to_string()).unwrap_or_default();
                self.notify(if n == 1 {
                    format!("Its separate entry is now part of {name}")
                } else {
                    format!("{n} separate entries are now part of {name}")
                });
            }
            Err(e) => log::warn!("fold ROMs into {id}: {e:#}"),
        }
    }

    /// Right-click → "Add to another game as a ROM…": `from`'s file becomes one more ROM of
    /// `to` (its entry then folds into `to`).
    pub fn add_as_rom(&mut self, from: GameId, to: GameId) {
        let (Some(f), Some(t)) = (self.library.get(from), self.library.get(to)) else { return };
        let Some(fe) = f.emu.as_ref() else { return };
        let mut list = registry_roms::effective(t);
        let path = registry_roms::stored(&fe.rom, t.install_dir.as_deref());
        let already = registry_roms::list(t).iter().any(|r| r.path == fe.rom);
        let (fname, tname) = (f.display_name().to_string(), t.display_name().to_string());
        if !already {
            list.push(crate::model::GameRom { name: fname.clone(), path });
        }
        let mut e = t.edits.clone();
        e.roms = registry_roms::to_store(&list);
        self.save_edits(to, e, false);
        self.notify(format!("{fname} is now a ROM of {tname} (Edit → Installation)"));
    }

    /// Play ▾ → "Ask every time": forget the remembered line.
    pub fn forget_play_choice(&mut self, id: GameId) {
        let Some(g) = self.library.get(id) else { return };
        if g.edits.play_default.is_none() {
            return;
        }
        let mut e = g.edits.clone();
        e.play_default = None;
        self.set_edits(id, e);
        self.notify("Play asks what to play again");
    }

    /// Play with `emulator` / `profile` this once (`always`: from now on, saved as the game's
    /// choice). Goes through Play's usual path (one game at a time, scripts, playtime).
    pub fn launch_with(&mut self, id: GameId, emulator: &str, profile: &str, always: bool) {
        let Some(g) = self.library.get(id) else { return };
        if always {
            let e = crate::emulation::options::always_use(&g.edits, emulator, profile);
            let name = registry::current()
                .and_then(|r| r.emulator(emulator).map(|x| x.label_in_text()))
                .unwrap_or_else(|| emulator.to_string());
            let game = g.display_name().to_string();
            self.set_edits(id, e);
            self.notify(format!("{game} uses {name} from now on (Edit → Installation)"));
            // This Play uses it too (also when the Play action is the user's own program).
        }
        self.switch.windowed = None;
        self.switch.play_choice = None;
        self.switch.play_with = Some((id, emulator.to_string(), profile.to_string()));
        self.start_game(id, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emulation::detect::InstallKind;
    use crate::model::EmuRef;

    fn emu(id: &str, kind: InstallKind) -> Emulator {
        Emulator {
            id: id.into(),
            def: "pcsx2".into(),
            name: "PCSX2".into(),
            kind,
            how: String::new(),
            user: false,
            cores: vec![],
            args: String::new(),
            auto: true,
            blocked: false,
        }
    }

    #[test]
    fn several_play_actions_ask_and_always_use_remembers() {
        use crate::edit_app::NewGame;
        use crate::model::{ActionKind, GameAction};
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let id = st.add_game(NewGame { name: "Two ways".into(), ..Default::default() }).unwrap();
        let sh = |name: &str, file: &std::path::Path| GameAction {
            name: name.into(),
            kind: ActionKind::Command,
            path: "sh".into(),
            args: format!("-c {}", crate::launch::shell_quote(&format!("touch '{}'", file.display()))),
            play: true,
            ..Default::default()
        };
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        let mut e = st.library.get(id).unwrap().edits.clone();
        e.actions = vec![sh("Normal", &a), sh("Online", &b)];
        st.set_edits(id, e);
        // Both stay Play actions (no "at most one" any more), and Play asks.
        assert_eq!(st.library.get(id).unwrap().edits.actions.iter().filter(|a| a.play).count(), 2);
        let entries = st.play_or_choose(id).expect("Play asks");
        assert_eq!(entries.iter().map(|e| e.label.as_str()).collect::<Vec<_>>(), ["Normal", "Online"]);
        assert!(!a.exists() && !b.exists(), "nothing started while asking");
        let wait =
            |st: &mut AppState| assert!(st.poll_until(Duration::from_secs(15), |s| s.launcher.running().is_none()));
        st.launch_choice(id, entries[1].choice.clone(), true);
        wait(&mut st);
        assert!(b.exists() && !a.exists(), "the second one ran");
        assert_eq!(st.library.get(id).unwrap().play_count, 1, "it counts as play");
        std::fs::remove_file(&b).unwrap();
        // Remembered: Play starts it without asking, also from places without a UI (tray).
        assert!(st.play_or_choose(id).is_none());
        wait(&mut st);
        assert!(b.exists() && !a.exists());
        std::fs::remove_file(&b).unwrap();
        st.launch(id);
        wait(&mut st);
        assert!(b.exists() && !a.exists());
        // Ask every time: asks again; plain Play without a UI runs the first.
        st.forget_play_choice(id);
        assert!(st.play_or_choose(id).is_some());
        st.launch(id);
        wait(&mut st);
        assert!(a.exists());
    }

    #[test]
    fn two_installs_side_by_side_are_both_offered() {
        let reg = Registry {
            emulators: vec![
                emu("pcsx2", InstallKind::Flatpak { app_id: "net.pcsx2.PCSX2".into() }),
                emu("pcsx2-appimage", InstallKind::AppImage { path: "/apps/pcsx2-nightly.AppImage".into() }),
            ],
            config: crate::config::EmulationConfig::detect_nothing(),
            home: "/home/u".into(),
        };
        let mut g = Game::blank("Biohazard Outbreak");
        g.emu = Some(EmuRef {
            platform: "sony_playstation2".into(),
            rom: "/roms/ps2/bio.iso".into(),
            ..Default::default()
        });
        let c = choices(&reg, &g);
        assert_eq!(c.len(), 2);
        assert!(c[0].current);
        assert_eq!(c[0].label, "PCSX2 — Flatpak · Default");
        assert_eq!(c[1].label, "PCSX2 — AppImage · Default");
        g.edits.emulator = Some("pcsx2-appimage".into());
        let c = choices(&reg, &g);
        assert_eq!(c[0].emulator, "pcsx2-appimage");
        assert!(c[0].current && !c[1].current);
        // Not emulated: nothing.
        assert!(choices(&reg, &Game::blank("x")).is_empty());
    }

    #[test]
    fn copies_of_one_emulator_say_their_folder_and_names_are_the_ones_given() {
        let ai = |p: &str| InstallKind::AppImage { path: p.into() };
        let mut ryu = emu("eden", ai("/e/Ryujinx/Ryujinx.AppImage"));
        (ryu.def, ryu.name) = ("eden".into(), "Ryujinx".into());
        let reg = Registry {
            emulators: vec![
                emu("pcsx2", ai("/e/PCSX2/pcsx2.AppImage")),
                emu("pcsx2-appimage", ai("/e/PCSX2_old/pcsx2.AppImage")),
                ryu,
            ],
            config: crate::config::EmulationConfig::detect_nothing(),
            home: "/home/u".into(),
        };
        let mut g = Game::blank("Okami");
        g.emu = Some(EmuRef {
            platform: "sony_playstation2".into(),
            rom: "/roms/ps2/okami.iso".into(),
            ..Default::default()
        });
        let labels: Vec<String> = choices(&reg, &g).into_iter().map(|c| c.label).collect();
        assert_eq!(labels, ["PCSX2 — pcsx2.AppImage · Default", "PCSX2 — PCSX2_old · Default"]);
        // A Ryujinx added "based on" Eden is Ryujinx in Play with too.
        g.emu =
            Some(EmuRef { platform: "nintendo_switch".into(), rom: "/roms/switch/z.nsp".into(), ..Default::default() });
        let mut reg2 = reg.clone();
        let mut eden = emu("eden-2", ai("/e/Eden/Eden.AppImage"));
        (eden.def, eden.name) = ("eden".into(), "Eden".into());
        reg2.emulators.push(eden);
        let labels: Vec<String> = choices(&reg2, &g).into_iter().map(|c| c.label).collect();
        assert!(
            labels.iter().any(|l| l.starts_with("Ryujinx — ")) && labels.iter().any(|l| l.starts_with("Eden — ")),
            "{labels:?}"
        );
    }
}
