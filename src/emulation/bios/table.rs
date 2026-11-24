//! The BIOS / firmware table (libretro System.dat, DuckStation, Batocera and the emulators'
//! sources; see research-platforms-bios.md §2b). An empty `md5` list means "known by its name"
//! (dumps differ per version: keys, firmware packages, arcade BIOS sets); PS2 BIOS files have no
//! fixed name or checksum and are recognised by their ROMVER (`file: ""`).

use super::BiosFile;

macro_rules! b {
    ($p:literal, $f:literal, [$($m:literal),*], req, $g:expr, $r:expr, $n:literal) => {
        BiosFile { platform: $p, file: $f, md5: &[$($m),*], required: true, group: $g, region: $r, note: $n }
    };
    ($p:literal, $f:literal, [$($m:literal),*], opt, $r:expr, $n:literal) => {
        BiosFile { platform: $p, file: $f, md5: &[$($m),*], required: false, group: None, region: $r, note: $n }
    };
}

const JP: Option<&str> = Some("Japan");
const US: Option<&str> = Some("USA");
const EU: Option<&str> = Some("Europe");
const PSX: Option<&str> = Some("psx");

pub(super) static BIOS: &[BiosFile] = &[
    // ---- Sony PlayStation: any one is enough; DuckStation picks by region ----
    b!("sony_playstation", "scph1000.bin", ["239665b1a3dade1b5a52c06338011044"], req, PSX, JP, ""),
    b!("sony_playstation", "scph3000.bin", ["849515939161e62f6b866f6853006780"], req, PSX, JP, ""),
    b!("sony_playstation", "scph3500.bin", ["cba733ceeff5aef5c32254f1d617fa62"], req, PSX, JP, ""),
    b!(
        "sony_playstation",
        "scph5000.bin",
        ["57a06303dfa9cf9351222dfcbb4a29d9", "eb201d2d98251a598af467d4347bb62f"],
        req,
        PSX,
        JP,
        ""
    ),
    b!("sony_playstation", "scph5500.bin", ["8dd7d5296a650fac7319bce665a6a53c"], req, PSX, JP, ""),
    b!("sony_playstation", "scph7000.bin", ["8e4c14f567745eff2f0408c8129f72a6"], req, PSX, JP, ""),
    b!("sony_playstation", "scph100.bin", ["8abc1b549a4a80954addc48ef02c4521"], req, PSX, JP, ""),
    b!("sony_playstation", "scph1000r.bin", ["476d68a94ccec3b9c8303bbd1daf2810"], req, PSX, JP, ""),
    b!(
        "sony_playstation",
        "scph1001.bin",
        ["924e392ed05558ffdb115408c263dccf", "dc2b9bf8da62ec93e868cfd29f0d067d", "da27e8b6dab242d8f91a9b25d80c63b8"],
        req,
        PSX,
        US,
        ""
    ),
    b!("sony_playstation", "scph5501.bin", ["490f666e1afb15b7362b406ed1cea246"], req, PSX, US, ""),
    b!("sony_playstation", "scph7001.bin", ["1e68c231d0896b7eadcad1d7d8e76129"], req, PSX, US, ""),
    b!(
        "sony_playstation",
        "scph101.bin",
        ["6e3735ff4c7dc899ee98981385f6f3d0", "9a09ab7e49b422c007e6d54d7c49b965"],
        req,
        PSX,
        US,
        ""
    ),
    b!(
        "sony_playstation",
        "scph1002.bin",
        ["54847e693405ffeb0359c6287434cbef", "417b34706319da7cf001e76e40136c23", "e2110b8a2b97a8e0b857a45d32f7e187"],
        req,
        PSX,
        EU,
        ""
    ),
    b!("sony_playstation", "scph5502.bin", ["32736f17079d0b2b7024407c39bd3050"], req, PSX, EU, ""),
    b!("sony_playstation", "scph7002.bin", ["b9d9a0286c33dc6b7237bb13cd46fdee"], req, PSX, EU, ""),
    b!(
        "sony_playstation",
        "scph102.bin",
        ["de93caec13d1a141a40a79f5c86168d6", "b10f5e0e3d9eb60e5159690680b1e774"],
        req,
        PSX,
        EU,
        ""
    ),
    b!(
        "sony_playstation",
        "psxonpsp660.bin",
        ["c53ca5908936d412331790f4426c6c33"],
        opt,
        None,
        "Region-free (PCSX ReARMed)"
    ),
    b!("sony_playstation", "ps1_rom.bin", ["81bbe60ba7a3d1cea1d48c14cbcc647b"], opt, None, "From a PS3"),
    // ---- Sony PlayStation 2: any BIOS dump, recognised by its ROMVER ----
    b!("sony_playstation2", "ps2-0230a-20080220.bin", ["21038400dc633070a78ad53090c53017"], req, Some("ps2"), US, ""),
    b!("sony_playstation2", "", [], req, Some("ps2"), JP, "PS2 BIOS (Japan)"),
    b!("sony_playstation2", "", [], req, Some("ps2"), US, "PS2 BIOS (USA)"),
    b!("sony_playstation2", "", [], req, Some("ps2"), EU, "PS2 BIOS (Europe)"),
    b!("sony_playstation2", "", [], req, Some("ps2"), Some("Asia"), "PS2 BIOS (Asia)"),
    b!("sony_playstation2", "", [], req, Some("ps2"), None, "PS2 BIOS"),
    // ---- Sony PlayStation 3 / Vita: firmware, installed through the emulator ----
    b!(
        "sony_playstation3",
        "PS3UPDAT.PUP",
        [],
        req,
        None,
        None,
        "PS3 firmware: install it in RPCS3 (File → Install Firmware)"
    ),
    b!("sony_vita", "PSVUPDAT.PUP", [], req, None, None, "Vita firmware: install it in Vita3K"),
    b!("sony_vita", "PSP2UPDAT.PUP", [], opt, None, "Vita font package: install it in Vita3K"),
    // ---- Nintendo ----
    b!("nintendo_switch", "prod.keys", [], req, None, None, "Switch keys from your console"),
    b!("nintendo_switch", "title.keys", [], opt, None, "Switch title keys from your console"),
    b!(
        "nintendo_switch",
        "firmware",
        [],
        req,
        None,
        None,
        "Switch firmware: install it in the emulator (Tools → Install Firmware)"
    ),
    b!("nintendo_wiiu", "keys.txt", [], opt, None, "Only needed for .wud / .wux discs"),
    b!(
        "nintendo_gamecube",
        "GC/USA/IPL.bin",
        ["b17148254a5799684c7d783206504926", "019e39822a9ca3029124f74dd4d55ac4", "fc924a7c879b661abc37cec4f018fdf3"],
        opt,
        US,
        "GameCube menu (optional)"
    ),
    b!(
        "nintendo_gamecube",
        "GC/EUR/IPL.bin",
        ["db92574caab77a7ec99d4605fd6f2450", "339848a0b7c2124cf155276c1e79cbd0", "0cdda509e2da83c85bfe423dd87346cc"],
        opt,
        EU,
        "GameCube menu (optional)"
    ),
    b!("nintendo_64", "64DD_IPL.bin", ["8d3d9f294b6e174bc7b1d2fd1c727530"], opt, JP, "Only for 64DD games"),
    b!("nintendo_ds", "bios7.bin", ["df692a80a5b1bc90728bc3dfc76cd948"], opt, None, ""),
    b!("nintendo_ds", "bios9.bin", ["a392174eb3e572fed6447e956bde4b25"], opt, None, ""),
    b!("nintendo_ds", "firmware.bin", [], opt, None, ""),
    b!("nintendo_dsi", "dsi_bios7.bin", ["559dae4ea78eb9d67702c56c1d791e81"], req, None, None, ""),
    b!("nintendo_dsi", "dsi_bios9.bin", ["87b665fce118f76251271c3732532777"], req, None, None, ""),
    b!("nintendo_dsi", "dsi_firmware.bin", [], req, None, None, ""),
    b!("nintendo_dsi", "dsi_nand.bin", [], req, None, None, ""),
    b!("nintendo_gameboyadvance", "gba_bios.bin", ["a860e8c0b6d573d191e4ec7db1b1e4f6"], opt, None, ""),
    b!("nintendo_gameboy", "gb_bios.bin", ["32fbbd84168d3482956eb3c5051637f5"], opt, None, ""),
    b!("nintendo_gameboycolor", "gbc_bios.bin", ["dbfce9db9deaa2567f6a84fde55f9680"], opt, None, ""),
    b!("nintendo_super_nes", "sgb_bios.bin", ["d574d4f9c12f305074798f54c091a8b4"], opt, None, "Super Game Boy"),
    b!("nintendo_super_nes", "SGB1.sfc", ["b15ddb15721c657d82c5bab6db982ee9"], opt, None, "Super Game Boy"),
    b!("nintendo_super_nes", "SGB2.sfc", ["8ecd73eb4edf7ed7e81aef1be80031d5"], opt, None, "Super Game Boy 2"),
    b!("nintendo_famicom_disk", "disksys.rom", ["ca30b50f880eb660a320674ed365ef7a"], req, None, JP, ""),
    // ---- Sega ----
    b!(
        "sega_cd",
        "bios_CD_U.bin",
        ["2efd74e3232ff260e371b99f84024f7f", "854b9150240a198070150e4566ae1290"],
        req,
        Some("segacd"),
        US,
        ""
    ),
    b!("sega_cd", "bios_CD_E.bin", ["e66fa1dc5820d254611fdcdba0662372"], req, Some("segacd"), EU, ""),
    b!("sega_cd", "bios_CD_J.bin", ["278a9397d192149e84e820ac621a8edd"], req, Some("segacd"), JP, ""),
    b!(
        "sega_genesis",
        "bios_MD.bin",
        ["45e298905a08f9cfb38fd504cd6dbc84", "d3293ebaaa7f4eb2a6766b68a0fb4609"],
        opt,
        None,
        ""
    ),
    b!("sega_mastersystem", "bios_E.sms", ["840481177270d5642a14ca71ee72844c"], opt, EU, ""),
    b!("sega_mastersystem", "bios_J.sms", ["24a519c53f67b00640d0048ef7089105"], opt, JP, ""),
    b!("sega_gamegear", "bios.gg", ["672e104c3be3a238301aceffc3b23fd6"], opt, None, ""),
    b!("sega_saturn", "sega_101.bin", ["85ec9ca47d8f6807718151cbcca8b964"], req, Some("saturn"), JP, ""),
    b!(
        "sega_saturn",
        "mpr-17933.bin",
        ["3240872c70984b6cbfda1586cab68dbe"],
        req,
        Some("saturn"),
        Some("USA / Europe"),
        ""
    ),
    b!("sega_saturn", "saturn_bios.bin", ["af5828fdff51384f99b3c4926be27762"], opt, None, "Kronos / Yabause"),
    b!("sega_saturn", "mpr-18811-mx.ic1", ["255113ba943c92a54facd25a10fd780c"], opt, None, "The King of Fighters '95"),
    b!("sega_saturn", "mpr-19367-mx.ic1", ["1cd19988d1d72a3e7caa0b73234c96b4"], opt, None, "Ultraman"),
    b!("sega_dreamcast", "dc/dc_boot.bin", ["e10c53c2f8b90bab96ead2d368858623"], opt, None, "Flycast runs without it"),
    b!("sega_dreamcast", "dc/dc_flash.bin", ["0a93f7940c455905bea6e392dfde92a4"], opt, None, ""),
    b!("arcade", "dc/naomi.zip", [], opt, None, "Sega Naomi games"),
    b!("arcade", "dc/naomi2.zip", [], opt, None, "Sega Naomi 2 games"),
    b!("arcade", "dc/awbios.zip", [], opt, None, "Atomiswave games"),
    b!("arcade", "neogeo.zip", [], opt, None, "Neo Geo games"),
    // ---- NEC ----
    b!("nec_turbografx_cd", "syscard3.pce", ["38179df8f4ac870017db21ebcbf53114"], req, None, None, ""),
    b!("nec_turbografx_cd", "syscard3u.pce", ["0754f903b52e3b3342202bdafb13efa5"], opt, US, ""),
    b!("nec_turbografx_cd", "syscard1.pce", ["2b7ccb3d86baa18f6402c176f3065082"], opt, JP, ""),
    b!("nec_turbografx_cd", "syscard2.pce", ["3cdd6614a918616bfc41c862e889dd79"], opt, JP, ""),
    b!("nec_turbografx_cd", "gexpress.pce", ["6d2cb14fc3e1f65ceb135633d1694122"], opt, JP, ""),
    b!("nec_pcfx", "pcfx.rom", ["08e36edbea28a017f79f8d4f7ff9b6d7"], req, None, JP, ""),
    b!("nec_pc98", "np2kai/BIOS.ROM", ["e246140dec5124c5e404869a84caefce"], req, None, JP, ""),
    b!("nec_pc98", "np2kai/FONT.ROM", ["2af6179d7de4893ea0b705c00e9a98d6"], req, None, JP, ""),
    b!("nec_pc98", "np2kai/SOUND.ROM", ["caf90f22197aed6f14c471c21e64658d"], req, None, JP, ""),
    b!("nec_pc98", "np2kai/ITF.ROM", ["e9fc3890963b12cf15d0a2eea5815b72"], req, None, JP, ""),
    // ---- SNK ----
    b!("snk_neogeo_aes", "neogeo.zip", [], req, None, None, "Neo Geo BIOS set (matching your ROM set version)"),
    b!("snk_neogeo_cd", "neocd/neocd_z.rom", ["11526d58d4c524daef7d5d677dc6b004"], opt, None, "NeoCD runs without it"),
    b!("snk_neogeo_cd", "neocd/neocd.bin", ["f39572af7584cb5b3f70ae8cc848aba2"], opt, None, ""),
    // ---- 3DO / Atari / others ----
    b!("3do", "panafz10.bin", ["51f2f43ae2f3508a14d9f56597e2d3ce"], req, Some("3do"), None, "Recommended"),
    b!("3do", "panafz1.bin", ["f47264dd47fe30f73ab3c010015c155b"], req, Some("3do"), None, ""),
    b!("3do", "panafz1j.bin", ["a496cfdded3da562759be3561317b605"], req, Some("3do"), JP, ""),
    b!("3do", "panafz10-norsa.bin", ["1477bda80dc33731a65468c1f5bcbee9"], req, Some("3do"), None, ""),
    b!("3do", "panafz10e-anvil.bin", ["a48e6746bd7edec0f40cff078f0bb19f"], req, Some("3do"), EU, ""),
    b!("3do", "goldstar.bin", ["8639fd5e549bd6238cfee79e3e749114"], req, Some("3do"), None, ""),
    b!("3do", "sanyotry.bin", ["35fa1a1ebaaeea286dc5cd15487c13ea"], req, Some("3do"), None, ""),
    b!("atari_lynx", "lynxboot.img", ["fcd403db69f54290b51035d82f835e7b"], req, None, None, ""),
    b!("atari_7800", "7800 BIOS (U).rom", ["0763f1ffb006ddbe32e52d497ee848ae"], opt, US, ""),
    b!("atari_7800", "7800 BIOS (E).rom", ["397bb566584be7b9764e7a68974c4263"], opt, EU, ""),
    b!("atari_5200", "5200.rom", ["281f20ea4320404ec820fb7ec0693b38"], opt, None, ""),
    b!("atari_8bit", "ATARIXL.ROM", ["06daac977823773a3eea3422fd26a703"], opt, None, ""),
    b!("atari_8bit", "ATARIBAS.ROM", ["0bac0c6a50104045d902df4503a4c30b"], opt, None, ""),
    b!("atari_8bit", "ATARIOSA.ROM", ["eb1f32f5d9f382db1bbfb8d7f9cb343a"], opt, None, ""),
    b!("atari_8bit", "ATARIOSB.ROM", ["a3e8d617c95d08031fe1b20d541434b2"], opt, None, ""),
    b!("atari_st", "tos.img", ["c1c57ce48e8ee4135885cee9e63a68a2"], req, None, None, "TOS (Atari ST system ROM)"),
    b!("mattel_intellivision", "exec.bin", ["62e761035cb657903761800f4437b8af"], req, None, None, ""),
    b!("mattel_intellivision", "grom.bin", ["0cd5946c6473e42e8e4c2137785e427f"], req, None, None, ""),
    b!("coleco_vision", "colecovision.rom", ["2c66f5911e5b42b8ebe113403548eee7"], req, None, None, ""),
    b!("magnavox_odyssey_2", "o2rom.bin", ["562d5ebf9e030a40d6fabfc2f33139fd"], req, None, None, ""),
    b!(
        "microsoft_msx",
        "MSX.ROM",
        ["364a1a579fe5cb8dba54519bcfcdac0d", "aa95aea2563cd5ec0a0919b44cc17d47"],
        opt,
        None,
        "fMSX"
    ),
    b!("microsoft_msx2", "MSX2.ROM", ["ec3a01c91f24fbddcbcab0ad301bc9ef"], opt, None, "fMSX"),
    b!("microsoft_msx2", "MSX2EXT.ROM", ["2183c2aff17cf4297bdb496de78c2e8a"], opt, None, "fMSX"),
    b!("commodore_amiga", "kick34005.A500", ["82a21c1890cae844b3df741f2762d48d"], opt, None, "Kickstart 1.3"),
    b!("commodore_amiga", "kick37175.A500", ["dc10d7bdd1b6f450773dfb558477c230"], opt, None, "Kickstart 2.04"),
    b!("commodore_amiga", "kick40068.A1200", ["646773759326fbac3b2311fd8c8793ee"], opt, None, "Kickstart 3.1"),
    b!(
        "commodore_amiga_cd32",
        "kick40060.CD32",
        ["5f8924d013dd57a89cf349f4cdedc6b1", "f2f241bf094168cfb9e7805dc2856433"],
        req,
        None,
        None,
        ""
    ),
    b!("commodore_amiga_cd32", "kick40060.CD32.ext", ["bb72565701b1b6faece07d68ea5da639"], opt, None, ""),
    b!("sharp_x68000", "keropi/iplrom.dat", ["7fd4caabac1d9169e289f0f7bbf71d8e"], req, None, JP, ""),
    b!("sharp_x68000", "keropi/cgrom.dat", ["cb0a5cfcf7247a7eab74bb2716260269"], req, None, JP, ""),
    b!("xbox", "mcpx_1.0.bin", ["d49c52a4102f6df7bcf8d0617ac475ed"], req, None, None, "Xbox boot ROM"),
    b!("xbox", "Complex_4627.bin", ["39cee882148a87f93cb440b99dde3ceb"], req, None, None, "Xbox flash BIOS"),
    b!("xbox", "xbox_hdd.qcow2", [], req, None, None, "Xbox hard disk image"),
];
