//! Emulation (roadmap 5): consoles, emulators, BIOS and how an emulated game is launched.
//!
//! * `platforms` — the console table (Playnite ids, names, ES / libretro names, folder aliases).
//! * `defs` — built-in emulator definitions and their profiles (how each starts a game).
//! * `detect` — finds installed emulators (Flatpak, AppImage, PATH, RetroArch cores) + the user's.
//! * `registry` — the process-wide published state (emulators found + `[emulation]` config) and
//!   building a game's launch command from it (`spec_for`, used by `launch::resolve`).
//! * `options` — emulator switches and extra arguments (per emulator, console, game, action).
//! * `bios` — which BIOS files a console needs, finding / checking / copying them.
//! * `report` — `lodestar --emulation-report`.
//! * `roms` — several ROMs per game and "Choose what to play" (Play's choices).
//! * `romdb` — the embedded ROM hash database (No-Intro / Redump names by CRC / serial).
//! * `zipread` — reading BIOS / keys / firmware out of .zip archives.
//! * `paths` — where a folder really is (symlinks, Flatpak document-portal paths).
//! * `roots` — where each emulator install keeps its files (Flatpak, AppImage home, portable copies).
//!
//! The ROM scanner itself is `scan::emulation` (`Source::Emulator`).

pub mod bios;
pub mod defs;
pub mod detect;
pub mod options;
pub mod paths;
pub mod platforms;
pub mod registry;
pub mod report;
pub mod romdb;
pub mod roms;
pub mod roots;
pub mod zipread;

/// What the library calls an emulated game's source: its console ("PlayStation 2", "GameCube",
/// "Nintendo Switch") instead of "Emulators". None for other games (and emulated games whose
/// console isn't known). The desktop game page, the placeholder cover and the controller use it.
pub fn source_label(g: &crate::model::Game) -> Option<String> {
    let e = g.emu.as_ref()?;
    let p = platforms::by_id(&e.platform).or_else(|| g.platform.as_deref().and_then(platforms::by_name))?;
    Some(console_label(p).to_string())
}

/// A console's name without its maker when the rest still names it ("Sony PlayStation 2" →
/// "PlayStation 2", "Nintendo GameCube" → "GameCube"; "Nintendo 64" and "Nintendo DS" stay; NES is "NES").
pub fn console_label(p: &platforms::Platform) -> &'static str {
    if p.id == "nintendo_nes" {
        return "NES";
    }
    const MAKERS: &[&str] = &[
        "Sony",
        "Nintendo",
        "Microsoft",
        "Sega",
        "NEC",
        "SNK",
        "Bandai",
        "Atari",
        "Commodore",
        "Mattel",
        "Coleco",
        "Magnavox",
        "GCE",
        "Sharp",
        "Sinclair",
        "Amstrad",
        "Thomson",
        "Watara",
    ];
    MAKERS
        .iter()
        .find_map(|m| p.name.strip_prefix(m)?.strip_prefix(' '))
        .filter(|rest| rest.chars().count() >= 3 && rest.starts_with(|c: char| c.is_alphabetic()))
        .unwrap_or(p.name)
}

#[cfg(test)]
mod label_tests {
    use super::*;

    #[test]
    fn source_label_names_the_console() {
        let mut g = crate::model::Game::blank("Crash");
        assert_eq!(source_label(&g), None);
        g.emu = Some(crate::model::EmuRef { platform: "sony_playstation2".into(), ..Default::default() });
        assert_eq!(source_label(&g).as_deref(), Some("PlayStation 2"));
        g.emu.as_mut().unwrap().platform = "nintendo_gamecube".into();
        assert_eq!(source_label(&g).as_deref(), Some("GameCube"));
        g.emu.as_mut().unwrap().platform = "nintendo_ds".into();
        assert_eq!(source_label(&g).as_deref(), Some("Nintendo DS"));
        g.emu.as_mut().unwrap().platform = "nintendo_switch".into();
        assert_eq!(source_label(&g).as_deref(), Some("Switch"));
    }
}
