//! Emulator switches and extra arguments (0.10, Playnite's "additional arguments" made friendly).
//!
//! Every emulator has a few switches: "Start fullscreen" (from its profiles' `{fs}` / `{win}`
//! words) plus the ones its own documentation lists (`defs::OptionDef`: PCSX2's `-batch`,
//! `-fastboot` …). A switch can be set at four levels, the most specific one that says anything
//! wins:
//!
//! 1. an emulator action (Edit → Actions),
//! 2. the game (Edit → Installation),
//! 3. its console (Settings → Emulation → the console's options),
//! 4. the emulator (Settings → Emulation → the emulator's options),
//!
//! and then its default (fullscreen: Settings → Emulation → "Start games fullscreen"). At each
//! level, words typed into that level's extra arguments beat the switch (`-nofullscreen` typed
//! for a game means windowed, whatever the switch says). Play ▾ → Play in a window always wins.
//!
//! Extra arguments of every level go where the profile's template says (`{args}`), else right
//! before the ROM and its `--`; general levels first, the action's last. A switch that is set
//! drops the words of its other side from the template (PCSX2 fullscreen Off: `-fullscreen`
//! goes, `-nofullscreen` comes) and adds its own when they aren't there yet.

use super::defs::OptionDef;
use super::detect::{Emulator, Profile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One level's switches and extra arguments (`[emulation.emulator_settings.<id>]`,
/// `[emulation.console_settings.<platform>]`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct EmuSettings {
    /// Switch id → on / off (absent = not set here).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, bool>,
    /// Extra arguments for every game (placed by the profile's template).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub extra_args: String,
}

impl EmuSettings {
    pub fn is_empty(&self) -> bool {
        self.options.is_empty() && self.extra_args.trim().is_empty()
    }
}

/// The fullscreen switch's id (its words come from the template).
pub const FULLSCREEN: &str = "fullscreen";

/// A switch as the UI shows it and the command builder applies it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Switch {
    pub id: String,
    pub label: String,
    pub help: String,
    pub on: Vec<String>,
    pub off: Vec<String>,
    /// When no level sets it.
    pub default: bool,
    /// Shown before "More options".
    pub key: bool,
}

impl Switch {
    fn from_def(d: &OptionDef) -> Switch {
        let v = |w: &[&str]| w.iter().map(|s| s.to_string()).collect();
        Switch {
            id: d.id.into(),
            label: d.label.into(),
            help: d.help.into(),
            on: v(d.on),
            off: v(d.off),
            default: d.default,
            key: d.key,
        }
    }

    /// Words that only one side has and that look like flags (`-nofullscreen`, `x=1`): what
    /// typed arguments are recognised by. Shared words (Dolphin's `-C`) say nothing.
    fn telling(&self, on: bool) -> impl Iterator<Item = &String> {
        let (mine, other) = if on { (&self.on, &self.off) } else { (&self.off, &self.on) };
        mine.iter().filter(move |w| !other.contains(w) && (w.starts_with('-') || w.contains('=')))
    }
}

/// The `{fs}` / `{win}` words of a template: (fullscreen words, windowed words).
pub fn fullscreen_words(words: &[String]) -> (Vec<String>, Vec<String>) {
    let (mut fs, mut win) = (vec![], vec![]);
    for w in words {
        if let Some(r) = w.strip_prefix("{fs}") {
            fs.push(r.to_string());
        } else if let Some(r) = w.strip_prefix("{win}") {
            win.push(r.to_string());
        }
    }
    (fs, win)
}

/// The template an emulator starts this profile with (the user's "Custom arguments" replace
/// the profile's).
pub fn template<'a>(emu: &'a Emulator, profile: &'a Profile) -> &'a str {
    if emu.args.trim().is_empty() { profile.args.as_str() } else { emu.args.as_str() }
}

/// The switches of an emulator with a profile: "Start fullscreen" when its arguments can say
/// so, then the ones its definition lists. `fullscreen_default` = Settings → Emulation's
/// "Start games fullscreen".
pub fn switches(emu: &Emulator, profile: &Profile, fullscreen_default: bool) -> Vec<Switch> {
    let words = crate::launch::split_command(template(emu, profile)).unwrap_or_default();
    switches_for(&words, emu.definition().map(|d| d.options).unwrap_or(&[]), &emu.name, fullscreen_default)
}

/// [`switches`] for already split template words and a definition's options.
pub fn switches_for(words: &[String], defs: &[OptionDef], name: &str, fullscreen_default: bool) -> Vec<Switch> {
    let mut out = vec![];
    let (fs, win) = fullscreen_words(words);
    if !fs.is_empty() || !win.is_empty() {
        out.push(Switch {
            id: FULLSCREEN.into(),
            label: "Start fullscreen".into(),
            help: format!("Off: {name} opens in a window"),
            on: fs,
            off: win,
            default: fullscreen_default,
            key: true,
        });
    }
    out.extend(defs.iter().map(Switch::from_def));
    out
}

/// One level, most specific first in a list.
#[derive(Clone, Copy, Debug)]
pub struct Level<'a> {
    pub options: &'a BTreeMap<String, bool>,
    /// That level's extra arguments, already split into words.
    pub words: &'a [String],
}

/// What a switch is, from the levels (most specific first): None = nobody said (default).
pub fn decide(sw: &Switch, levels: &[Level]) -> Option<bool> {
    for l in levels {
        let on = sw.telling(true).any(|w| l.words.contains(w));
        let off = sw.telling(false).any(|w| l.words.contains(w));
        match (on, off) {
            (true, false) => return Some(true),
            (false, true) => return Some(false),
            _ => {}
        }
        if let Some(v) = l.options.get(&sw.id) {
            return Some(*v);
        }
    }
    None
}

/// The value a level would inherit if it set nothing: the levels below it, then the default.
pub fn inherited(sw: &Switch, below: &[Level]) -> bool {
    decide(sw, below).unwrap_or(sw.default)
}

/// Put the switches' words right in already filled words: `out` is the template's words,
/// `extra` the extra arguments (to be placed at the insertion point). For every switch that is
/// set (`decided`), the other side's words go from both and its own are added to `extra` when
/// missing. Fullscreen is filled by the template already (`{fs}` / `{win}`): only typed
/// duplicates and typed words of the other side go. Words of a switch that appear in `extra`
/// and in `out` are kept once (in `out`).
pub fn apply(out: &mut Vec<String>, insert_at: &mut usize, extra: &mut Vec<String>, decided: &[(Switch, bool)]) {
    let mut add: Vec<String> = vec![];
    for (sw, v) in decided {
        let other: Vec<&String> = sw.telling(!*v).collect();
        let mine: &[String] = if *v { &sw.on } else { &sw.off };
        // Drop the other side (from the template too, except fullscreen: `fill` did that).
        if sw.id != FULLSCREEN {
            let mut i = 0;
            while i < out.len() {
                if other.contains(&&out[i]) {
                    out.remove(i);
                    if i < *insert_at {
                        *insert_at -= 1;
                    }
                } else {
                    i += 1;
                }
            }
        }
        extra.retain(|w| !other.contains(&w));
        if sw.id != FULLSCREEN {
            for w in mine {
                if !out.contains(w) && !extra.contains(w) && !add.contains(w) {
                    add.push(w.clone());
                }
            }
        }
    }
    // Typed words that the template already has (`-nofullscreen` typed, `{win}-nofullscreen`
    // filled): once.
    let flags: Vec<&String> = decided.iter().flat_map(|(s, _)| s.telling(true).chain(s.telling(false))).collect();
    let mut seen: Vec<String> = vec![];
    extra.retain(|w| {
        if !flags.contains(&w) {
            return true;
        }
        if out.contains(w) || seen.contains(w) {
            return false;
        }
        seen.push(w.clone());
        true
    });
    add.append(extra);
    *extra = add;
}

/// Does a template word name the game's file (`{rom}`, `{Rom}`, `{ImagePath}` …)?
pub fn is_rom_word(w: &str) -> bool {
    [
        "{rom}",
        "{rom_dir}",
        "{rom_name}",
        "{ImagePath}",
        "{ImageDir}",
        "{ImageName}",
        "{ImageNameNoExt}",
        "{Rom}",
        "{RomDir}",
        "{RomName}",
    ]
    .iter()
    .any(|t| w.contains(t))
}

/// Play ▾ → "Play with ▸ <emulator>": `game` is changed so Play uses that emulator and profile
/// this once (a copy of the game is launched). An emulator Play action takes the choice; any
/// other Play action is set aside (the emulator command runs).
pub fn with_choice(game: &mut crate::model::Game, emulator: &str, profile: &str) {
    let some = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    game.edits.emulator = some(emulator);
    game.edits.emulator_profile = some(profile);
    for a in game.edits.actions.iter_mut().filter(|a| a.play) {
        if a.kind == crate::model::ActionKind::Emulator {
            a.emulator = some(emulator);
            a.profile = some(profile);
        } else {
            a.play = false;
        }
    }
}

/// "Always use this for this game": the game's own choice becomes `emulator` / `profile`
/// (an emulator Play action that names an emulator follows too). Returns the new edits.
pub fn always_use(edits: &crate::model::UserEdits, emulator: &str, profile: &str) -> crate::model::UserEdits {
    let mut e = edits.clone();
    let some = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    e.emulator = some(emulator);
    e.emulator_profile = some(profile);
    for a in
        e.actions.iter_mut().filter(|a| a.play && a.kind == crate::model::ActionKind::Emulator && a.emulator.is_some())
    {
        a.emulator = some(emulator);
        a.profile = some(profile);
    }
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sv(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn pcsx2() -> Vec<Switch> {
        let words = crate::launch::split_command("-batch {fs}-fullscreen {win}-nofullscreen -- {rom}").unwrap();
        switches_for(&words, super::super::defs::by_id("pcsx2").unwrap().options, "PCSX2", true)
    }

    #[test]
    fn switches_come_from_the_template_and_the_definition() {
        let s = pcsx2();
        let ids: Vec<&str> = s.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["fullscreen", "batch", "fastboot", "bigpicture"]);
        assert_eq!(s[0].on, sv(&["-fullscreen"]));
        assert_eq!(s[0].off, sv(&["-nofullscreen"]));
        assert!(s[0].default);
        // No {fs} / {win}: no fullscreen switch.
        assert!(switches_for(&sv(&["{rom}"]), &[], "X", true).is_empty());
    }

    #[test]
    fn the_most_specific_level_wins_and_typed_words_beat_switches() {
        let s = pcsx2();
        let fs = &s[0];
        let none = BTreeMap::new();
        let off: BTreeMap<String, bool> = [("fullscreen".to_string(), false)].into();
        let on: BTreeMap<String, bool> = [("fullscreen".to_string(), true)].into();
        let typed = sv(&["-nofullscreen"]);
        let empty: Vec<String> = vec![];
        assert_eq!(decide(fs, &[Level { options: &none, words: &empty }]), None);
        assert_eq!(
            decide(fs, &[Level { options: &none, words: &empty }, Level { options: &off, words: &empty }]),
            Some(false)
        );
        assert_eq!(
            decide(fs, &[Level { options: &on, words: &empty }, Level { options: &off, words: &empty }]),
            Some(true)
        );
        // Typed at the same level beats that level's switch.
        assert_eq!(decide(fs, &[Level { options: &on, words: &typed }]), Some(false));
        // A more specific switch beats a less specific typed word.
        assert_eq!(
            decide(fs, &[Level { options: &on, words: &empty }, Level { options: &none, words: &typed }]),
            Some(true)
        );
        assert!(!inherited(fs, &[Level { options: &off, words: &empty }]));
        assert!(inherited(fs, &[]));
    }

    #[test]
    fn apply_swaps_sides_and_keeps_words_once() {
        let s = pcsx2();
        let fast = s.iter().find(|s| s.id == "fastboot").unwrap().clone();
        let batch = s.iter().find(|s| s.id == "batch").unwrap().clone();
        let mut out = sv(&["-batch", "-slowboot", "-nofullscreen", "--", "/g.iso"]);
        let mut at = 3;
        let mut extra = sv(&["-nofullscreen", "-x"]);
        apply(&mut out, &mut at, &mut extra, &[(s[0].clone(), false), (fast, true), (batch, true)]);
        assert_eq!(out, sv(&["-batch", "-nofullscreen", "--", "/g.iso"]));
        assert_eq!(at, 2);
        assert_eq!(extra, sv(&["-fastboot", "-x"]));
    }

    #[test]
    fn batch_off_drops_it() {
        let s = pcsx2();
        let batch = s.iter().find(|s| s.id == "batch").unwrap().clone();
        let mut out = sv(&["-batch", "-fullscreen", "--", "/g.iso"]);
        let mut at = 2;
        let mut extra = vec![];
        apply(&mut out, &mut at, &mut extra, &[(batch, false)]);
        assert_eq!(out, sv(&["-fullscreen", "--", "/g.iso"]));
        assert_eq!(at, 1);
        assert!(extra.is_empty());
    }

    #[test]
    fn play_with_sets_the_choice_once() {
        use crate::model::{ActionKind, Game, GameAction};
        let mut g = Game::blank("X");
        g.edits.actions =
            vec![GameAction { play: true, kind: ActionKind::Command, path: "/x".into(), ..Default::default() }];
        let mut once = g.clone();
        with_choice(&mut once, "pcsx2-appimage", "default");
        assert_eq!(once.edits.emulator.as_deref(), Some("pcsx2-appimage"));
        assert!(once.play_action().is_none(), "a program Play action is set aside");
        g.edits.actions[0].kind = ActionKind::Emulator;
        g.edits.actions[0].emulator = Some("pcsx2".into());
        let e = always_use(&g.edits, "pcsx2-appimage", "");
        assert_eq!(e.emulator.as_deref(), Some("pcsx2-appimage"));
        assert_eq!(e.emulator_profile, None);
        assert_eq!(e.actions[0].emulator.as_deref(), Some("pcsx2-appimage"));
    }
}
