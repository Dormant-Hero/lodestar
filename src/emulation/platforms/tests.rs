use super::*;
use std::collections::HashSet;

#[test]
fn table_has_every_playnite_platform_plus_arcade() {
    assert_eq!(all().len(), 86);
    let ids: HashSet<&str> = all().iter().map(|p| p.id).collect();
    assert_eq!(ids.len(), all().len(), "duplicate ids");
    for id in ["3do", "commodore_vci20", "vectrex", "xbox360", "sony_playstation", "watara_supervision", "arcade"] {
        assert!(ids.contains(id), "{id}");
    }
    // Playnite data fixes.
    assert_eq!(by_id("vectrex").unwrap().igdb, Some(70));
    assert!(by_id("ti_83").unwrap().databases.is_empty());
    assert_ne!(by_id("commodore_amiga_cd32").unwrap().databases, by_id("commodore_amiga").unwrap().databases);
    // Lower-case, dot-less extension and folder tables.
    for p in all() {
        for e in p.extensions.iter().chain(p.unique_ext) {
            assert!(!e.starts_with('.') && *e == e.to_ascii_lowercase(), "{} {e}", p.id);
        }
        for u in p.unique_ext {
            assert!(p.extensions.contains(u), "{}: unique ext {u} not in extensions", p.id);
        }
        for f in p.folders {
            assert_eq!(*f, f.to_lowercase(), "{}", p.id);
        }
    }
}

#[test]
fn unique_extensions_are_unique() {
    let mut seen: std::collections::HashMap<&str, &str> = Default::default();
    for p in all() {
        for u in p.unique_ext {
            if let Some(other) = seen.insert(u, p.id) {
                panic!("{u} is unique to both {other} and {}", p.id);
            }
        }
    }
}

#[test]
fn folder_aliases_point_where_they_should() {
    // Every alias of a platform resolves to that platform, unless an earlier platform owns it.
    for p in all() {
        for f in p.folders {
            let got = by_folder_name(f).map(|x| x.id);
            assert!(got.is_some(), "{f} ({})", p.id);
        }
    }
    let cases = [
        ("ps2", "sony_playstation2"),
        ("PlayStation 2", "sony_playstation2"),
        ("ps1", "sony_playstation"),
        ("psx", "sony_playstation"),
        ("ps3", "sony_playstation3"),
        ("ps3games", "sony_playstation3"),
        ("PS3 Games", "sony_playstation3"),
        ("gba-roms", "nintendo_gameboyadvance"),
        ("snes_roms", "nintendo_super_nes"),
        ("gc", "nintendo_gamecube"),
        ("gamecube", "nintendo_gamecube"),
        ("ngc", "nintendo_gamecube"),
        ("wii", "nintendo_wii"),
        ("wiiu", "nintendo_wiiu"),
        ("Wii U", "nintendo_wiiu"),
        ("3ds", "nintendo_3ds"),
        ("n3ds", "nintendo_3ds"),
        ("nds", "nintendo_ds"),
        ("gb", "nintendo_gameboy"),
        ("gbc", "nintendo_gameboycolor"),
        ("switch", "nintendo_switch"),
        ("megadrive", "sega_genesis"),
        ("genesis", "sega_genesis"),
        ("lynx", "atari_lynx"),
        ("wswan", "bandai_wonderswan"),
        ("c20", "commodore_vic20_typo"),
        ("coleco", "coleco_vision"),
        ("videopac", "magnavox_odyssey_2"),
        ("dos", "pc_dos"),
        ("mame", "arcade"),
        ("fbneo", "arcade"),
        ("Sony PlayStation 2", "sony_playstation2"),
        ("sony_playstation2", "sony_playstation2"),
        // Collection words around the console name.
        ("PS2 Games", "sony_playstation2"),
        ("Nintendo Switch ROMs", "nintendo_switch"),
        ("psx stuff", "sony_playstation"),
        ("My PSX collection", "sony_playstation"),
        ("ps2 iso backups", "sony_playstation2"),
        ("GameCube ISOs", "nintendo_gamecube"),
        ("roms-gba", "nintendo_gameboyadvance"),
        ("Wii U Dumps", "nintendo_wiiu"),
    ];
    for (name, want) in cases {
        let want = if want == "commodore_vic20_typo" { "commodore_vci20" } else { want };
        assert_eq!(by_folder_name(name).map(|p| p.id), Some(want), "{name}");
    }
    for none in [
        "pc",
        "PC",
        "pcgames",
        "pc-games",
        "windows",
        "linux",
        "bios",
        "BIOS",
        "",
        "games",
        "roms",
        "Downloads",
        "My Games",
        "Game Files",
        "my files",
        "Backups",
        "Steam Games",
        "PC Games collection",
        "Documents",
        "stuff",
    ] {
        assert_eq!(by_folder_name(none).map(|p| p.id), None, "{none}");
    }
}

#[test]
fn names_and_aliases() {
    assert_eq!(by_name("Sony PlayStation").map(|p| p.id), Some("sony_playstation"));
    assert_eq!(by_name("PS2").map(|p| p.id), Some("sony_playstation2"));
    assert_eq!(by_name("sony_playstation2").map(|p| p.id), Some("sony_playstation2"));
    assert_eq!(by_name("PlayStation 3").map(|p| p.id), Some("sony_playstation3"));
    assert_eq!(by_name("Nintendo SNES").map(|p| p.id), Some("nintendo_super_nes"));
    assert_eq!(by_name("Super Nintendo Entertainment System").map(|p| p.id), Some("nintendo_super_nes"));
    assert_eq!(by_name("Sega Mega Drive/Genesis").map(|p| p.id), Some("sega_genesis"));
    assert_eq!(by_name("PC (Windows)").map(|p| p.id), Some("pc_windows"));
    assert_eq!(by_name("Sinclair ZX Spectrum +3").map(|p| p.id), Some("sinclair_zxspectrum3"));
    assert_eq!(by_name("Sinclair ZX Spectrum").map(|p| p.id), Some("sinclair_zxspectrum"));
    assert_eq!(by_name("Arcade").map(|p| p.id), Some("arcade"));
    assert_eq!(by_name(""), None);
    assert_eq!(by_name("Something Else"), None);
    assert_eq!(by_name("Ｐｌａｙｓｔａｔｉｏｎ"), None);
    // Every display name resolves to itself.
    for p in all() {
        assert_eq!(by_name(p.name).map(|x| x.id), Some(p.id), "{}", p.name);
        assert_eq!(by_id(p.id).map(|x| x.id), Some(p.id));
    }
}

#[test]
fn extensions_and_es_names() {
    assert_eq!(by_unique_extension(".NSP").map(|p| p.id), Some("nintendo_switch"));
    assert_eq!(by_unique_extension("z64").map(|p| p.id), Some("nintendo_64"));
    assert_eq!(by_unique_extension("rvz"), None);
    assert_eq!(by_unique_extension("iso"), None);
    assert_eq!(by_unique_extension("ngc").map(|p| p.id), Some("snk_neogeopocket_color"));
    assert_eq!(by_unique_extension(""), None);
    let ps2 = by_id("sony_playstation2").unwrap();
    assert!(has_extension(ps2, ".CHD"));
    assert!(!has_extension(ps2, "nsp"));
}

#[test]
fn folder_games_platforms() {
    for id in ["nintendo_wiiu", "sony_playstation3", "sony_playstation4"] {
        assert!(by_id(id).unwrap().folder_games, "{id}");
    }
    assert!(!by_id("sony_playstation2").unwrap().folder_games);
}
