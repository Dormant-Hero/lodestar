//! The extra artwork sources (pure: URLs, parsing, matching; fixtures in
//! `tests/fixtures/art/`). Endpoints as verified against the live services:
//!
//! * **Lutris.net** (no key): `GET https://lutris.net/api/games?search=<name>` →
//!   `{count, results: [{name, slug, year, coverart, banner_url, icon_url,
//!   provider_games: [{service: "steam", slug: "<appid>"}]}]}`. Cover 264×352 (IGDB copy),
//!   banner 184×69, icon 128×128.
//! * **GOG** (no key): `GET https://catalog.gog.com/v1/catalog?query=like:<name>&…` →
//!   `{products: [{id, title, coverVertical 342×482, coverHorizontal 1600×740, logo,
//!   galaxyBackgroundImage}]}`; by product id `GET https://api.gog.com/v2/games/<id>` →
//!   `_links.{boxArtImage, galaxyBackgroundImage, backgroundImage, logo, iconSquare, icon}`.
//! * **PCGamingWiki** (no key): its Cargo API refuses anonymous queries now ("You don't have
//!   permission to run arbitrary Cargo queries"), so the plain MediaWiki API is used: the page's
//!   infobox wikitext (`prop=revisions&rvsection=0`, `|cover = X.png`, `|steam appid = N`), then
//!   `prop=imageinfo` for the file's URL and a 300 px thumbnail. Pages are found by title (the
//!   wiki's redirects count) or `list=search`; a game with a Steam appid only matches a page
//!   whose infobox lists that appid.
//! * **libretro-thumbnails** (no key): thumbnails.libretro.com serves no files at the moment,
//!   so the GitHub mirror is used: names from
//!   `GET https://api.github.com/repos/libretro-thumbnails/<System_Name>/git/trees/master:Named_Boxarts`
//!   (one request per system, cached on disk for two weeks; GitHub allows 60 an hour), images
//!   from `https://raw.githubusercontent.com/libretro-thumbnails/<System_Name>/master/<kind>/<name>.png`.
//!   File names are No-Intro names ("Legend of Zelda, The - Ocarina of Time (USA)").
//! * **TheGamesDB** (API key): `GET https://api.thegamesdb.net/v1.1/Games/ByGameName?name=…&include=boxart`
//!   and `GET /v1/Games/Images?games_id=…&filter[type]=fanart,banner,clearlogo,boxart`; files are
//!   `base_url.original + filename` on cdn.thegamesdb.net. The key is added when sending.

use super::{ArtSlot, encode, same_name};
use serde_json::Value;

pub const SRC_GOG: &str = "GOG";
pub const SRC_IGDB: &str = "IGDB";
pub const SRC_TGDB: &str = "TheGamesDB";
pub const SRC_LIBRETRO: &str = "libretro";
pub const SRC_LUTRIS: &str = "Lutris";
pub const SRC_PCGW: &str = "PCGamingWiki";

pub const LUTRIS_API: &str = "https://lutris.net/api/games";
pub const GOG_CATALOG: &str = "https://catalog.gog.com/v1/catalog";
pub const GOG_GAMES: &str = "https://api.gog.com/v2/games";
pub const PCGW_API: &str = "https://www.pcgamingwiki.com/w/api.php";
pub const LIBRETRO_TREES: &str = "https://api.github.com/repos/libretro-thumbnails";
pub const LIBRETRO_RAW: &str = "https://raw.githubusercontent.com/libretro-thumbnails";
pub const TGDB_API: &str = "https://api.thegamesdb.net";

/// One picture a source offers for a slot (turned into a `Candidate` by the engine).
#[derive(Clone, Debug, PartialEq)]
pub struct Pic {
    pub url: String,
    pub thumb: Option<String>,
    pub width: u32,
    pub height: u32,
    /// Extra label text ("Europe", "fan art" …), may be empty.
    pub note: String,
}

impl Pic {
    pub fn new(url: impl Into<String>, w: u32, h: u32) -> Pic {
        Pic { url: url.into(), thumb: None, width: w, height: h, note: String::new() }
    }
    fn thumb(mut self, t: Option<String>) -> Pic {
        self.thumb = t;
        self
    }
    fn note(mut self, n: &str) -> Pic {
        self.note = n.to_string();
        self
    }
}

/// `https://…` (protocol-relative `//…` upgraded), else None.
fn https(u: &str) -> Option<String> {
    let u = u.trim();
    if u.starts_with("https://") {
        Some(u.to_string())
    } else {
        u.strip_prefix("//").map(|rest| format!("https://{rest}"))
    }
}

fn str_of<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("").trim()
}

/// Ids arrive as numbers or strings depending on the API.
fn id_of(v: &Value, k: &str) -> Option<u64> {
    match v.get(k)? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn dims(res: &str) -> (u32, u32) {
    res.split_once('x').and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?))).unwrap_or((0, 0))
}

// ---------------------------------------------------------------------------------------------
// Console platforms (libretro / TheGamesDB / IGDB ids)

/// A console platform the retro sources know.
#[derive(Debug, PartialEq, Eq)]
pub struct Platform {
    /// Playnite platform id ("nintendo_64"); `crate::emulation::platforms` uses the same ids.
    pub id: &'static str,
    /// Playnite display name ("Nintendo 64").
    pub name: &'static str,
    /// libretro-thumbnails system name ("Nintendo - Nintendo 64"); None = no thumbnail repo
    /// (Switch), so libretro is never asked.
    pub libretro: Option<&'static str>,
    /// TheGamesDB platform id.
    pub tgdb: Option<u32>,
    /// IGDB platform id.
    pub igdb: Option<u32>,
    /// Other spellings seen in libraries (Lutris, ES folder names, add-ons, hand edits).
    aliases: &'static [&'static str],
}

macro_rules! opt {
    (-) => {
        None
    };
    ($x:literal) => {
        Some($x)
    };
}

macro_rules! platforms {
    ($($id:literal, $name:literal, $lr:tt, $tg:tt, $ig:tt, [$($a:literal),*];)*) => {
        &[$(Platform { id: $id, name: $name, libretro: opt!($lr), tgdb: opt!($tg),
            igdb: opt!($ig), aliases: &[$($a),*] }),*]
    };
}

/// The consoles (Playnite names and ids; libretro-thumbnails repos; TheGamesDB ids
/// from ES-DE's and Skyscraper's maps, IGDB ids from Playnite, Vectrex corrected to 70). PCs (DOS,
/// Windows, Linux, Mac) are not consoles here: their games keep the PC stores as sources.
/// `-` = the service has no such system (or the id isn't known).
pub static PLATFORMS: &[Platform] = platforms![
    "nintendo_nes", "Nintendo Entertainment System", "Nintendo - Nintendo Entertainment System", 7, 18, ["nes", "nintendo nes", "famicom", "nintendo famicom", "fc"];
    "nintendo_famicom_disk", "Nintendo Family Computer Disk System", "Nintendo - Family Computer Disk System", 4936, 51, ["fds", "famicom disk system", "family computer disk system"];
    "nintendo_super_nes", "Nintendo SNES", "Nintendo - Super Nintendo Entertainment System", 6, 19, ["snes", "super nintendo", "super famicom", "super nintendo entertainment system", "sfc", "snesna"];
    "nintendo_64", "Nintendo 64", "Nintendo - Nintendo 64", 3, 4, ["n64"];
    "nintendo_gamecube", "Nintendo GameCube", "Nintendo - GameCube", 2, 21, ["gamecube", "gc", "ngc"];
    "nintendo_wii", "Nintendo Wii", "Nintendo - Wii", 9, 5, ["wii"];
    "nintendo_wiiu", "Nintendo Wii U", "Nintendo - Wii U", 38, 41, ["wiiu", "wii u"];
    "nintendo_switch", "Nintendo Switch", -, 4971, 130, ["switch", "ns", "nsw"];
    "nintendo_gameboy", "Nintendo Game Boy", "Nintendo - Game Boy", 4, 33, ["gb", "game boy", "gameboy"];
    "nintendo_gameboycolor", "Nintendo Game Boy Color", "Nintendo - Game Boy Color", 41, 22, ["gbc", "game boy color", "gameboy color"];
    "nintendo_gameboyadvance", "Nintendo Game Boy Advance", "Nintendo - Game Boy Advance", 5, 24, ["gba", "game boy advance", "gameboy advance"];
    "nintendo_virtualboy", "Nintendo Virtual Boy", "Nintendo - Virtual Boy", 4918, 87, ["virtualboy", "virtual boy", "vb"];
    "nintendo_ds", "Nintendo DS", "Nintendo - Nintendo DS", 8, 20, ["nds", "ds"];
    "nintendo_dsi", "Nintendo DSi", "Nintendo - Nintendo DSi", -, 159, ["dsi", "ndsi"];
    "nintendo_3ds", "Nintendo 3DS", "Nintendo - Nintendo 3DS", 4912, 37, ["3ds", "n3ds", "new nintendo 3ds", "new 3ds", "nintendo 3ds digital"];
    "sega_genesis", "Sega Genesis", "Sega - Mega Drive - Genesis", 18, 29, ["genesis", "megadrive", "mega drive", "sega mega drive", "md", "megadrivejp"];
    "sega_mastersystem", "Sega Master System", "Sega - Master System - Mark III", 35, 64, ["mastersystem", "master system", "sms", "mark3", "mark iii"];
    "sega_gamegear", "Sega Game Gear", "Sega - Game Gear", 20, 35, ["gamegear", "game gear", "gg"];
    "sega_saturn", "Sega Saturn", "Sega - Saturn", 17, 32, ["saturn", "saturnjp"];
    "sega_dreamcast", "Sega Dreamcast", "Sega - Dreamcast", 16, 23, ["dreamcast", "dc"];
    "sega_cd", "Sega CD", "Sega - Mega-CD - Sega CD", 21, 78, ["segacd", "megacd", "mega cd", "sega mega cd", "megacdjp"];
    "sega_32x", "Sega 32X", "Sega - 32X", 33, 30, ["32x", "sega32x", "sega32xjp", "sega32xna"];
    "sega_sg1000", "Sega SG-1000", "Sega - SG-1000", 4949, 84, ["sg1000", "sg 1000"];
    "sony_playstation", "Sony PlayStation", "Sony - PlayStation", 10, 7, ["psx", "ps1", "playstation", "playstation 1"];
    "sony_playstation2", "Sony PlayStation 2", "Sony - PlayStation 2", 11, 8, ["ps2", "playstation 2"];
    "sony_playstation3", "Sony PlayStation 3", "Sony - PlayStation 3", 12, 9, ["ps3", "playstation 3", "sony playstation 3 psn"];
    "sony_playstation4", "Sony PlayStation 4", "Sony - PlayStation 4", 4919, 48, ["ps4", "playstation 4"];
    "sony_psp", "Sony PlayStation Portable", "Sony - PlayStation Portable", 13, 38, ["psp", "playstation portable", "sony psp"];
    "sony_vita", "Sony PlayStation Vita", "Sony - PlayStation Vita", 39, 46, ["psvita", "vita", "psv", "ps vita", "playstation vita"];
    "xbox", "Microsoft Xbox", "Microsoft - Xbox", 14, 11, ["xbox"];
    "xbox360", "Microsoft Xbox 360", "Microsoft - Xbox 360", 15, 12, ["xbox360", "xbox 360", "x360"];
    "nec_turbografx_16", "NEC TurboGrafx 16", "NEC - PC Engine - TurboGrafx 16", 34, 86, ["pcengine", "pc engine", "turbografx 16", "turbografx16", "tg16", "pce", "nec pc engine turbografx 16"];
    "nec_turbografx_cd", "NEC TurboGrafx-CD", "NEC - PC Engine CD - TurboGrafx-CD", 4955, 150, ["pcenginecd", "pc engine cd", "turbografx cd", "tgcd", "tg cd", "pcecd"];
    "nec_supergrafx", "NEC SuperGrafx", "NEC - PC Engine SuperGrafx", -, 128, ["supergrafx", "sgx", "pc engine supergrafx"];
    "nec_pcfx", "NEC PC-FX", "NEC - PC-FX", 4930, 274, ["pcfx", "pc fx"];
    "nec_pc98", "NEC PC-98", "NEC - PC-98", 4934, 149, ["pc98", "pc 98"];
    "atari_2600", "Atari 2600", "Atari - 2600", 22, 59, ["atari2600", "2600", "a26"];
    "atari_5200", "Atari 5200", "Atari - 5200", 26, 66, ["atari5200", "5200"];
    "atari_7800", "Atari 7800", "Atari - 7800", 27, 60, ["atari7800", "7800"];
    "atari_8bit", "Atari 8-bit", "Atari - 8-bit Family", 4943, 65, ["atari800", "atarixe", "atari 8 bit family", "a800", "atarixl"];
    "atari_jaguar", "Atari Jaguar", "Atari - Jaguar", 28, 62, ["atarijaguar", "jaguar"];
    "atari_lynx", "Atari Lynx", "Atari - Lynx", 4924, 61, ["lynx", "atarilynx"];
    "atari_st", "Atari ST/STE", "Atari - ST", 4937, 63, ["atarist", "atari st"];
    "bandai_wonderswan", "Bandai WonderSwan", "Bandai - WonderSwan", 4925, 57, ["wonderswan", "wswan", "ws"];
    "bandai_wonderswan_color", "Bandai WonderSwan Color", "Bandai - WonderSwan Color", 4926, 123, ["wonderswancolor", "wonderswan color", "wswanc", "wsc"];
    "coleco_vision", "Coleco ColecoVision", "Coleco - ColecoVision", 31, 68, ["colecovision", "coleco"];
    "commodore_64", "Commodore 64", "Commodore - 64", 40, 15, ["c64", "commodore64"];
    "commodore_amiga", "Commodore Amiga", "Commodore - Amiga", 4911, 16, ["amiga", "amiga500", "amiga600", "amiga1200"];
    "commodore_amiga_cd32", "Commodore Amiga CD32", "Commodore - CD32", 4947, 114, ["amigacd32", "cd32", "amiga cd32"];
    "commodore_plus4", "Commodore Plus/4", "Commodore - Plus-4", 5007, 94, ["plus4", "cplus4", "c16"];
    "commodore_vci20", "Commodore VIC20", "Commodore - VIC-20", 4945, 71, ["vic20", "vic 20", "c20"];
    "commodore_pet", "Commodore PET", "Commodore - PET", -, 90, ["pet"];
    "vectrex", "GCE Vectrex", "GCE - Vectrex", 4939, 70, ["vectrex"];
    "magnavox_odyssey_2", "Magnavox Odyssey 2", "Magnavox - Odyssey2", 4927, 133, ["odyssey2", "odyssey 2", "videopac", "o2"];
    "mattel_intellivision", "Mattel Intellivision", "Mattel - Intellivision", 32, 67, ["intellivision", "intv"];
    "microsoft_msx", "Microsoft MSX", "Microsoft - MSX", 4929, 27, ["msx", "msx1"];
    "microsoft_msx2", "Microsoft MSX2", "Microsoft - MSX2", -, 53, ["msx2"];
    "amstrad_cpc", "Amstrad CPC", "Amstrad - CPC", 4914, 25, ["amstradcpc", "cpc"];
    "3do", "3DO Interactive Multiplayer", "The 3DO Company - 3DO", 25, 50, ["3do", "panasonic 3do"];
    "sharp_x68000", "Sharp X68000", "Sharp - X68000", 4931, 121, ["x68000", "x68k"];
    "sinclair_zxspectrum", "Sinclair ZX Spectrum", "Sinclair - ZX Spectrum", 4913, 26, ["zxspectrum", "zx spectrum", "speccy", "sinclair zx spectrum 3"];
    "sinclair_zx81", "Sinclair ZX81", "Sinclair - ZX 81", 5010, 373, ["zx81"];
    "snk_neogeo_aes", "SNK Neo Geo AES", "SNK - Neo Geo", 24, 80, ["neogeo", "neo geo", "mvs", "aes"];
    "snk_neogeo_cd", "SNK Neo Geo CD", "SNK - Neo Geo CD", 4956, 136, ["neogeocd", "neo geo cd", "ngcd", "neogeocdjp"];
    "snk_neogeopocket", "SNK Neo Geo Pocket", "SNK - Neo Geo Pocket", 4922, 119, ["ngp", "neo geo pocket"];
    "snk_neogeopocket_color", "SNK Neo Geo Pocket Color", "SNK - Neo Geo Pocket Color", 4923, 120, ["ngpc", "neo geo pocket color"];
    "thomson_mo5", "Thomson MO5", "Thomson - MOTO", -, 156, ["moto", "mo5", "to7", "to8", "thomson", "thomson to7"];
    "tic_80", "TIC-80", "TIC-80", -, -, ["tic80"];
    "watara_supervision", "Watara Supervision", "Watara - Supervision", 4959, -, ["supervision"];
];

/// Emulation platforms that are PCs (their games are PC games for the art sources).
const NOT_CONSOLES: &[&str] = &["pc_dos", "pc_windows", "pc_linux", "macintosh", "adobe_flash"];

fn plat_key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || *c == ' ')
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every console the art sources know: `PLATFORMS`, then the emulation platforms it lacks
/// (`crate::emulation::platforms`, so a console added there is known here too), with a lookup
/// from every spelling (normalised, and without spaces) to its row.
struct Consoles {
    rows: Vec<&'static Platform>,
    by_key: std::collections::HashMap<String, usize>,
}

fn consoles() -> &'static Consoles {
    static C: std::sync::OnceLock<Consoles> = std::sync::OnceLock::new();
    C.get_or_init(|| {
        let mut rows: Vec<&'static Platform> = PLATFORMS.iter().collect();
        let mut by_key = std::collections::HashMap::new();
        let add = |by_key: &mut std::collections::HashMap<String, usize>, s: &str, i: usize| {
            let k = plat_key(s);
            if k.is_empty() {
                return;
            }
            let squashed = k.replace(' ', "");
            by_key.entry(k).or_insert(i);
            by_key.entry(squashed).or_insert(i);
        };
        for (i, p) in PLATFORMS.iter().enumerate() {
            add(&mut by_key, p.name, i);
            add(&mut by_key, p.id, i);
            if let Some(lr) = p.libretro {
                add(&mut by_key, lr, i);
            }
            for a in p.aliases {
                add(&mut by_key, a, i);
            }
        }
        for ep in crate::emulation::platforms::all() {
            if NOT_CONSOLES.contains(&ep.id) {
                continue;
            }
            let known = rows
                .iter()
                .position(|p| p.id == ep.id)
                .or_else(|| ep.libretro.and_then(|lr| rows.iter().position(|p| p.libretro == Some(lr))));
            let i = match known {
                Some(i) => i,
                None if ep.libretro.is_some() || ep.tgdb.is_some() || ep.igdb.is_some() => {
                    // Built once per process: the leak is one row per platform.
                    rows.push(Box::leak(Box::new(Platform {
                        id: ep.id,
                        name: ep.name,
                        libretro: ep.libretro,
                        tgdb: ep.tgdb,
                        igdb: ep.igdb,
                        aliases: ep.folders,
                    })));
                    rows.len() - 1
                }
                None => continue,
            };
            for s in [ep.id, ep.name, ep.short]
                .into_iter()
                .chain(ep.libretro)
                .chain(ep.databases.iter().copied())
                .chain(ep.es.iter().copied())
                .chain(ep.folders.iter().copied())
            {
                add(&mut by_key, s, i);
            }
        }
        Consoles { rows, by_key }
    })
}

/// The console platform a game's `platform` names: a display name, alias, ES folder name,
/// libretro system or Playnite id (None for PC / unknown platforms).
pub fn console_platform(platform: Option<&str>) -> Option<&'static Platform> {
    let k = plat_key(platform?);
    if k.is_empty() {
        return None;
    }
    let c = consoles();
    c.by_key.get(&k).or_else(|| c.by_key.get(&k.replace(' ', ""))).map(|&i| c.rows[i])
}

/// The platform the art sources should use for `g`: its library platform when that is a console,
/// else (an emulated game with no or an unknown platform) its emulation platform's name.
pub fn art_platform(g: &crate::model::Game) -> Option<String> {
    let lib = g.platform.as_deref().map(str::trim).filter(|p| !p.is_empty());
    if lib.is_some_and(|p| console_platform(Some(p)).is_some()) {
        return lib.map(str::to_string);
    }
    g.emu
        .as_ref()
        .map(|e| e.platform.trim())
        .and_then(|id| console_platform(Some(id)))
        .map(|p| p.name.to_string())
        .or_else(|| lib.map(str::to_string))
}

/// The ROM's own names for an emulated game, best first: the No-Intro / Redump name (with its
/// region) and the ROM file name. libretro-thumbnails names its images after the same sets, so
/// these pick the right regional cover exactly.
pub fn rom_names(g: &crate::model::Game) -> Vec<String> {
    let Some(e) = &g.emu else {
        return vec![];
    };
    let mut out: Vec<String> = vec![];
    let mut push = |s: &str| {
        let s = s.trim();
        if !s.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(s)) {
            out.push(s.to_string());
        }
    };
    if let Some(d) = &e.dat_name {
        push(d);
    }
    // Folder games (Wii U, PS3) are named after their folder; generic launch files say nothing.
    let stem = e.rom.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if !["eboot", "title", "default", "main", "game", "index"].contains(&stem.to_ascii_lowercase().as_str()) {
        push(stem);
    }
    out
}

/// A name as libretro-thumbnails spells its files (`&*/:`<>?\|"` become `_`).
pub fn libretro_file_name(name: &str) -> String {
    name.chars().map(|c| if "&*/:`<>?\\|\"".contains(c) { '_' } else { c }).collect()
}

/// The region words of a No-Intro / Redump name (its first parenthesised group, lower case):
/// "Tekken 5 (Europe, Australia)" → ["europe", "australia"].
fn region_words(name: &str) -> Vec<String> {
    let Some(open) = name.find('(') else {
        return vec![];
    };
    let rest = &name[open + 1..];
    let Some(close) = rest.find(')') else {
        return vec![];
    };
    rest[..close].split(',').map(|w| w.trim().to_lowercase()).filter(|w| !w.is_empty()).collect()
}

// ---------------------------------------------------------------------------------------------
// Lutris.net

#[derive(Clone, Debug, PartialEq)]
pub struct LutrisGame {
    pub name: String,
    pub slug: String,
    pub coverart: Option<String>,
    pub banner: Option<String>,
    pub icon: Option<String>,
    /// Steam appids Lutris links the game to.
    pub steam: Vec<u32>,
}

pub fn lutris_search_url(term: &str) -> String {
    format!("{LUTRIS_API}?search={}", encode(term.trim()))
}

/// `api/games?search=` → games. None = malformed.
pub fn parse_lutris(body: &[u8]) -> Option<Vec<LutrisGame>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let lutris_url = |s: &str| https(s).filter(|u| u.starts_with("https://lutris.net/"));
    Some(
        v.get("results")?
            .as_array()?
            .iter()
            .filter_map(|g| {
                let name = str_of(g, "name").to_string();
                if name.is_empty() {
                    return None;
                }
                let steam = g
                    .get("provider_games")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter(|p| str_of(p, "service") == "steam")
                            .filter_map(|p| str_of(p, "slug").parse().ok())
                            .collect()
                    })
                    .unwrap_or_default();
                Some(LutrisGame {
                    name,
                    slug: str_of(g, "slug").to_string(),
                    coverart: lutris_url(str_of(g, "coverart")),
                    banner: lutris_url(str_of(g, "banner_url")),
                    icon: lutris_url(str_of(g, "icon_url")),
                    steam,
                })
            })
            .collect(),
    )
}

/// The Lutris entry that is this game: by Steam appid when both know one, else same name.
pub fn confident_lutris<'a>(name: &str, appid: Option<u32>, games: &'a [LutrisGame]) -> Option<&'a LutrisGame> {
    if let Some(a) = appid
        && let Some(g) = games.iter().find(|g| g.steam.contains(&a))
    {
        return Some(g);
    }
    games.iter().find(|g| same_name(name, &g.name) && appid.is_none_or(|a| g.steam.is_empty() || g.steam.contains(&a)))
}

pub fn lutris_pics(g: &LutrisGame, slot: ArtSlot) -> Vec<Pic> {
    let one = |u: &Option<String>, w, h| u.iter().map(|u| Pic::new(u.clone(), w, h)).collect();
    match slot {
        ArtSlot::Cover => one(&g.coverart, 264, 352),
        ArtSlot::Banner => one(&g.banner, 184, 69),
        ArtSlot::Icon => one(&g.icon, 128, 128),
        _ => vec![],
    }
}

// ---------------------------------------------------------------------------------------------
// GOG

#[derive(Clone, Debug, Default, PartialEq)]
pub struct GogProduct {
    pub id: u64,
    pub title: String,
    pub cover: Option<String>,
    /// Wide capsule (catalog only).
    pub horizontal: Option<String>,
    pub background: Vec<String>,
    pub logo: Option<String>,
    pub icon: Vec<String>,
}

pub fn gog_catalog_url(term: &str) -> String {
    format!(
        "{GOG_CATALOG}?limit=10&query=like:{}&order=desc:score&productType=in:game,pack&locale=en-US",
        encode(term.trim())
    )
}

pub fn gog_game_url(id: u64) -> String {
    format!("{GOG_GAMES}/{id}?locale=en-US")
}

fn gog_img(s: &str) -> Option<String> {
    https(s).filter(|u| u.split('/').nth(2).is_some_and(|h| h.ends_with(".gog-statics.com") || h.ends_with(".gog.com")))
}

/// Catalog search → products. None = malformed.
pub fn parse_gog_catalog(body: &[u8]) -> Option<Vec<GogProduct>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    Some(
        v.get("products")?
            .as_array()?
            .iter()
            .filter_map(|p| {
                Some(GogProduct {
                    id: id_of(p, "id")?,
                    title: str_of(p, "title").to_string(),
                    cover: gog_img(str_of(p, "coverVertical")),
                    horizontal: gog_img(str_of(p, "coverHorizontal")),
                    background: gog_img(str_of(p, "galaxyBackgroundImage")).into_iter().collect(),
                    logo: gog_img(str_of(p, "logo")),
                    icon: vec![],
                })
            })
            .collect(),
    )
}

/// `v2/games/<id>` → the product's images. None = malformed.
pub fn parse_gog_game(body: &[u8]) -> Option<GogProduct> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let links = v.get("_links")?;
    let link = |k: &str| links.get(k).and_then(|l| l.get("href")).and_then(Value::as_str).and_then(gog_img);
    let product = v.pointer("/_embedded/product");
    Some(GogProduct {
        id: product.and_then(|p| id_of(p, "id")).unwrap_or(0),
        title: product.map(|p| str_of(p, "title")).unwrap_or("").to_string(),
        cover: link("boxArtImage"),
        horizontal: None,
        background: [link("galaxyBackgroundImage"), link("backgroundImage")].into_iter().flatten().collect(),
        logo: link("logo"),
        icon: [link("iconSquare"), link("icon")].into_iter().flatten().collect(),
    })
}

pub fn confident_gog<'a>(name: &str, products: &'a [GogProduct]) -> Option<&'a GogProduct> {
    products.iter().find(|p| same_name(name, &p.title))
}

pub fn gog_pics(p: &GogProduct, slot: ArtSlot) -> Vec<Pic> {
    let mut v = vec![];
    match slot {
        ArtSlot::Cover => v.extend(p.cover.iter().map(|u| Pic::new(u.clone(), 342, 482))),
        ArtSlot::Hero => v.extend(p.background.iter().map(|u| Pic::new(u.clone(), 2560, 670))),
        ArtSlot::Logo => v.extend(p.logo.iter().map(|u| Pic::new(u.clone(), 0, 0))),
        ArtSlot::Icon => v.extend(p.icon.iter().map(|u| Pic::new(u.clone(), 128, 128))),
        ArtSlot::Banner => v.extend(p.horizontal.iter().map(|u| Pic::new(u.clone(), 1600, 740))),
    }
    v.dedup_by(|a, b| a.url == b.url);
    v
}

// ---------------------------------------------------------------------------------------------
// PCGamingWiki

#[derive(Clone, Debug, PartialEq)]
pub struct PcgwPage {
    pub title: String,
    /// The asked-for title was a redirect to this page.
    pub redirected: bool,
    /// Infobox `cover` file name.
    pub cover: Option<String>,
    pub steam_appids: Vec<u32>,
}

pub fn pcgw_page_url(title: &str) -> String {
    format!(
        "{PCGW_API}?action=query&prop=revisions&rvprop=content&rvslots=main&rvsection=0&titles={}&redirects=1&format=json&formatversion=2",
        encode(title.trim())
    )
}

pub fn pcgw_search_url(term: &str) -> String {
    format!(
        "{PCGW_API}?action=query&list=search&srsearch={}&srlimit=5&srprop=&srnamespace=0&format=json&formatversion=2",
        encode(term.trim())
    )
}

pub fn pcgw_file_url(file: &str) -> String {
    format!(
        "{PCGW_API}?action=query&prop=imageinfo&iiprop=url%7Csize&iiurlwidth=300&titles={}&format=json&formatversion=2",
        encode(&format!("File:{}", file.trim()))
    )
}

/// A `|key = value` line of an infobox.
fn infobox_field<'a>(wikitext: &'a str, key: &str) -> Option<&'a str> {
    wikitext.lines().find_map(|l| {
        let l = l.trim_start().strip_prefix('|')?;
        let (k, v) = l.split_once('=')?;
        (k.trim().eq_ignore_ascii_case(key)).then(|| v.trim())
    })
}

/// Page query → the page (None = malformed, Some(None) = no such page).
pub fn parse_pcgw_page(body: &[u8]) -> Option<Option<PcgwPage>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let q = v.get("query")?;
    let page = q.get("pages")?.as_array()?.first()?;
    if page.get("missing").is_some() || page.get("invalid").is_some() {
        return Some(None);
    }
    let title = str_of(page, "title").to_string();
    let text = page.pointer("/revisions/0/slots/main/content").and_then(Value::as_str).unwrap_or("");
    if !text.contains("{{Infobox game") {
        return Some(None); // disambiguation, series or company page
    }
    let cover = infobox_field(text, "cover")
        .filter(|c| !c.is_empty() && !c.contains(['{', '}', '<', '|', '[']))
        .map(str::to_string);
    let steam_appids = infobox_field(text, "steam appid")
        .map(|s| s.split([',', ' ']).filter_map(|p| p.trim().parse().ok()).collect())
        .unwrap_or_default();
    let redirected = q.get("redirects").and_then(Value::as_array).is_some_and(|r| !r.is_empty());
    Some(Some(PcgwPage { title, redirected, cover, steam_appids }))
}

/// `list=search` → page titles. None = malformed.
pub fn parse_pcgw_search(body: &[u8]) -> Option<Vec<String>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    Some(
        v.pointer("/query/search")?
            .as_array()?
            .iter()
            .map(|h| str_of(h, "title").to_string())
            .filter(|t| !t.is_empty())
            .collect(),
    )
}

/// `prop=imageinfo` → the file (None = malformed, Some(None) = no such file).
pub fn parse_pcgw_file(body: &[u8]) -> Option<Option<Pic>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let page = v.pointer("/query/pages")?.as_array()?.first()?;
    let Some(ii) = page.pointer("/imageinfo/0") else {
        return Some(None);
    };
    let pcgw = |s: &str| https(s).filter(|u| u.contains(".pcgamingwiki.com/"));
    let Some(url) = pcgw(str_of(ii, "url")) else {
        return Some(None);
    };
    let n = |k: &str| ii.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
    Some(Some(Pic::new(url, n("width"), n("height")).thumb(pcgw(str_of(ii, "thumburl")))))
}

/// Is `page` the wiki page of the game (`name`, Steam `appid`)? With an appid, the infobox must
/// list it; without one, the title must be the name (or the wiki redirected the name to it).
pub fn pcgw_confident(name: &str, appid: Option<u32>, page: &PcgwPage) -> bool {
    match appid {
        Some(a) if !page.steam_appids.is_empty() => page.steam_appids.contains(&a),
        Some(_) => same_name(name, &page.title),
        None => page.redirected || same_name(name, &page.title),
    }
}

// ---------------------------------------------------------------------------------------------
// libretro-thumbnails

/// Image kinds (folders) of a libretro-thumbnails repository.
pub const LR_BOXART: &str = "Named_Boxarts";
pub const LR_SNAP: &str = "Named_Snaps";
pub const LR_TITLE: &str = "Named_Titles";

pub fn libretro_repo(system: &str) -> String {
    system.replace(' ', "_")
}

/// The box art names of a system (GitHub tree of `Named_Boxarts`).
pub fn libretro_index_url(system: &str) -> String {
    format!("{LIBRETRO_TREES}/{}/git/trees/master:{LR_BOXART}", libretro_repo(system))
}

pub fn libretro_image_url(system: &str, kind: &str, stem: &str) -> String {
    format!("{LIBRETRO_RAW}/{}/master/{kind}/{}.png", libretro_repo(system), encode(stem))
}

/// GitHub tree → image names without `.png` (symlinked alternates skipped: the raw host
/// returns the link text for them). None = malformed.
pub fn parse_libretro_tree(body: &[u8]) -> Option<Vec<String>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let tree = v.get("tree")?.as_array()?;
    Some(
        tree.iter()
            .filter(|t| str_of(t, "type") == "blob" && str_of(t, "mode") != "120000")
            .filter_map(|t| str_of(t, "path").strip_suffix(".png").map(str::to_string))
            .filter(|s| !s.is_empty() && !s.contains('/'))
            .collect(),
    )
}

/// A No-Intro name as people write it: "(…)" / "[…]" tags dropped and a trailing article moved
/// back ("Legend of Zelda, The - Ocarina of Time (USA)" → "The Legend of Zelda - Ocarina of Time").
pub fn nointro_title(stem: &str) -> String {
    let mut s = String::new();
    let mut depth = 0i32;
    for c in stem.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => s.push(c),
            _ => {}
        }
    }
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let (head, tail) = match s.split_once(" - ") {
        Some((h, t)) => (h.to_string(), Some(t.to_string())),
        None => (s.clone(), None),
    };
    let mut head = head;
    for art in ["The", "A", "An"] {
        if let Some(h) = head.strip_suffix(&format!(", {art}")) {
            head = format!("{art} {h}");
            break;
        }
    }
    match tail {
        Some(t) => format!("{head} - {t}"),
        None => head,
    }
}

/// Lower is better: released, preferred region.
fn nointro_rank(stem: &str) -> u32 {
    let tags = stem.to_ascii_lowercase();
    let mut r = if tags.contains("usa") {
        0
    } else if tags.contains("world") {
        1
    } else if tags.contains("europe") {
        2
    } else if tags.contains("japan") {
        4
    } else {
        3
    };
    for bad in [
        "(beta",
        "(proto",
        "(demo",
        "(sample",
        "(debug",
        "(kiosk",
        "(pirate",
        "(hack",
        "(unl)",
        "gamecube",
        "virtual console",
        "(manual",
    ] {
        if tags.contains(bad) {
            r += 20;
        }
    }
    if tags.contains("(rev") || tags.contains("(v1.") {
        r += 1;
    }
    r
}

/// The box art names of one libretro system, with their match keys worked out once (big
/// systems have thousands of names; bulk runs match every game against them).
#[derive(Debug, Default)]
pub struct LrIndex {
    pub stems: Vec<String>,
    /// `norm_name(nointro_title(stem))` of each stem.
    keys: Vec<String>,
    /// Stem indices by key and by lowercase file name.
    by_key: std::collections::HashMap<String, Vec<u32>>,
    by_file: std::collections::HashMap<String, u32>,
}

impl LrIndex {
    pub fn new(stems: Vec<String>) -> LrIndex {
        let keys: Vec<String> = stems.iter().map(|s| super::norm_name(&nointro_title(s))).collect();
        let mut by_key: std::collections::HashMap<String, Vec<u32>> = Default::default();
        let mut by_file = std::collections::HashMap::new();
        for (i, (s, k)) in stems.iter().zip(&keys).enumerate() {
            if !k.is_empty() {
                by_key.entry(k.clone()).or_default().push(i as u32);
            }
            by_file.entry(s.to_lowercase()).or_insert(i as u32);
        }
        LrIndex { stems, keys, by_key, by_file }
    }

    fn ranked(&self, mut idx: Vec<u32>) -> Vec<&str> {
        idx.sort_by_key(|&i| {
            let s = &self.stems[i as usize];
            (nointro_rank(s), s.len())
        });
        idx.into_iter().map(|i| self.stems[i as usize].as_str()).collect()
    }

    /// Names that are the game `name`, best first: the exact file name, else the same title in
    /// any region (released USA / World / Europe versions first).
    pub fn matches(&self, name: &str) -> Vec<&str> {
        let name = name.trim();
        if let Some(&i) = self.by_file.get(&name.to_lowercase()) {
            return vec![self.stems[i as usize].as_str()];
        }
        let want = super::norm_name(&nointro_title(name));
        match self.by_key.get(&want) {
            Some(v) if !want.is_empty() => self.ranked(v.clone()),
            _ => vec![],
        }
    }

    /// Like `matches`, but the ROM's own names (`rom_names`: No-Intro / Redump name, file name)
    /// come first: an exact image name wins; else the same title, the ROM's region first.
    pub fn matches_rom(&self, rom_names: &[String], name: &str) -> Vec<&str> {
        for n in rom_names {
            if let Some(&i) = self.by_file.get(&libretro_file_name(n.trim()).to_lowercase()) {
                return vec![self.stems[i as usize].as_str()];
            }
        }
        let mut hits = std::iter::once(name)
            .chain(rom_names.iter().map(String::as_str))
            .map(|n| self.matches(n))
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        let region = rom_names.iter().map(|n| region_words(n)).find(|r| !r.is_empty()).unwrap_or_default();
        if !region.is_empty() {
            // Stable: the usual ranking stays within each group.
            hits.sort_by_key(|s| !region_words(s).iter().any(|w| region.contains(w)));
        }
        hits
    }

    /// Looser matches for a search: every word of `term` in the title (best ranked first, one
    /// per title).
    pub fn search(&self, term: &str, max: usize) -> Vec<&str> {
        let exact = self.matches(term);
        if !exact.is_empty() {
            return exact.into_iter().take(max).collect();
        }
        let want = super::norm_name(&nointro_title(term));
        let words: Vec<&str> = want.split(' ').filter(|w| !w.is_empty()).collect();
        if words.is_empty() {
            return vec![];
        }
        let idx: Vec<u32> = self
            .keys
            .iter()
            .enumerate()
            .filter(|(_, k)| {
                let tw: Vec<&str> = k.split(' ').collect();
                words.iter().all(|w| tw.contains(w))
            })
            .map(|(i, _)| i as u32)
            .collect();
        let mut seen = std::collections::HashSet::new();
        let mut hits: Vec<(u32, &str)> = vec![];
        let mut ranked_idx = idx;
        ranked_idx.sort_by_key(|&i| {
            let s = &self.stems[i as usize];
            (nointro_rank(s), s.len())
        });
        for i in ranked_idx {
            if seen.insert(self.keys[i as usize].as_str()) {
                hits.push((i, self.stems[i as usize].as_str()));
            }
            if hits.len() >= max {
                break;
            }
        }
        hits.into_iter().map(|(_, s)| s).collect()
    }
}

/// Pictures of one No-Intro name for `slot` (the snap / title may not exist for every name:
/// the engine checks).
pub fn libretro_pics(system: &str, stem: &str, slot: ArtSlot) -> Vec<Pic> {
    let region = stem.split_once('(').map(|(_, r)| r.split(')').next().unwrap_or("").to_string()).unwrap_or_default();
    match slot {
        ArtSlot::Cover => vec![Pic::new(libretro_image_url(system, LR_BOXART, stem), 0, 0).note(&region)],
        ArtSlot::Hero => vec![
            Pic::new(libretro_image_url(system, LR_SNAP, stem), 0, 0).note("screenshot"),
            Pic::new(libretro_image_url(system, LR_TITLE, stem), 0, 0).note("title screen"),
        ],
        _ => vec![],
    }
}

/// The raw host serves a symlinked image as its link text ("Other Name (USA).png"): the URL of
/// the real file, when `body` is such a link next to `url`.
pub fn libretro_symlink(url: &str, body: &[u8]) -> Option<String> {
    if !url.starts_with(LIBRETRO_RAW) || body.len() > 512 {
        return None;
    }
    let target = std::str::from_utf8(body).ok()?.trim();
    if !target.ends_with(".png") || target.contains(['/', '\n', '\\']) || target.starts_with('.') {
        return None;
    }
    let (dir, _) = url.rsplit_once('/')?;
    Some(format!("{dir}/{}", encode(target)))
}

// ---------------------------------------------------------------------------------------------
// TheGamesDB

/// TheGamesDB's PC platform.
const TGDB_PC: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct TgdbGame {
    pub id: u64,
    pub title: String,
    pub platform: Option<u32>,
    /// Front box art.
    pub boxart: Option<Pic>,
}

/// Search URL without the key (added by the engine when sending).
pub fn tgdb_search_url(term: &str, platform: Option<u32>) -> String {
    let mut u = format!("{TGDB_API}/v1.1/Games/ByGameName?name={}&include=boxart", encode(term.trim()));
    if let Some(p) = platform {
        u.push_str(&format!("&filter%5Bplatform%5D={p}"));
    }
    u
}

pub fn tgdb_images_url(id: u64) -> String {
    format!("{TGDB_API}/v1/Games/Images?games_id={id}&filter%5Btype%5D=fanart,banner,clearlogo,boxart")
}

pub fn is_tgdb_api(url: &str) -> bool {
    url.starts_with(TGDB_API)
}

fn tgdb_base(base: Option<&Value>, size: &str) -> Option<String> {
    base?.get(size).and_then(Value::as_str).and_then(https).filter(|u| u.contains(".thegamesdb.net/"))
}

fn tgdb_pic(base: Option<&Value>, img: &Value) -> Option<Pic> {
    let file = str_of(img, "filename");
    if file.is_empty() || file.contains("..") || file.starts_with('/') {
        return None;
    }
    let (w, h) = dims(str_of(img, "resolution"));
    Some(
        Pic::new(format!("{}{file}", tgdb_base(base, "original")?), w, h)
            .thumb(tgdb_base(base, "thumb").map(|t| format!("{t}{file}"))),
    )
}

/// `Games/ByGameName` (with `include=boxart`) → games. None = malformed.
pub fn parse_tgdb_games(body: &[u8]) -> Option<Vec<TgdbGame>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let games = v.pointer("/data/games")?.as_array()?;
    let base = v.pointer("/include/boxart/base_url");
    let boxes = v.pointer("/include/boxart/data");
    Some(
        games
            .iter()
            .filter_map(|g| {
                let id = id_of(g, "id")?;
                let boxart = boxes
                    .and_then(|b| b.get(id.to_string()))
                    .and_then(Value::as_array)
                    .and_then(|a| a.iter().filter(|i| str_of(i, "side") == "front").find_map(|i| tgdb_pic(base, i)));
                Some(TgdbGame {
                    id,
                    title: str_of(g, "game_title").to_string(),
                    platform: id_of(g, "platform").map(|p| p as u32),
                    boxart,
                })
            })
            .collect(),
    )
}

/// `Games/Images` → (type, picture) of game `id`. None = malformed.
pub fn parse_tgdb_images(body: &[u8], id: u64) -> Option<Vec<(String, Pic)>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let data = v.get("data")?;
    let base = data.get("base_url");
    let Some(imgs) = data.get("images").and_then(|i| i.get(id.to_string())).and_then(Value::as_array) else {
        return Some(vec![]);
    };
    Some(
        imgs.iter()
            .filter(|i| str_of(i, "type") != "boxart" || str_of(i, "side") == "front")
            .filter_map(|i| Some((str_of(i, "type").to_string(), tgdb_pic(base, i)?)))
            .collect(),
    )
}

pub fn confident_tgdb<'a>(name: &str, games: &'a [TgdbGame], pc: bool) -> Option<&'a TgdbGame> {
    let same = |g: &&TgdbGame| same_name(name, &g.title);
    pc.then(|| games.iter().filter(same).find(|g| g.platform == Some(TGDB_PC)))
        .flatten()
        .or_else(|| games.iter().find(same))
}

/// The TheGamesDB image type for a slot.
pub fn tgdb_kind(slot: ArtSlot) -> Option<&'static str> {
    match slot {
        ArtSlot::Cover => Some("boxart"),
        ArtSlot::Hero => Some("fanart"),
        ArtSlot::Logo => Some("clearlogo"),
        ArtSlot::Banner => Some("banner"),
        ArtSlot::Icon => None,
    }
}
