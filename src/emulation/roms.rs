//! Several ROMs per game (Playnite's ROM list) and "Choose what to play".
//!
//! An emulated game has one ROM until the user adds more (Edit → Installation → ROMs): the
//! file(s) the scan found, a multi-disc set counting as one ROM. `UserEdits::roms` holds the
//! list once it was changed; its entry with an empty path is the scanned ROM, which follows
//! rescans. The first ROM is the default: Play starts it, and so does an emulator action set to
//! "Default ROM". Other ROMs are stored relative to the game's folder (`{InstallDir}/…`) when
//! they are inside it.
//!
//! Play asks "Choose what to play" when there is more than one way: several Play actions, an
//! emulator Play action set to "Choose when playing" (one line per profile / ROM), or a game
//! with several ROMs and no Play action of its own (one line per ROM). One way only: Play starts
//! it at once. "Always use this" remembers a line (`UserEdits::play_default`).

use super::registry::Registry;
use crate::model::{ActionKind, EmuRef, Game, GameAction, GameRom, PlayChoice};
use std::path::{Path, PathBuf};

/// `GameRom::path` of the scanned ROM.
pub const SCANNED: &str = "";

const INSTALL_DIR: &str = "{InstallDir}";

/// One ROM of a game as Play and the editors show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rom {
    /// What actions and choices store (`GameRom::path`; "" = the scanned ROM).
    pub key: String,
    /// Its name ("Pokémon Blue"): the user's, else the game's (scanned ROM) or the file's.
    pub name: String,
    /// The file or folder it starts.
    pub path: PathBuf,
}

/// A stored ROM path as a real path: `{InstallDir}` is the game's folder, `~/` the home folder.
pub fn expand(stored: &str, install_dir: Option<&Path>) -> PathBuf {
    let s = stored.trim();
    if let Some(rest) = s.strip_prefix(INSTALL_DIR) {
        let rest = rest.trim_start_matches('/');
        return match install_dir {
            Some(d) if rest.is_empty() => d.to_path_buf(),
            Some(d) => d.join(rest),
            None => PathBuf::from(rest),
        };
    }
    if let (Some(rest), Some(h)) = (s.strip_prefix("~/"), dirs::home_dir()) {
        return h.join(rest);
    }
    PathBuf::from(s)
}

/// How a picked ROM is stored: `{InstallDir}/…` when it is inside the game's folder.
pub fn stored(path: &Path, install_dir: Option<&Path>) -> String {
    if let Some(d) = install_dir.filter(|d| d.is_absolute() && d.parent().is_some())
        && let Ok(rel) = path.strip_prefix(d)
    {
        let rel = rel.to_string_lossy();
        return if rel.is_empty() { INSTALL_DIR.to_string() } else { format!("{INSTALL_DIR}/{rel}") };
    }
    path.to_string_lossy().into_owned()
}

/// A ROM's name from its file: the tidied file name ("Pokemon Azul (Spain).gb" → "Pokémon
/// Azul"); a generic launch file ("EBOOT.BIN", "default.xbe") is named after its folder.
pub fn file_label(path: &Path) -> String {
    use crate::scan::emulation::names;
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = names::stem(&file).to_string();
    let base = if crate::scan::emulation::walk::generic_stem(&stem) || stem.is_empty() {
        // <game>/PS3_GAME/USRDIR/EBOOT.BIN → <game>
        path.ancestors()
            .skip(1)
            .filter_map(|a| a.file_name().map(|n| n.to_string_lossy().into_owned()))
            .find(|n| !["usrdir", "ps3_game", "o", "bin", "data"].contains(&n.to_ascii_lowercase().as_str()))
            .unwrap_or(stem)
    } else {
        stem
    };
    let official = names::has_region_tag(&base);
    names::tidy(&base, official)
}

/// The game's ROM list as stored, or the one scanned ROM (empty for games that aren't emulated).
pub fn effective(g: &Game) -> Vec<GameRom> {
    if g.emu.is_none() {
        return vec![];
    }
    if g.edits.roms.is_empty() {
        vec![GameRom { name: String::new(), path: SCANNED.into() }]
    } else {
        g.edits.roms.clone()
    }
}

/// The ROM list to store for `list`: nothing while it is only the scanned ROM under its own name
/// (existing libraries keep empty edits).
pub fn to_store(list: &[GameRom]) -> Vec<GameRom> {
    let plain = |r: &GameRom| r.path.trim().is_empty() && r.name.trim().is_empty();
    if list.is_empty() || (list.len() == 1 && plain(&list[0])) {
        return vec![];
    }
    list.iter().map(|r| GameRom { name: r.name.trim().to_string(), path: r.path.trim().to_string() }).collect()
}

/// The game's ROMs, the default first (empty for games that aren't emulated).
pub fn list(g: &Game) -> Vec<Rom> {
    let Some(e) = g.emu.as_ref() else { return vec![] };
    effective(g)
        .into_iter()
        .map(|r| {
            let scanned = r.path.trim().is_empty();
            let path = if scanned { e.rom.clone() } else { expand(&r.path, g.install_dir.as_deref()) };
            let name = match r.name.trim() {
                "" if scanned => g.display_name().to_string(),
                "" => file_label(&path),
                n => n.to_string(),
            };
            Rom { key: r.path.trim().to_string(), name, path }
        })
        .collect()
}

/// The game's `EmuRef` for one of its ROMs (`key` None = the default, the first of the list).
/// `base` is the scanned one. Err: that ROM was removed from the list.
pub fn rom_ref(g: &Game, base: &EmuRef, key: Option<&str>) -> Result<EmuRef, String> {
    let list = effective(g);
    let entry = match key {
        None => list.first().cloned(),
        Some(k) => list.iter().find(|r| r.path.trim() == k.trim()).cloned(),
    };
    let Some(entry) = entry else {
        return match key {
            Some(_) if g.emu.is_some() => {
                Err("the ROM it starts isn't in the game's ROM list any more — pick another one (Edit → Actions)"
                    .into())
            }
            _ => Ok(base.clone()),
        };
    };
    if entry.path.trim().is_empty() {
        return Ok(base.clone());
    }
    let rom = expand(&entry.path, g.install_dir.as_deref());
    // Another file of the same console: none of the scanned file's identity applies.
    Ok(EmuRef { rom, files: vec![], serial: None, crc: None, dat_name: None, region: None, ..base.clone() })
}

/// The default ROM's `EmuRef` (`base` when the list doesn't say otherwise).
pub fn default_ref(g: &Game, base: &EmuRef) -> EmuRef {
    rom_ref(g, base, None).unwrap_or_else(|_| base.clone())
}

// ---------------------------------------------------------------------------------------------
// Choose what to play
// ---------------------------------------------------------------------------------------------

/// One line of "Choose what to play".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// "Pokémon Blue", "Online", "Play: SaveMGO · Metal Gear Online" (an action's name first).
    pub label: String,
    pub choice: PlayChoice,
}

fn action_label(a: &GameAction) -> String {
    match a.name.trim() {
        "" => "Play".to_string(),
        n => n.to_string(),
    }
}

/// The profiles "Choose when playing" offers for an emulator action: (emulator id, profile id,
/// profile name) of its emulator (automatic = the one Play would use) for the game's console.
pub fn action_profiles(reg: &Registry, g: &Game, a: &GameAction) -> Vec<(String, String, String)> {
    let Some(e) = g.emu.as_ref() else { return vec![] };
    let mut plain = a.clone();
    plain.choose_profile = false;
    let Ok((emu, _)) = super::registry::action_choice(reg, g, &plain) else { return vec![] };
    super::detect::profiles(&emu, &reg.config)
        .into_iter()
        .filter(|p| p.supports(&e.platform))
        .map(|p| (emu.id.clone(), p.id, p.name))
        .collect()
}

/// The ROMs Play can offer: the default always (its own launch says what's wrong if it's gone),
/// the others only while their file or folder is there (a deleted copy stops being offered; it
/// stays in the stored list, so a drive that's only unplugged brings it back).
pub fn playable(g: &Game) -> Vec<Rom> {
    list(g).into_iter().enumerate().filter(|(i, r)| *i == 0 || r.path.exists()).map(|(_, r)| r).collect()
}

/// The lines of "Choose what to play" (empty when Play has one way only: it starts that).
/// `reg` lists the profiles of "Choose when playing" (None: nothing published yet, each such
/// action is one line).
pub fn entries(reg: Option<&Registry>, g: &Game) -> Vec<Entry> {
    let roms = playable(g);
    let plays: Vec<(usize, &GameAction)> = g.edits.actions.iter().enumerate().filter(|(_, a)| a.play).collect();
    let mut out = vec![];
    if plays.is_empty() {
        if roms.len() >= 2 {
            for r in &roms {
                // The list is headed "Choose what to play": each line is just the game's name.
                out.push(Entry {
                    label: r.name.clone(),
                    choice: PlayChoice { rom: Some(r.key.clone()), ..Default::default() },
                });
            }
        }
        return out;
    }
    for (i, a) in plays {
        let emulator = a.kind == ActionKind::Emulator && g.emu.is_some();
        let base = action_label(a);
        let profs: Vec<Option<(String, String, String)>> = match reg {
            Some(reg) if emulator && a.choose_profile => {
                let v = action_profiles(reg, g, a);
                if v.is_empty() { vec![None] } else { v.into_iter().map(Some).collect() }
            }
            _ => vec![None],
        };
        let rom_opts: Vec<Option<&Rom>> =
            if emulator && a.choose_rom && roms.len() >= 2 { roms.iter().map(Some).collect() } else { vec![None] };
        for p in &profs {
            for r in &rom_opts {
                let parts: Vec<&str> =
                    p.iter().map(|p| p.2.as_str()).chain(r.iter().map(|r| r.name.as_str())).collect();
                let label = if parts.is_empty() { base.clone() } else { format!("{base}: {}", parts.join(" · ")) };
                out.push(Entry {
                    label,
                    choice: PlayChoice {
                        action: Some(i),
                        action_name: a.name.trim().to_string(),
                        emulator: p.as_ref().map(|p| p.0.clone()),
                        profile: p.as_ref().map(|p| p.1.clone()),
                        rom: r.map(|r| r.key.clone()),
                    },
                });
            }
        }
    }
    if out.len() < 2 { vec![] } else { out }
}

/// The remembered line ("Always use this") when it is still one of `entries`.
pub fn remembered(g: &Game, entries: &[Entry]) -> Option<PlayChoice> {
    let c = g.edits.play_default.as_ref()?;
    entries.iter().any(|e| e.choice == *c).then(|| c.clone())
}

/// Change (a copy of) `g` so that Play starts `c`: that action is the only Play action and
/// takes the profile / ROM, or that ROM goes first.
pub fn apply(g: &mut Game, c: &PlayChoice) {
    match c.action {
        Some(i) if i < g.edits.actions.len() => {
            for (j, a) in g.edits.actions.iter_mut().enumerate() {
                a.play = j == i;
            }
            let a = &mut g.edits.actions[i];
            if let Some(p) = &c.profile {
                if a.emulator.is_none() {
                    a.emulator.clone_from(&c.emulator);
                }
                a.profile = Some(p.clone());
            }
            if let Some(r) = &c.rom {
                a.rom = Some(r.clone());
            }
            a.choose_profile = false;
            a.choose_rom = false;
        }
        Some(_) => {}
        None => {
            if let Some(k) = &c.rom {
                let mut l = effective(g);
                if let Some(pos) = l.iter().position(|r| r.path.trim() == k.trim()) {
                    let r = l.remove(pos);
                    l.insert(0, r);
                    g.edits.roms = l;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
