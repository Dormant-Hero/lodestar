//! Consoles: Playnite platform ids and names, with the ids other services use for them and the
//! folder names / file extensions that identify them.
//!
//! The table is Playnite's `Platforms.yaml` (all 85 platforms, ids kept for import compatibility,
//! including the `commodore_vci20` typo) with its known data bugs fixed (Vectrex IGDB id 70, not
//! 67; TI-83 has no "TIC-80" database; Amiga CD32 has its own CD32 database name), plus one
//! Lodestar-only platform, `arcade` (Playnite has no arcade / MAME platform).
//!
//! Folder aliases cover ES-DE / EmuDeck, Batocera, RetroPie and the short names people use. A
//! folder called `pc` is never an emulated platform (it usually holds Windows games), and
//! `pc_windows` / `pc_linux` have no folder aliases at all, so they are never scanned as ROMs.

use std::collections::HashMap;
use std::sync::OnceLock;

/// One console / computer platform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Platform {
    /// Playnite platform id ("sony_playstation2").
    pub id: &'static str,
    /// Playnite display name ("Sony PlayStation 2"); what `Game.platform` is set to.
    pub name: &'static str,
    /// Short name ("PS2").
    pub short: &'static str,
    pub igdb: Option<u32>,
    /// Playnite / libretro database names ("Sony - PlayStation 2").
    pub databases: &'static [&'static str],
    /// libretro-thumbnails system name (None: no repo, e.g. Switch).
    pub libretro: Option<&'static str>,
    /// TheGamesDB platform id.
    pub tgdb: Option<u32>,
    /// ES-DE / EmuDeck system folder names (ROM folders, downloaded media), most specific first.
    pub es: &'static [&'static str],
    /// Folder-name aliases, lower-case ("ps2", "playstation2").
    pub folders: &'static [&'static str],
    /// ROM file extensions (lower-case, no dot).
    pub extensions: &'static [&'static str],
    /// Extensions that on their own mean this platform.
    pub unique_ext: &'static [&'static str],
    /// Games can be folders (Wii U, PS3).
    pub folder_games: bool,
}

macro_rules! plat {
    ($id:literal, $name:literal, $short:literal, igdb $igdb:expr, db [$($db:literal),*], lr $lr:expr,
     tgdb $tg:expr, es [$($es:literal),*], folders [$($f:literal),*], ext [$($x:literal),*],
     unique [$($u:literal),*] $(, $fg:ident)?) => {
        Platform {
            id: $id,
            name: $name,
            short: $short,
            igdb: $igdb,
            databases: &[$($db),*],
            libretro: $lr,
            tgdb: $tg,
            es: &[$($es),*],
            folders: &[$($f),*],
            extensions: &[$($x),*],
            unique_ext: &[$($u),*],
            folder_games: plat!(@fg $($fg)?),
        }
    };
    (@fg) => { false };
    (@fg folder_games) => { true };
}

static PLATFORMS: &[Platform] = &[
    plat!("3do", "3DO Interactive Multiplayer", "3DO", igdb Some(50), db ["The 3DO Company - 3DO"],
        lr Some("The 3DO Company - 3DO"), tgdb Some(25), es ["3do"],
        folders ["3do", "panasonic 3do", "3do interactive multiplayer"],
        ext ["chd", "cue", "iso", "bin", "zip", "7z"], unique []),
    plat!("adobe_flash", "Adobe Flash", "Flash", igdb None, db [], lr None, tgdb None, es ["flash"],
        folders ["flash", "swf", "adobe flash"], ext ["swf"], unique ["swf"]),
    plat!("amstrad_cpc", "Amstrad CPC", "CPC", igdb Some(25), db ["Amstrad - CPC"], lr Some("Amstrad - CPC"),
        tgdb Some(4914), es ["amstradcpc"], folders ["amstradcpc", "cpc", "amstrad cpc"],
        ext ["dsk", "sna", "cdt", "cpr", "tap", "voc", "kcr", "m3u", "zip", "7z"], unique ["cdt", "cpr", "sna"]),
    plat!("apple_2", "Apple II", "Apple II", igdb Some(75), db [], lr None, tgdb Some(4942),
        es ["apple2"], folders ["apple2", "appleii", "apple ii"],
        ext ["dsk", "do", "po", "nib", "woz", "2mg", "d13", "zip", "7z"], unique ["do", "po", "woz", "2mg"]),
    plat!("atari_2600", "Atari 2600", "2600", igdb Some(59), db ["Atari - 2600"], lr Some("Atari - 2600"),
        tgdb Some(22), es ["atari2600"], folders ["atari2600", "2600", "a26", "atari 2600"],
        ext ["a26", "bin", "zip", "7z"], unique ["a26"]),
    plat!("atari_5200", "Atari 5200", "5200", igdb Some(66), db ["Atari - 5200"], lr Some("Atari - 5200"),
        tgdb Some(26), es ["atari5200"], folders ["atari5200", "5200", "atari 5200"],
        ext ["a52", "car", "bin", "rom", "xfd", "atr", "zip", "7z"], unique ["a52"]),
    plat!("atari_7800", "Atari 7800", "7800", igdb Some(60), db ["Atari - 7800"], lr Some("Atari - 7800"),
        tgdb Some(27), es ["atari7800"], folders ["atari7800", "7800", "atari 7800"],
        ext ["a78", "bin", "zip", "7z"], unique ["a78"]),
    plat!("atari_8bit", "Atari 8-bit", "Atari 8-bit", igdb Some(65), db [], lr Some("Atari - 8-bit Family"),
        tgdb Some(4943), es ["atari800", "atarixe"],
        folders ["atari800", "atarixe", "xegs", "a800", "atarixl", "atari 8bit", "atari 8 bit"],
        ext ["atr", "atx", "xfd", "xex", "cas", "car", "com", "bin", "rom", "dcm", "zip", "7z"], unique ["atx"]),
    plat!("atari_falcon030", "Atari Falcon030", "Falcon", igdb None, db [], lr None, tgdb None, es [],
        folders ["falcon", "falcon030", "atarifalcon"], ext ["st", "msa", "stx", "dim", "ipf", "m3u", "zip", "7z"],
        unique []),
    plat!("atari_jaguar", "Atari Jaguar", "Jaguar", igdb Some(62), db ["Atari - Jaguar"], lr Some("Atari - Jaguar"),
        tgdb Some(28), es ["atarijaguar"], folders ["atarijaguar", "jaguar", "atari jaguar"],
        ext ["j64", "jag", "rom", "abs", "cof", "bin", "prg", "cue", "cdi", "zip", "7z"], unique ["j64", "jag"]),
    plat!("atari_lynx", "Atari Lynx", "Lynx", igdb Some(61), db ["Atari - Lynx"], lr Some("Atari - Lynx"),
        tgdb Some(4924), es ["atarilynx"], folders ["atarilynx", "lynx", "atari lynx"],
        ext ["lnx", "o", "zip", "7z"], unique ["lnx"]),
    plat!("atari_st", "Atari ST/STE", "Atari ST", igdb Some(63), db ["Atari - ST"], lr Some("Atari - ST"),
        tgdb Some(4937), es ["atarist"], folders ["atarist", "st", "atari st", "atari ste"],
        ext ["st", "msa", "stx", "dim", "ipf", "m3u", "zip", "7z"], unique ["msa", "stx"]),
    plat!("bandai_wonderswan_color", "Bandai WonderSwan Color", "WSC", igdb Some(123),
        db ["Bandai - WonderSwan Color"], lr Some("Bandai - WonderSwan Color"), tgdb Some(4926),
        es ["wonderswancolor"], folders ["wonderswancolor", "wswanc", "wsc", "wonderswan color"],
        ext ["wsc", "ws", "pc2", "zip", "7z"], unique ["wsc"]),
    plat!("bandai_wonderswan", "Bandai WonderSwan", "WonderSwan", igdb Some(57), db ["Bandai - WonderSwan"],
        lr Some("Bandai - WonderSwan"), tgdb Some(4925), es ["wonderswan"],
        folders ["wonderswan", "wswan", "ws"], ext ["ws", "pc2", "zip", "7z"], unique ["ws", "pc2"]),
    plat!("coleco_vision", "Coleco ColecoVision", "ColecoVision", igdb Some(68), db ["Coleco - ColecoVision"],
        lr Some("Coleco - ColecoVision"), tgdb Some(31), es ["colecovision"],
        folders ["colecovision", "coleco"], ext ["col", "cv", "bin", "rom", "zip", "7z"], unique ["col", "cv"]),
    plat!("commodore_64", "Commodore 64", "C64", igdb Some(15),
        db ["Commodore - 64", "Commodore - 64 (PP)", "Commodore - 64 (Tapes)"], lr Some("Commodore - 64"),
        tgdb Some(40), es ["c64"], folders ["c64", "commodore64", "commodore 64"],
        ext ["d64", "d71", "d81", "g64", "t64", "tap", "prg", "p00", "crt", "x64", "nib", "m3u", "zip", "7z"],
        unique ["d64", "g64", "t64", "crt"]),
    plat!("commodore_amiga_cd32", "Commodore Amiga CD32", "CD32", igdb Some(114), db ["Commodore - CD32"],
        lr Some("Commodore - CD32"), tgdb Some(4947), es ["amigacd32"],
        folders ["amigacd32", "cd32", "amiga cd32"], ext ["chd", "cue", "iso", "ccd", "mds", "nrg", "m3u"],
        unique []),
    plat!("commodore_amiga", "Commodore Amiga", "Amiga", igdb Some(16), db ["Commodore - Amiga"],
        lr Some("Commodore - Amiga"), tgdb Some(4911), es ["amiga", "amiga600", "amiga1200"],
        folders ["amiga", "amiga500", "amiga600", "amiga1200", "commodore amiga"],
        ext ["adf", "adz", "dms", "ipf", "lha", "hdf", "hdz", "uae", "m3u", "zip", "7z"],
        unique ["adf", "adz", "dms", "lha", "hdf"]),
    plat!("commodore_cbm5x0", "Commodore CBM-5x0", "CBM-5x0", igdb None, db ["Commodore - CBM-5x0"], lr None,
        tgdb None, es [], folders ["cbm5x0", "cbm500"], ext ["d64", "d80", "d82", "prg", "crt", "zip", "7z"],
        unique []),
    plat!("commodore_cbm2", "Commodore CBM-II", "CBM-II", igdb None, db ["Commodore - CBM-II"], lr None,
        tgdb None, es [], folders ["cbm2", "cbmii"], ext ["d64", "d80", "d82", "prg", "crt", "zip", "7z"],
        unique []),
    plat!("commodore_pet", "Commodore PET", "PET", igdb Some(90), db [], lr Some("Commodore - PET"),
        tgdb None, es ["pet"], folders ["pet", "commodore pet"], ext ["d64", "d80", "d82", "prg", "tap", "zip", "7z"],
        unique []),
    plat!("commodore_plus4", "Commodore Plus/4", "Plus/4", igdb Some(94), db ["Commodore - Plus-4"],
        lr Some("Commodore - Plus-4"), tgdb Some(5007), es ["plus4"],
        folders ["plus4", "cplus4", "c16", "commodore plus4"], ext ["d64", "prg", "tap", "crt", "t64", "zip", "7z"],
        unique []),
    plat!("commodore_vci20", "Commodore VIC20", "VIC-20", igdb Some(71), db ["Commodore - VIC-20"],
        lr Some("Commodore - VIC-20"), tgdb Some(4945), es ["vic20"],
        folders ["vic20", "c20", "commodore vic20", "commodore vic 20"],
        ext ["20", "40", "60", "a0", "b0", "crt", "d64", "prg", "tap", "zip", "7z"], unique ["a0", "b0"]),
    plat!("vectrex", "GCE Vectrex", "Vectrex", igdb Some(70), db ["GCE - Vectrex"], lr Some("GCE - Vectrex"),
        tgdb Some(4939), es ["vectrex"], folders ["vectrex", "gce vectrex"],
        ext ["vec", "gam", "bin", "zip", "7z"], unique ["vec"]),
    plat!("macintosh", "Macintosh", "Mac", igdb Some(14), db [], lr None, tgdb Some(37),
        es ["macintosh"], folders ["macintosh", "mac", "apple macintosh"], ext ["dsk", "img", "hfv", "zip", "7z"],
        unique ["hfv"]),
    plat!("magnavox_odyssey_2", "Magnavox Odyssey 2", "Odyssey 2", igdb Some(133), db ["Magnavox - Odyssey2"],
        lr Some("Magnavox - Odyssey2"), tgdb Some(4927), es ["odyssey2", "videopac"],
        folders ["odyssey2", "videopac", "o2", "magnavox odyssey2"], ext ["o2", "bin", "zip", "7z"], unique ["o2"]),
    plat!("mattel_intellivision", "Mattel Intellivision", "Intellivision", igdb Some(67),
        db ["Mattel - Intellivision"], lr Some("Mattel - Intellivision"), tgdb Some(32),
        es ["intellivision"], folders ["intellivision", "intv", "mattel intellivision"],
        ext ["int", "bin", "rom", "zip", "7z"], unique ["int"]),
    plat!("microsoft_msx", "Microsoft MSX", "MSX", igdb Some(27), db ["Microsoft - MSX"], lr Some("Microsoft - MSX"),
        tgdb Some(4929), es ["msx", "msx1"], folders ["msx", "msx1", "microsoft msx"],
        ext ["rom", "mx1", "mx2", "dsk", "cas", "ri", "m3u", "zip", "7z"], unique ["mx1"]),
    plat!("microsoft_msx2", "Microsoft MSX2", "MSX2", igdb Some(53), db ["Microsoft - MSX2", "Microsoft - MSX 2"],
        lr Some("Microsoft - MSX2"), tgdb None, es ["msx2"], folders ["msx2", "microsoft msx2"],
        ext ["rom", "mx2", "dsk", "cas", "m3u", "zip", "7z"], unique []),
    plat!("xbox360", "Microsoft Xbox 360", "Xbox 360", igdb Some(12),
        db ["Microsoft - Xbox 360", "Microsoft - Xbox 360 (Digital)", "Microsoft - XBOX 360 (Games on Demand)"],
        lr Some("Microsoft - Xbox 360"), tgdb Some(15), es ["xbox360"],
        folders ["xbox360", "x360", "xbox 360"], ext ["iso", "xex", "zar"], unique ["xex", "zar"], folder_games),
    plat!("xbox_one", "Microsoft Xbox One", "Xbox One", igdb Some(49), db [], lr None, tgdb None,
        es ["xboxone"], folders ["xboxone", "xbox one"], ext [], unique []),
    plat!("xbox_series", "Microsoft Xbox Series", "Xbox Series", igdb Some(169), db [], lr None, tgdb None,
        es [], folders ["xboxseries", "xbox series"], ext [], unique []),
    plat!("xbox", "Microsoft Xbox", "Xbox", igdb Some(11), db ["Microsoft - Xbox"], lr Some("Microsoft - Xbox"),
        tgdb Some(14), es ["xbox"], folders ["xbox", "microsoft xbox"], ext ["iso", "xiso", "xbe"],
        unique ["xiso", "xbe"]),
    plat!("nec_pc98", "NEC PC-98", "PC-98", igdb Some(149), db ["NEC - PC-98"], lr Some("NEC - PC-98"),
        tgdb Some(4934), es ["pc98"], folders ["pc98", "pc 98", "nec pc98"],
        ext ["d98", "98d", "fdi", "fdd", "hdi", "hdd", "nhd", "d88", "88d", "2hd", "tfd", "thd", "hdm", "xdf", "zip", "7z"],
        unique ["d98", "98d", "hdi", "nhd"]),
    plat!("nec_pcfx", "NEC PC-FX", "PC-FX", igdb Some(274), db ["NEC - PC-FX"], lr Some("NEC - PC-FX"),
        tgdb Some(4930), es ["pcfx"], folders ["pcfx", "pc fx", "nec pcfx"], ext ["chd", "cue", "ccd", "toc", "zip", "7z"],
        unique []),
    plat!("nec_supergrafx", "NEC SuperGrafx", "SuperGrafx", igdb Some(128), db ["NEC - PC Engine SuperGrafx"],
        lr Some("NEC - PC Engine SuperGrafx"), tgdb None, es ["supergrafx"],
        folders ["supergrafx", "sgx", "nec supergrafx"], ext ["sgx", "pce", "zip", "7z"], unique ["sgx"]),
    plat!("nec_turbografx_16", "NEC TurboGrafx 16", "TurboGrafx-16", igdb Some(86),
        db ["NEC - PC Engine - TurboGrafx 16"], lr Some("NEC - PC Engine - TurboGrafx 16"), tgdb Some(34),
        es ["pcengine", "tg16"],
        folders ["pcengine", "tg16", "pce", "turbografx", "turbografx16", "turbografx 16", "pc engine"],
        ext ["pce", "sgx", "zip", "7z"], unique ["pce"]),
    plat!("nec_turbografx_cd", "NEC TurboGrafx-CD", "TurboGrafx-CD", igdb Some(150),
        db ["NEC - PC Engine CD - TurboGrafx-CD"], lr Some("NEC - PC Engine CD - TurboGrafx-CD"),
        tgdb Some(4955), es ["pcenginecd", "tg-cd"],
        folders ["pcenginecd", "tg-cd", "tgcd", "pcecd", "turbografxcd", "turbografx cd", "pc engine cd"],
        ext ["chd", "cue", "ccd", "toc", "m3u", "zip", "7z"], unique []),
    plat!("nintendo_3ds", "Nintendo 3DS", "3DS", igdb Some(37),
        db ["Nintendo - Nintendo 3DS", "Nintendo - Nintendo 3DS (Digital)", "Nintendo - New Nintendo 3DS",
            "Nintendo - New Nintendo 3DS (Digital)"],
        lr Some("Nintendo - Nintendo 3DS"), tgdb Some(4912), es ["n3ds"],
        folders ["n3ds", "3ds", "nintendo 3ds", "new3ds"],
        ext ["3ds", "cci", "cxi", "cia", "3dsx", "app", "axf", "elf", "zcci", "zcxi", "z3dsx"],
        unique ["3ds", "cci", "cxi", "cia", "3dsx", "zcci", "zcxi", "z3dsx"]),
    plat!("nintendo_64", "Nintendo 64", "N64", igdb Some(4), db ["Nintendo - Nintendo 64", "Nintendo - Nintendo 64DD"],
        lr Some("Nintendo - Nintendo 64"), tgdb Some(3), es ["n64", "n64dd"],
        folders ["n64", "n64dd", "nintendo64", "nintendo 64"], ext ["z64", "n64", "v64", "u1", "ndd", "d64", "zip", "7z"],
        unique ["z64", "n64", "v64", "ndd"]),
    plat!("nintendo_ds", "Nintendo DS", "DS", igdb Some(20),
        db ["Nintendo - Nintendo DS", "Nintendo - Nintendo DS (Download Play)"], lr Some("Nintendo - Nintendo DS"),
        tgdb Some(8), es ["nds"], folders ["nds", "ds", "nintendo ds"],
        ext ["nds", "ids", "srl", "app", "bin", "zip", "7z"], unique ["nds", "ids", "srl"]),
    plat!("nintendo_dsi", "Nintendo DSi", "DSi", igdb Some(159),
        db ["Nintendo - Nintendo DSi", "Nintendo - Nintendo DSi (Digital)"], lr Some("Nintendo - Nintendo DSi"),
        tgdb None, es ["nds"], folders ["dsi", "nintendo dsi"], ext ["dsi", "nds", "zip", "7z"],
        unique ["dsi"]),
    plat!("nintendo_nes", "Nintendo Entertainment System", "NES", igdb Some(18),
        db ["Nintendo - Nintendo Entertainment System"], lr Some("Nintendo - Nintendo Entertainment System"),
        tgdb Some(7), es ["nes", "famicom"], folders ["nes", "famicom", "fc"],
        ext ["nes", "unf", "unif", "fds", "zip", "7z"], unique ["nes", "unf", "unif"]),
    plat!("nintendo_famicom_disk", "Nintendo Family Computer Disk System", "FDS", igdb Some(51),
        db ["Nintendo - Family Computer Disk System"], lr Some("Nintendo - Family Computer Disk System"),
        tgdb Some(4936), es ["fds"], folders ["fds", "famicom disk system", "famicomdisk"],
        ext ["fds", "qd", "zip", "7z"], unique ["fds", "qd"]),
    plat!("nintendo_gameboyadvance", "Nintendo Game Boy Advance", "GBA", igdb Some(24),
        db ["Nintendo - Game Boy Advance"], lr Some("Nintendo - Game Boy Advance"), tgdb Some(5),
        es ["gba"], folders ["gba", "gameboyadvance", "game boy advance"],
        ext ["gba", "agb", "mb", "elf", "bin", "zip", "7z"], unique ["gba", "agb"]),
    plat!("nintendo_gameboycolor", "Nintendo Game Boy Color", "GBC", igdb Some(22), db ["Nintendo - Game Boy Color"],
        lr Some("Nintendo - Game Boy Color"), tgdb Some(41), es ["gbc"],
        folders ["gbc", "gameboycolor", "game boy color"], ext ["gbc", "cgb", "gb", "sgb", "zip", "7z"],
        unique ["gbc", "cgb"]),
    plat!("nintendo_gameboy", "Nintendo Game Boy", "Game Boy", igdb Some(33), db ["Nintendo - Game Boy"],
        lr Some("Nintendo - Game Boy"), tgdb Some(4), es ["gb"], folders ["gb", "gameboy", "game boy"],
        ext ["gb", "dmg", "sgb", "zip", "7z"], unique ["gb", "dmg", "sgb"]),
    plat!("nintendo_gamecube", "Nintendo GameCube", "GameCube", igdb Some(21), db ["Nintendo - GameCube"],
        lr Some("Nintendo - GameCube"), tgdb Some(2), es ["gc", "gamecube"],
        folders ["gc", "gamecube", "ngc", "nintendo gamecube"],
        ext ["rvz", "iso", "gcm", "gcz", "ciso", "wia", "tgc", "dol", "elf", "dff", "m3u"], unique ["gcm", "tgc"]),
    plat!("nintendo_super_nes", "Nintendo SNES", "SNES", igdb Some(19),
        db ["Nintendo - Super Nintendo Entertainment System"],
        lr Some("Nintendo - Super Nintendo Entertainment System"), tgdb Some(6),
        es ["snes", "sfc", "snesna"],
        folders ["snes", "sfc", "snesna", "superfamicom", "super famicom", "super nintendo", "supernintendo"],
        ext ["sfc", "smc", "fig", "swc", "bs", "st", "bml", "zip", "7z"], unique ["sfc", "smc", "fig", "swc"]),
    plat!("nintendo_switch", "Nintendo Switch", "Switch", igdb Some(130), db [], lr None,
        tgdb Some(4971), es ["switch"], folders ["switch", "ns", "nsw", "nintendo switch"],
        ext ["nsp", "xci", "nsz", "xcz", "nro", "nso", "nca"], unique ["nsp", "xci", "nsz", "xcz", "nro", "nso", "nca"]),
    plat!("nintendo_virtualboy", "Nintendo Virtual Boy", "Virtual Boy", igdb Some(87), db ["Nintendo - Virtual Boy"],
        lr Some("Nintendo - Virtual Boy"), tgdb Some(4918), es ["virtualboy"],
        folders ["virtualboy", "vb", "virtual boy"], ext ["vb", "vboy", "bin", "zip", "7z"], unique ["vb", "vboy"]),
    plat!("nintendo_wiiu", "Nintendo Wii U", "Wii U", igdb Some(41), db ["Nintendo - Wii U", "Nintendo - Wii U (Digital)"],
        lr Some("Nintendo - Wii U"), tgdb Some(38), es ["wiiu"], folders ["wiiu", "wii u", "nintendo wii u"],
        ext ["wua", "wud", "wux", "rpx", "wuhb", "iso", "elf", "tmd"], unique ["wud", "wux", "wua", "rpx", "wuhb"],
        folder_games),
    plat!("nintendo_wii", "Nintendo Wii", "Wii", igdb Some(5), db ["Nintendo - Wii", "Nintendo - Wii (Digital)"],
        lr Some("Nintendo - Wii"), tgdb Some(9), es ["wii"], folders ["wii", "nintendo wii"],
        ext ["rvz", "wbfs", "iso", "wia", "ciso", "gcz", "wad", "dol", "elf", "m3u"], unique ["wbfs", "wad"]),
    plat!("pc_dos", "PC (DOS)", "DOS", igdb Some(13), db ["DOS"], lr Some("DOS"), tgdb Some(1),
        es ["dos"], folders ["dos", "msdos", "ms-dos", "pcdos"],
        ext ["conf", "exe", "com", "bat", "dosz", "zip", "7z", "iso", "cue", "img", "ima", "vhd", "m3u"],
        unique ["dosz"]),
    plat!("pc_linux", "PC (Linux)", "Linux", igdb Some(3), db [], lr None, tgdb None, es [], folders [],
        ext [], unique []),
    plat!("pc_windows", "PC (Windows)", "Windows", igdb Some(6), db [], lr None, tgdb Some(1),
        es ["windows"], folders [], ext [], unique []),
    plat!("sega_32x", "Sega 32X", "32X", igdb Some(30), db ["Sega - 32X"], lr Some("Sega - 32X"),
        tgdb Some(33), es ["sega32x", "sega32xjp", "sega32xna"], folders ["sega32x", "sega32xjp", "sega32xna", "32x"],
        ext ["32x", "bin", "md", "smd", "zip", "7z"], unique ["32x"]),
    plat!("sega_cd", "Sega CD", "Sega CD", igdb Some(78), db ["Sega - Mega-CD - Sega CD"],
        lr Some("Sega - Mega-CD - Sega CD"), tgdb Some(21), es ["segacd", "megacd", "megacdjp"],
        folders ["segacd", "megacd", "megacdjp", "mcd", "sega cd", "mega cd"],
        ext ["chd", "cue", "iso", "bin", "m3u", "zip", "7z"], unique []),
    plat!("sega_dreamcast", "Sega Dreamcast", "Dreamcast", igdb Some(23), db ["Sega - Dreamcast"],
        lr Some("Sega - Dreamcast"), tgdb Some(16), es ["dreamcast"],
        folders ["dreamcast", "dc", "sega dreamcast"], ext ["chd", "gdi", "cdi", "cue", "m3u", "elf", "zip", "7z"],
        unique ["gdi", "cdi"]),
    plat!("sega_gamegear", "Sega Game Gear", "Game Gear", igdb Some(35), db ["Sega - Game Gear"],
        lr Some("Sega - Game Gear"), tgdb Some(20), es ["gamegear"], folders ["gamegear", "gg", "game gear"],
        ext ["gg", "bin", "zip", "7z"], unique ["gg"]),
    plat!("sega_genesis", "Sega Genesis", "Genesis", igdb Some(29), db ["Sega - Mega Drive - Genesis"],
        lr Some("Sega - Mega Drive - Genesis"), tgdb Some(18), es ["genesis", "megadrive", "megadrivejp"],
        folders ["genesis", "megadrive", "megadrivejp", "md", "mega drive", "sega genesis", "sega mega drive"],
        ext ["md", "gen", "smd", "bin", "68k", "sgd", "zip", "7z"], unique ["md", "gen", "smd", "68k", "sgd"]),
    plat!("sega_mastersystem", "Sega Master System", "Master System", igdb Some(64), db ["Sega - Master System - Mark III"],
        lr Some("Sega - Master System - Mark III"), tgdb Some(35), es ["mastersystem", "mark3"],
        folders ["mastersystem", "mark3", "sms", "master system", "sega master system"],
        ext ["sms", "bin", "zip", "7z"], unique ["sms"]),
    plat!("sega_saturn", "Sega Saturn", "Saturn", igdb Some(32), db ["Sega - Saturn"], lr Some("Sega - Saturn"),
        tgdb Some(17), es ["saturn", "saturnjp"], folders ["saturn", "saturnjp", "ss", "sega saturn"],
        ext ["chd", "cue", "ccd", "mds", "toc", "iso", "m3u", "zip", "7z"], unique []),
    plat!("sega_sg1000", "Sega SG-1000", "SG-1000", igdb Some(84), db ["Sega - SG-1000"], lr Some("Sega - SG-1000"),
        tgdb Some(4949), es ["sg-1000"], folders ["sg-1000", "sg1000", "sega sg1000"],
        ext ["sg", "bin", "zip", "7z"], unique ["sg"]),
    plat!("sharp_x68000", "Sharp X68000", "X68000", igdb Some(121), db ["Sharp - X68000"], lr Some("Sharp - X68000"),
        tgdb Some(4931), es ["x68000"], folders ["x68000", "x68k", "sharp x68000"],
        ext ["dim", "d88", "88d", "hdm", "dup", "2hd", "xdf", "hdf", "img", "m3u", "zip", "7z"], unique []),
    plat!("sinclair_zxspectrum3", "Sinclair ZX Spectrum +3", "ZX Spectrum +3", igdb None,
        db ["Sinclair - ZX Spectrum +3"], lr Some("Sinclair - ZX Spectrum"), tgdb Some(4913), es [],
        folders ["zxspectrum3", "zx spectrum +3", "spectrum3"], ext ["dsk", "tzx", "tap", "z80", "rzx", "scl", "trd", "zip", "7z"],
        unique []),
    plat!("sinclair_zxspectrum", "Sinclair ZX Spectrum", "ZX Spectrum", igdb Some(26), db ["Sinclair - ZX Spectrum"],
        lr Some("Sinclair - ZX Spectrum"), tgdb Some(4913), es ["zxspectrum"],
        folders ["zxspectrum", "zx", "speccy", "spectrum", "zx spectrum"],
        ext ["tzx", "tap", "z80", "rzx", "scl", "trd", "pzx", "dsk", "zip", "7z"], unique ["tzx", "z80", "rzx", "scl", "trd", "pzx"]),
    plat!("sinclair_zx81", "Sinclair ZX81", "ZX81", igdb Some(373), db ["Sinclair - ZX 81"], lr Some("Sinclair - ZX 81"),
        tgdb Some(5010), es ["zx81"], folders ["zx81", "zx 81"], ext ["p", "t81", "tzx", "zip", "7z"],
        unique ["t81"]),
    plat!("snk_neogeo_aes", "SNK Neo Geo AES", "Neo Geo", igdb Some(80), db [], lr Some("SNK - Neo Geo"),
        tgdb Some(24), es ["neogeo"], folders ["neogeo", "neo geo", "mvs", "aes"], ext ["zip", "7z", "neo"],
        unique ["neo"]),
    plat!("snk_neogeo_cd", "SNK Neo Geo CD", "Neo Geo CD", igdb Some(136), db ["SNK - Neo Geo CD"],
        lr Some("SNK - Neo Geo CD"), tgdb Some(4956), es ["neogeocd", "neogeocdjp"],
        folders ["neogeocd", "neogeocdjp", "ngcd", "neo geo cd"], ext ["chd", "cue", "zip", "7z"], unique []),
    plat!("snk_neogeopocket_color", "SNK Neo Geo Pocket Color", "NGPC", igdb Some(120), db ["SNK - Neo Geo Pocket Color"],
        lr Some("SNK - Neo Geo Pocket Color"), tgdb Some(4923), es ["ngpc"],
        folders ["ngpc", "neogeopocketcolor", "neo geo pocket color"], ext ["ngc", "ngpc", "npc", "zip", "7z"],
        unique ["ngc", "ngpc", "npc"]),
    plat!("snk_neogeopocket", "SNK Neo Geo Pocket", "NGP", igdb Some(119), db ["SNK - Neo Geo Pocket"],
        lr Some("SNK - Neo Geo Pocket"), tgdb Some(4922), es ["ngp"],
        folders ["ngp", "neogeopocket", "neo geo pocket"], ext ["ngp", "zip", "7z"], unique ["ngp"]),
    plat!("sony_playstation2", "Sony PlayStation 2", "PS2", igdb Some(8), db ["Sony - PlayStation 2"],
        lr Some("Sony - PlayStation 2"), tgdb Some(11), es ["ps2"],
        folders ["ps2", "playstation2", "playstation 2", "sony playstation 2"],
        ext ["chd", "iso", "cso", "zso", "gz", "cue", "bin", "mdf", "nrg", "img", "isz", "elf", "m3u"], unique ["zso"]),
    plat!("sony_playstation3", "Sony PlayStation 3", "PS3", igdb Some(9),
        db ["Sony - PlayStation 3", "Sony - PlayStation 3 (PSN)"], lr Some("Sony - PlayStation 3"),
        tgdb Some(12), es ["ps3"], folders ["ps3", "playstation3", "playstation 3", "sony playstation 3"],
        ext ["iso"], unique [], folder_games),
    plat!("sony_playstation4", "Sony PlayStation 4", "PS4", igdb Some(48), db [], lr Some("Sony - PlayStation 4"),
        tgdb Some(4919), es ["ps4"], folders ["ps4", "playstation4", "playstation 4"], ext ["bin"],
        unique [], folder_games),
    plat!("sony_playstation5", "Sony PlayStation 5", "PS5", igdb Some(167), db [], lr None,
        tgdb Some(4980), es [], folders ["ps5", "playstation5", "playstation 5"], ext [], unique []),
    plat!("sony_psp", "Sony PlayStation Portable", "PSP", igdb Some(38),
        db ["Sony - PlayStation Portable", "Sony - PlayStation Portable (PSN)", "Sony - PlayStation Portable (PSX2PSP)"],
        lr Some("Sony - PlayStation Portable"), tgdb Some(13), es ["psp"],
        folders ["psp", "playstation portable", "sony psp"], ext ["iso", "cso", "chd", "pbp", "elf"],
        unique []),
    plat!("sony_vita", "Sony PlayStation Vita", "Vita", igdb Some(46),
        db ["Sony - PlayStation Vita", "Sony - PlayStation Vita (PSN)"], lr Some("Sony - PlayStation Vita"),
        tgdb Some(39), es ["psvita"], folders ["psvita", "vita", "psv", "playstation vita"],
        ext ["vpk", "psvita", "zip"], unique ["vpk", "psvita"]),
    plat!("sony_playstation", "Sony PlayStation", "PS1", igdb Some(7), db ["Sony - PlayStation"],
        lr Some("Sony - PlayStation"), tgdb Some(10), es ["psx"],
        folders ["psx", "ps1", "ps", "playstation", "playstation1", "playstation 1", "sony playstation"],
        ext ["m3u", "cue", "chd", "pbp", "ccd", "toc", "mds", "ecm", "iso", "img", "bin", "cbn", "exe", "psexe", "psf", "minipsf"],
        unique ["psexe", "minipsf", "cbn"]),
    plat!("ti_83", "Texas Instruments TI-83", "TI-83", igdb None, db [], lr None, tgdb None, es [],
        folders ["ti83", "ti 83"], ext ["83g", "83l", "83p", "zip", "7z"], unique ["83g", "83l", "83p"]),
    plat!("thomson_mo5", "Thomson MO5", "MO5", igdb Some(156), db ["Thomson - MOTO"], lr Some("Thomson - MOTO"),
        tgdb Some(5022), es ["moto", "to8"], folders ["moto", "mo5", "thomson", "thomson mo5"],
        ext ["fd", "k7", "m5", "m7", "rom", "sap", "zip", "7z"], unique ["k7", "m5", "m7"]),
    plat!("thomson_to7", "Thomson TO7", "TO7", igdb None, db ["Thomson - MOTO"], lr Some("Thomson - MOTO"),
        tgdb Some(5023), es ["moto"], folders ["to7", "to8", "thomson to7"],
        ext ["fd", "k7", "m5", "m7", "rom", "sap", "zip", "7z"], unique []),
    plat!("tic_80", "TIC-80", "TIC-80", igdb None, db [], lr Some("TIC-80"), tgdb None, es ["tic80"],
        folders ["tic80", "tic-80"], ext ["tic", "zip", "7z"], unique ["tic"]),
    plat!("uzebox", "Uzebox", "Uzebox", igdb None, db ["Uzebox"], lr None, tgdb None, es ["uzebox"],
        folders ["uzebox"], ext ["uze", "zip"], unique ["uze"]),
    plat!("watara_supervision", "Watara Supervision", "Supervision", igdb None, db ["Watara - Supervision"],
        lr Some("Watara - Supervision"), tgdb Some(4959), es ["supervision"],
        folders ["supervision", "watara supervision"], ext ["sv", "bin", "zip", "7z"], unique ["sv"]),
    // Lodestar-only: Playnite has no arcade platform.
    plat!("arcade", "Arcade", "Arcade", igdb Some(52), db ["FBNeo - Arcade Games"], lr Some("MAME"),
        tgdb Some(23), es ["arcade", "mame", "fbneo", "fba", "cps1", "cps2", "cps3", "naomi", "atomiswave"],
        folders ["arcade", "mame", "fbneo", "fba", "finalburn", "cps1", "cps2", "cps3", "naomi", "naomi2",
            "atomiswave", "model3", "mame2003", "mameplus", "daphne"],
        ext ["zip", "7z", "chd"], unique []),
];

/// Every known platform.
pub fn all() -> &'static [Platform] {
    PLATFORMS
}

pub fn by_id(id: &str) -> Option<&'static Platform> {
    index().ids.get(id).copied()
}

/// Lookup key: lower-case ASCII letters and digits only ("Sony PlayStation 2" → "sonyplaystation2"),
/// written into `buf` (no allocation for names up to 128 bytes); `None` when the name is empty
/// or too long.
fn key<'a>(s: &str, buf: &'a mut [u8; 128]) -> Option<&'a str> {
    let mut n = 0;
    for c in s.chars() {
        let c = match c {
            '+' => 'p', // "ZX Spectrum +3" ≠ "ZX Spectrum 3"? keep them apart.
            c if c.is_ascii_alphanumeric() => c.to_ascii_lowercase(),
            c if c.is_alphanumeric() => return None, // non-ASCII names: no alias matches
            _ => continue,
        };
        if n == buf.len() {
            return None;
        }
        buf[n] = c as u8;
        n += 1;
    }
    (n > 0).then(|| std::str::from_utf8(&buf[..n]).unwrap_or(""))
}

fn owned_key(s: &str) -> Option<String> {
    let mut buf = [0u8; 128];
    key(s, &mut buf).map(str::to_string)
}

struct Index {
    ids: HashMap<&'static str, &'static Platform>,
    /// Display names, short names, ids, extra names (for `by_name`).
    names: HashMap<String, &'static Platform>,
    /// Folder aliases + ES names + everything in `names` (for `by_folder_name`).
    folders: HashMap<String, &'static Platform>,
    unique: HashMap<&'static str, &'static Platform>,
}

/// Extra display names other sources / metadata use for a platform (matched by `by_name`).
const EXTRA_NAMES: &[(&str, &str)] = &[
    ("sony_playstation", "PlayStation"),
    ("sony_playstation", "PSX"),
    ("sony_playstation", "PlayStation 1"),
    ("sony_playstation2", "PlayStation 2"),
    ("sony_playstation3", "PlayStation 3"),
    ("sony_playstation4", "PlayStation 4"),
    ("sony_playstation5", "PlayStation 5"),
    ("sony_psp", "PlayStation Portable"),
    ("sony_vita", "PlayStation Vita"),
    ("sony_vita", "PS Vita"),
    ("nintendo_super_nes", "Super Nintendo Entertainment System"),
    ("nintendo_super_nes", "Super Nintendo"),
    ("nintendo_super_nes", "Super Famicom"),
    ("nintendo_nes", "Famicom"),
    ("nintendo_64", "Nintendo64"),
    ("nintendo_gamecube", "GameCube"),
    ("nintendo_gamecube", "GC"),
    ("nintendo_wii", "Wii"),
    ("nintendo_wiiu", "Wii U"),
    ("nintendo_switch", "Switch"),
    ("nintendo_ds", "NDS"),
    ("nintendo_3ds", "New Nintendo 3DS"),
    ("nintendo_gameboy", "Game Boy"),
    ("nintendo_gameboy", "GB"),
    ("nintendo_gameboycolor", "Game Boy Color"),
    ("nintendo_gameboyadvance", "Game Boy Advance"),
    ("sega_genesis", "Sega Mega Drive"),
    ("sega_genesis", "Mega Drive"),
    ("sega_genesis", "Sega Mega Drive/Genesis"),
    ("sega_genesis", "Sega Genesis/Mega Drive"),
    ("sega_genesis", "Mega Drive/Genesis"),
    ("sega_cd", "Sega Mega-CD"),
    ("sega_cd", "Mega-CD"),
    ("sega_dreamcast", "Dreamcast"),
    ("sega_saturn", "Saturn"),
    ("nec_turbografx_16", "PC Engine"),
    ("nec_turbografx_16", "TurboGrafx-16"),
    ("nec_turbografx_16", "TurboGrafx-16/PC Engine"),
    ("nec_turbografx_cd", "PC Engine CD"),
    ("nec_turbografx_cd", "Turbografx-16/PC Engine CD"),
    ("xbox", "Xbox"),
    ("xbox360", "Xbox 360"),
    ("snk_neogeo_aes", "Neo Geo"),
    ("snk_neogeo_aes", "Neo Geo AES"),
    ("snk_neogeo_aes", "Neo Geo MVS"),
    ("pc_dos", "DOS"),
    ("pc_dos", "MS-DOS"),
    ("pc_windows", "Windows"),
    ("pc_windows", "PC"),
    ("pc_linux", "Linux"),
    ("vectrex", "Vectrex"),
    ("atari_st", "Atari ST"),
    ("commodore_vci20", "Commodore VIC-20"),
    ("commodore_vci20", "VIC-20"),
    ("arcade", "MAME"),
];

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut ids = HashMap::new();
        let mut names: HashMap<String, &'static Platform> = HashMap::new();
        let mut folders: HashMap<String, &'static Platform> = HashMap::new();
        let mut unique = HashMap::new();
        let add = |map: &mut HashMap<String, &'static Platform>, s: &str, p: &'static Platform| {
            if let Some(k) = owned_key(s) {
                map.entry(k).or_insert(p);
            }
        };
        for p in PLATFORMS {
            ids.insert(p.id, p);
            for u in p.unique_ext {
                unique.entry(*u).or_insert(p);
            }
        }
        // Pass 1: exact names (most authoritative), pass 2: short names, then extras.
        for p in PLATFORMS {
            add(&mut names, p.name, p);
            add(&mut names, p.id, p);
        }
        for p in PLATFORMS {
            add(&mut names, p.short, p);
        }
        for (id, n) in EXTRA_NAMES {
            if let Some(p) = ids.get(id) {
                add(&mut names, n, p);
            }
        }
        // Folders: aliases first (they are the folder names people really use), then names.
        for p in PLATFORMS {
            for f in p.folders {
                add(&mut folders, f, p);
            }
        }
        for p in PLATFORMS {
            for e in p.es {
                add(&mut folders, e, p);
            }
        }
        for (k, p) in &names {
            folders.entry(k.clone()).or_insert(p);
        }
        // Never: "pc" (Windows games), and the PC platforms are never ROM folders.
        folders.retain(|k, p| k != "pc" && !matches!(p.id, "pc_windows" | "pc_linux"));
        Index { ids, names, folders, unique }
    })
}

/// The platform a ROM folder's name stands for: "ps2", "PlayStation 2", "ps3games", "PS2 Games",
/// "Nintendo Switch ROMs", "My PSX collection" (collection words around the console name are
/// ignored: games, roms, isos, collection, backups, dumps, stuff …, and a leading "my" / "roms"
/// / "games"). "pc" is never an emulated platform (it usually holds Windows games).
pub fn by_folder_name(name: &str) -> Option<&'static Platform> {
    let mut buf = [0u8; 128];
    let k = key(name, &mut buf)?;
    if k == "pc" {
        return None;
    }
    let idx = index();
    if let Some(p) = idx.folders.get(k) {
        return Some(p);
    }
    const SUFFIXES: &[&str] = &[
        "games",
        "game",
        "roms",
        "rom",
        "isos",
        "iso",
        "collection",
        "library",
        "backups",
        "backup",
        "dumps",
        "dump",
        "stuff",
        "files",
    ];
    const PREFIXES: &[&str] = &["my", "roms", "games"];
    let lookup = |stem: &str| (!stem.is_empty() && stem != "pc").then(|| idx.folders.get(stem).copied()).flatten();
    let strip_suffixes = |mut k: &str| -> Option<&'static Platform> {
        // Up to two words ("ps2 iso backups").
        for _ in 0..2 {
            let s = SUFFIXES.iter().find_map(|x| k.strip_suffix(x).filter(|s| !s.is_empty()))?;
            if let Some(p) = lookup(s) {
                return Some(p);
            }
            k = s;
        }
        None
    };
    if let Some(p) = strip_suffixes(k) {
        return Some(p);
    }
    for pre in PREFIXES {
        if let Some(rest) = k.strip_prefix(pre).filter(|r| r.len() >= 2)
            && let Some(p) = lookup(rest).or_else(|| strip_suffixes(rest))
        {
            return Some(p);
        }
    }
    None
}

/// A game's platform as people read it: "windows" → "Windows" (run through Proton / Wine),
/// "linux" → "Linux"; a console under any of its names → its full name ("PS2",
/// "PlayStation 2" → "Sony PlayStation 2"), so each platform reads one way everywhere;
/// others as-is.
pub fn display_name(p: &str) -> &str {
    match p.trim() {
        x if x.eq_ignore_ascii_case("linux") => "Linux",
        x if x.eq_ignore_ascii_case("windows") => "Windows",
        x if x.eq_ignore_ascii_case("mac") || x.eq_ignore_ascii_case("osx") => "macOS",
        x if x.eq_ignore_ascii_case("browser") => "Browser",
        x if x.eq_ignore_ascii_case(crate::cloud::PLATFORM) => "Cloud",
        x => by_name(x).filter(|pl| !pl.id.starts_with("pc_")).map_or(x, |pl| pl.name),
    }
}

/// A platform by its display name or a short name / alias ("Sony PlayStation 2", "PS2").
/// Cheap (one hash lookup, no allocation): library collections call it per game per frame.
pub fn by_name(name: &str) -> Option<&'static Platform> {
    if let Some(p) = index().ids.get(name) {
        return Some(p);
    }
    let mut buf = [0u8; 128];
    let k = key(name, &mut buf)?;
    index().names.get(k).copied()
}

/// The platform an extension alone identifies ("z64" → Nintendo 64).
pub fn by_unique_extension(ext: &str) -> Option<&'static Platform> {
    let e = ext.trim_start_matches('.');
    let mut buf = [0u8; 16];
    if e.is_empty() || e.len() > buf.len() || !e.is_ascii() {
        return None;
    }
    for (i, b) in e.bytes().enumerate() {
        buf[i] = b.to_ascii_lowercase();
    }
    let e = std::str::from_utf8(&buf[..e.len()]).ok()?;
    index().unique.get(e).copied()
}

/// Does `ext` (lower- or upper-case, with or without the dot) belong to the platform?
#[cfg(test)]
pub fn has_extension(p: &Platform, ext: &str) -> bool {
    let e = ext.trim_start_matches('.');
    p.extensions.iter().any(|x| x.eq_ignore_ascii_case(e))
}

#[cfg(test)]
mod tests;
