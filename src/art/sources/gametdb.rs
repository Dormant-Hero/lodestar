//! GameTDB artwork (no key): covers and disc / cartridge art of Wii, GameCube, Wii U, DS and
//! 3DS games by their game ID — the code the ROM header carries (read by the emulation scan:
//! `crate::scan::emulation::identify`, stored as `EmuRef::serial`).
//!
//! URL scheme (checked against art.gametdb.com, Nov 2026):
//! `https://art.gametdb.com/<system>/<kind>/<region>/<ID>.<ext>`
//!
//! | system (games) | front cover | 3D / boxed | full (front + spine + back) | disc |
//! | --- | --- | --- | --- | --- |
//! | `wii` (Wii and GameCube, 6-char ID "RMGE01") | `cover` png 160×224 | `cover3D` png 176×248 | `coverfullHQ` png 1024×680 | `disc` png 160×160 |
//! | `wiiu` (6-char ID "ALZE01") | `coverHQ` jpg 768×1080 | `cover3D` png 176×248 | `coverfullHQ` jpg 1632×1080 | `disc` png 160×160 |
//! | `ds` (4-char game code "ASME") | `coverHQ` jpg 768×680 | `box` png 240×216 | `coverfullHQ` jpg 1616×680 | `cart` png 336×360 |
//! | `3ds` (4-char code from "CTR-P-AMKE") | `coverHQ` jpg 768×680 | `box` png 240×216 | `coverfullHQ` jpg 1616×680 | `cart` png 360×360 |
//!
//! Regions are GameTDB's language folders (US, EN, FR, DE, ES, IT, NL, PT, AU, SE, DK, NO, FI,
//! RU, KO, JA, ZHTW, ZHCN). The order tried: the region the game ID names (a European "P" disc
//! in the user's own language when it is a European one), the user's region (from the
//! locale), then US → EN → JA. A missing image is a 404; the engine asks with HEAD first.

use super::ArtSlot;
use super::more::Pic;

pub const SRC_GAMETDB: &str = "GameTDB";
pub const GAMETDB_ART: &str = "https://art.gametdb.com";

/// The GameTDB art folder of a console (Playnite platform id); None = GameTDB has no art.
pub fn gametdb_system(platform_id: &str) -> Option<&'static str> {
    Some(match platform_id {
        "nintendo_wii" | "nintendo_gamecube" => "wii",
        "nintendo_wiiu" => "wiiu",
        "nintendo_ds" | "nintendo_dsi" => "ds",
        "nintendo_3ds" => "3ds",
        _ => return None,
    })
}

/// The GameTDB ID of a game from the serial its ROM header carries: Wii / GameCube / Wii U
/// six characters ("RMGE01"), DS four ("ASME"), 3DS the last part of the product code
/// ("CTR-P-AMKE" → "AMKE").
pub fn gametdb_id(system: &str, serial: &str) -> Option<String> {
    let s = serial.trim();
    let ok = |c: &str, n: usize| c.len() == n && c.chars().all(|c| c.is_ascii_alphanumeric());
    let id = match system {
        "wii" | "wiiu" => s.to_string(),
        "ds" => s.to_string(),
        "3ds" => s.rsplit('-').next().unwrap_or("").to_string(),
        _ => return None,
    };
    let n = if matches!(system, "wii" | "wiiu") { 6 } else { 4 };
    ok(&id, n).then(|| id.to_ascii_uppercase())
}

/// (system, ID) of a console game with a usable serial.
pub fn gametdb_key(platform_id: Option<&str>, serial: Option<&str>) -> Option<(&'static str, String)> {
    let system = gametdb_system(platform_id?)?;
    Some((system, gametdb_id(system, serial?)?))
}

/// (system, ID) of a library game (emulated games with a header serial).
pub fn gametdb_key_of(g: &crate::model::Game) -> Option<(&'static str, String)> {
    let e = g.emu.as_ref()?;
    gametdb_key(Some(e.platform.trim()), e.serial.as_deref())
}

/// The serial a library game's ROM header carries (emulated games).
pub fn rom_serial(g: &crate::model::Game) -> Option<String> {
    g.emu.as_ref().and_then(|e| e.serial.clone()).filter(|s| !s.trim().is_empty())
}

/// European languages GameTDB keeps covers for.
const PAL: &[&str] = &["EN", "FR", "DE", "ES", "IT", "NL", "PT", "SE", "DK", "NO", "FI", "RU", "AU"];

/// GameTDB region of a locale ("de_DE.UTF-8" → DE, "en_US" → US, "en_GB" → EN, "pt_BR" → PT).
pub fn locale_region(locale: &str) -> Option<&'static str> {
    let l = locale.split(['.', '@']).next().unwrap_or("").to_ascii_lowercase();
    let (lang, country) = l.split_once(['_', '-']).unwrap_or((&l, ""));
    Some(match (lang, country) {
        ("en", "us" | "ca") => "US",
        ("en", "au" | "nz") => "AU",
        ("en", _) => "EN",
        ("fr", "ca") => "US",
        ("fr", _) => "FR",
        ("de", _) => "DE",
        ("es", "es") | ("es", "") => "ES",
        ("es", _) => "US",
        ("it", _) => "IT",
        ("nl", _) => "NL",
        ("pt", _) => "PT",
        ("sv", _) => "SE",
        ("da", _) => "DK",
        ("nb" | "nn" | "no", _) => "NO",
        ("fi", _) => "FI",
        ("ru", _) => "RU",
        ("ko", _) => "KO",
        ("ja", _) => "JA",
        ("zh", "tw" | "hk") => "ZHTW",
        ("zh", _) => "ZHCN",
        _ => return None,
    })
}

/// The user's region guess from the locale (LC_ALL, LC_MESSAGES, LANG).
pub fn user_region() -> Option<&'static str> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .find(|v| !v.trim().is_empty() && v != "C" && v != "POSIX")
        .and_then(|v| locale_region(&v))
}

/// The regions to try for `id` in order: the ID's own region (European discs in the user's
/// European language first), the user's region, then US → EN → JA.
pub fn regions(id: &str, user: Option<&'static str>) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = vec![];
    let mut push = |r: &'static str| {
        if !out.contains(&r) {
            out.push(r);
        }
    };
    // The region letter: 4th character of both the 4- and 6-character IDs.
    let pal_user = user.filter(|u| PAL.contains(u));
    match id.chars().nth(3).map(|c| c.to_ascii_uppercase()) {
        Some('E') | Some('N') => push("US"),
        Some('J') => push("JA"),
        Some('K') | Some('Q') | Some('T') => push("KO"),
        Some('W') => push("ZHTW"),
        Some('C') => push("ZHCN"),
        Some('D') => push("DE"),
        Some('F') => push("FR"),
        Some('I') => push("IT"),
        Some('S') => push("ES"),
        Some('H') => push("NL"),
        Some('R') => push("RU"),
        Some('U') => push("AU"),
        Some('V') => push("SE"),
        Some('P') | Some('X') | Some('Y') | Some('Z') | Some('L') | Some('M') => {
            if let Some(u) = pal_user {
                push(u);
            }
            push("EN");
        }
        _ => {}
    }
    if let Some(u) = user {
        push(u);
    }
    for r in ["US", "EN", "JA"] {
        push(r);
    }
    out
}

/// One picture kind of a system: (folder, extension, width, height, label).
type Kind = (&'static str, &'static str, u32, u32, &'static str);

/// The picture kinds of `system` for `slot`, best first (the first is what automatic
/// downloads take). Covers: front, boxed / 3D, full; icons: the disc or cartridge.
pub fn gametdb_kinds(system: &str, slot: ArtSlot) -> &'static [Kind] {
    match (system, slot) {
        ("wii", ArtSlot::Cover) => &[
            ("cover", "png", 160, 224, "front"),
            ("cover3D", "png", 176, 248, "3D box"),
            ("coverfullHQ", "png", 1024, 680, "full box"),
        ],
        ("wiiu", ArtSlot::Cover) => &[
            ("coverHQ", "jpg", 768, 1080, "front"),
            ("cover3D", "png", 176, 248, "3D box"),
            ("coverfullHQ", "jpg", 1632, 1080, "full box"),
        ],
        ("ds", ArtSlot::Cover) | ("3ds", ArtSlot::Cover) => &[
            ("coverHQ", "jpg", 768, 680, "front"),
            ("box", "png", 240, 216, "box"),
            ("coverfullHQ", "jpg", 1616, 680, "full box"),
        ],
        ("wii", ArtSlot::Icon) | ("wiiu", ArtSlot::Icon) => &[("disc", "png", 160, 160, "disc")],
        ("ds", ArtSlot::Icon) => &[("cart", "png", 336, 360, "cartridge")],
        ("3ds", ArtSlot::Icon) => &[("cart", "png", 360, 360, "cartridge")],
        _ => &[],
    }
}

pub fn gametdb_url(system: &str, kind: &str, region: &str, id: &str, ext: &str) -> String {
    format!("{GAMETDB_ART}/{system}/{kind}/{region}/{id}.{ext}")
}

/// A smaller copy for the gallery's thumbnail (the HQ covers are big).
fn thumb_of(system: &str, kind: &str, region: &str, id: &str) -> Option<String> {
    match (system, kind) {
        ("ds" | "3ds", "coverHQ") => Some(gametdb_url(system, "coverM", region, id, "jpg")),
        ("wiiu", "coverHQ") => Some(gametdb_url(system, "cover", region, id, "jpg")),
        ("ds" | "3ds" | "wiiu", "coverfullHQ") => Some(gametdb_url(system, "coverfull", region, id, "jpg")),
        _ => None,
    }
}

/// The picture of `kind` in `region` (label "front · US").
pub fn gametdb_pic(system: &str, k: &Kind, region: &str, id: &str) -> Pic {
    let (kind, ext, w, h, label) = *k;
    let mut p = Pic::new(gametdb_url(system, kind, region, id, ext), w, h);
    p.thumb = thumb_of(system, kind, region, id);
    p.note = format!("{label} · {region}");
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_from_rom_serials() {
        assert_eq!(gametdb_id("wii", "RMGE01").as_deref(), Some("RMGE01"));
        assert_eq!(gametdb_id("wii", "GALE01").as_deref(), Some("GALE01"));
        assert_eq!(gametdb_id("wii", "SLUS-20312"), None);
        assert_eq!(gametdb_id("wiiu", "ALZE01").as_deref(), Some("ALZE01"));
        // A WUD's 4-character code can't name the image (publisher part missing).
        assert_eq!(gametdb_id("wiiu", "ALZE"), None);
        assert_eq!(gametdb_id("ds", "asme").as_deref(), Some("ASME"));
        assert_eq!(gametdb_id("ds", "ASMEX"), None);
        assert_eq!(gametdb_id("3ds", "CTR-P-AMKE").as_deref(), Some("AMKE"));
        assert_eq!(gametdb_id("3ds", "KTR-P-CB8E").as_deref(), Some("CB8E"));
        assert_eq!(gametdb_id("3ds", "0004000000175E00"), None);
        assert_eq!(gametdb_id("gba", "BPEE"), None);
    }

    #[test]
    fn systems_and_keys() {
        assert_eq!(gametdb_system("nintendo_gamecube"), Some("wii"));
        assert_eq!(gametdb_system("nintendo_wii"), Some("wii"));
        assert_eq!(gametdb_system("nintendo_wiiu"), Some("wiiu"));
        assert_eq!(gametdb_system("nintendo_ds"), Some("ds"));
        assert_eq!(gametdb_system("nintendo_3ds"), Some("3ds"));
        assert_eq!(gametdb_system("nintendo_gba"), None);
        assert_eq!(gametdb_key(Some("nintendo_3ds"), Some("CTR-P-BNEA")), Some(("3ds", "BNEA".into())));
        assert_eq!(gametdb_key(Some("nintendo_wii"), None), None);
        assert_eq!(gametdb_key(None, Some("RMGE01")), None);
    }

    #[test]
    fn urls_follow_the_gametdb_scheme() {
        assert_eq!(
            gametdb_url("wii", "cover", "US", "RMGE01", "png"),
            "https://art.gametdb.com/wii/cover/US/RMGE01.png"
        );
        let front = gametdb_pic("wiiu", &gametdb_kinds("wiiu", ArtSlot::Cover)[0], "EN", "ALZP01");
        assert_eq!(front.url, "https://art.gametdb.com/wiiu/coverHQ/EN/ALZP01.jpg");
        assert_eq!(front.thumb.as_deref(), Some("https://art.gametdb.com/wiiu/cover/EN/ALZP01.jpg"));
        assert_eq!((front.width, front.height, front.note.as_str()), (768, 1080, "front · EN"));
        let disc = gametdb_pic("wii", &gametdb_kinds("wii", ArtSlot::Icon)[0], "US", "GALE01");
        assert_eq!(disc.url, "https://art.gametdb.com/wii/disc/US/GALE01.png");
        let cart = gametdb_pic("3ds", &gametdb_kinds("3ds", ArtSlot::Icon)[0], "US", "AMKE");
        assert_eq!(cart.url, "https://art.gametdb.com/3ds/cart/US/AMKE.png");
        let ds: Vec<&str> = gametdb_kinds("ds", ArtSlot::Cover).iter().map(|k| k.0).collect();
        assert_eq!(ds, ["coverHQ", "box", "coverfullHQ"]);
        assert!(gametdb_kinds("ds", ArtSlot::Hero).is_empty());
    }

    #[test]
    fn region_fallback_order() {
        // The ID's own region, then the user's, then US → EN → JA.
        assert_eq!(regions("RMGE01", Some("DE")), ["US", "DE", "EN", "JA"]);
        assert_eq!(regions("RMGJ01", None), ["JA", "US", "EN"]);
        // A European disc: the user's European language first, else English.
        assert_eq!(regions("RMGP01", Some("FR")), ["FR", "EN", "US", "JA"]);
        assert_eq!(regions("RMGP01", Some("US")), ["EN", "US", "JA"]);
        assert_eq!(regions("RMGP01", None), ["EN", "US", "JA"]);
        assert_eq!(regions("AMKD", Some("EN")), ["DE", "EN", "US", "JA"]);
        // "A" (every region) and odd letters: the user's region, then the defaults.
        assert_eq!(regions("BNEA", Some("JA")), ["JA", "US", "EN"]);
        assert_eq!(regions("BNEA", None), ["US", "EN", "JA"]);
        assert_eq!(regions("", None), ["US", "EN", "JA"]);
    }

    #[test]
    fn locales_map_to_regions() {
        assert_eq!(locale_region("en_US.UTF-8"), Some("US"));
        assert_eq!(locale_region("en_GB.UTF-8"), Some("EN"));
        assert_eq!(locale_region("de_AT.UTF-8@euro"), Some("DE"));
        assert_eq!(locale_region("pt_BR"), Some("PT"));
        assert_eq!(locale_region("ja_JP.UTF-8"), Some("JA"));
        assert_eq!(locale_region("zh_TW"), Some("ZHTW"));
        assert_eq!(locale_region("sv_SE"), Some("SE"));
        assert_eq!(locale_region("C"), None);
    }
}
