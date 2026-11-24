//! Artwork sources: URLs, response parsing, name matching (pure; unit-tested with fixtures).
//!
//! A source turns (Steam appid | GOG id | name [+ console platform], slot) into image URLs:
//!   * Steam store (no key): appids by name via `storesearch`; per app the asset file names
//!     from `IStoreBrowseService/GetItems` (handles the hashed paths newer apps use), falling
//!     back to the classic CDN paths and `appdetails`' `header_image`.
//!   * SteamGridDB (user key): `search/autocomplete` → game ids; grids/heroes/logos/icons.
//!   * GOG, Lutris.net, PCGamingWiki, libretro-thumbnails (no key) and TheGamesDB, IGDB (the
//!     user's own credentials): see `art/sources/more.rs` / `igdb.rs`; GameTDB: `gametdb.rs`.

use super::ArtSlot;
use crate::art::net::encode;
use crate::config::MetadataConfig;
use serde_json::Value;
use std::collections::HashMap;

mod more;
pub use more::*;
mod gametdb;
pub use gametdb::*;

pub const SRC_STEAM: &str = "Steam";
pub const SRC_SGDB: &str = "SteamGridDB";

pub const STEAM_ASSETS: &str = "https://shared.steamstatic.com/store_item_assets/";
pub const STEAM_CDN: &str = "https://shared.steamstatic.com/store_item_assets/steam/apps";
pub const STEAM_ICON_CDN: &str = "https://cdn.cloudflare.steamstatic.com/steamcommunity/public/images/apps";
pub const STORE_SEARCH: &str = "https://store.steampowered.com/api/storesearch/";
pub const STORE_APPDETAILS: &str = "https://store.steampowered.com/api/appdetails";
pub const GET_ITEMS: &str = "https://api.steampowered.com/IStoreBrowseService/GetItems/v1/";
pub const SGDB_API: &str = "https://www.steamgriddb.com/api/v2";

/// How many name-search hits are looked at per source.
pub const TOP_HITS: usize = 3;
/// Most SteamGridDB images offered per game / per search.
pub const SGDB_PER_GAME: usize = 30;
pub const SGDB_PER_SEARCH: usize = 60;
/// GetItems ids per request in bulk runs.
pub const GET_ITEMS_CHUNK: usize = 30;

/// An online artwork source. `ALL` is the built-in priority order (Settings → Metadata can
/// reorder and switch them off: `MetadataConfig::source_order` / `sources_off`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SourceKind {
    Steam,
    SteamGridDb,
    Gog,
    Igdb,
    TheGamesDb,
    Libretro,
    Lutris,
    PcGamingWiki,
    GameTdb,
}

impl SourceKind {
    pub const ALL: [SourceKind; 9] = [
        SourceKind::Steam,
        SourceKind::SteamGridDb,
        SourceKind::Gog,
        SourceKind::Igdb,
        SourceKind::TheGamesDb,
        SourceKind::Libretro,
        SourceKind::Lutris,
        SourceKind::PcGamingWiki,
        SourceKind::GameTdb,
    ];
    pub fn name(self) -> &'static str {
        match self {
            SourceKind::Steam => SRC_STEAM,
            SourceKind::SteamGridDb => SRC_SGDB,
            SourceKind::Gog => SRC_GOG,
            SourceKind::Igdb => SRC_IGDB,
            SourceKind::TheGamesDb => SRC_TGDB,
            SourceKind::Libretro => SRC_LIBRETRO,
            SourceKind::Lutris => SRC_LUTRIS,
            SourceKind::PcGamingWiki => SRC_PCGW,
            SourceKind::GameTdb => SRC_GAMETDB,
        }
    }
    pub fn from_name(s: &str) -> Option<SourceKind> {
        SourceKind::ALL.into_iter().find(|k| k.name().eq_ignore_ascii_case(s.trim()))
    }
    /// One line for Settings: what it has and what it needs.
    pub fn blurb(self) -> &'static str {
        match self {
            SourceKind::Steam => "Covers, backgrounds, logos, banners and icons of Steam games",
            SourceKind::SteamGridDb => "Community art for almost any game (free API key)",
            SourceKind::Gog => "Box art, backgrounds, logos and icons of GOG games",
            SourceKind::Igdb => "Covers and artwork, plus descriptions and more (Twitch app)",
            SourceKind::TheGamesDb => "Box art, fan art, logos and banners (API key)",
            SourceKind::Libretro => "Box art and screenshots of console games",
            SourceKind::Lutris => "Covers, banners and icons from lutris.net",
            SourceKind::PcGamingWiki => "Covers of PC games from pcgamingwiki.com",
            SourceKind::GameTdb => "Covers and disc art of Wii, GameCube, Wii U, DS and 3DS games",
        }
    }
    /// Needs the user's own credentials.
    pub fn keyed(self) -> bool {
        matches!(self, SourceKind::SteamGridDb | SourceKind::Igdb | SourceKind::TheGamesDb)
    }
    /// The slots this source can have pictures for.
    pub fn slots(self) -> &'static [ArtSlot] {
        use ArtSlot::*;
        match self {
            SourceKind::Steam | SourceKind::SteamGridDb | SourceKind::Gog => &[Cover, Hero, Logo, Icon, Banner],
            SourceKind::Igdb => &[Cover, Hero],
            SourceKind::TheGamesDb => &[Cover, Hero, Logo, Banner],
            SourceKind::Libretro => &[Cover, Hero],
            SourceKind::Lutris => &[Cover, Icon, Banner],
            SourceKind::PcGamingWiki => &[Cover],
            SourceKind::GameTdb => &[Cover, Icon],
        }
    }
    pub fn has_slot(self, slot: ArtSlot) -> bool {
        self.slots().contains(&slot)
    }
    /// Bit in the automatic fill's "already asked" mask (stable: stored in the DB).
    pub fn bit(self) -> u16 {
        match self {
            SourceKind::Steam => 1,
            SourceKind::SteamGridDb => 2,
            SourceKind::Gog => 4,
            SourceKind::Igdb => 8,
            // 16 belonged to a removed source: old masks may still have it set, so it stays unused.
            SourceKind::TheGamesDb => 32,
            SourceKind::Libretro => 64,
            SourceKind::Lutris => 128,
            SourceKind::PcGamingWiki => 256,
            SourceKind::GameTdb => 512,
        }
    }
    /// Has its credentials in `cfg` (always true for the no-key sources).
    pub fn configured(self, cfg: &MetadataConfig) -> bool {
        use crate::config::secret;
        match self {
            SourceKind::SteamGridDb => secret(&cfg.steamgriddb_key).is_some(),
            SourceKind::Igdb => cfg.igdb_credentials().is_some(),
            SourceKind::TheGamesDb => secret(&cfg.thegamesdb_key).is_some(),
            _ => true,
        }
    }
    /// What to set up in Settings → Metadata when not `configured`.
    pub fn needs(self) -> &'static str {
        match self {
            SourceKind::SteamGridDb => "Needs a SteamGridDB API key in Settings → Metadata",
            SourceKind::Igdb => "Needs a Twitch client ID + secret in Settings → Metadata",
            SourceKind::TheGamesDb => "Needs a TheGamesDB API key in Settings → Metadata",
            _ => "",
        }
    }
}

/// Which of the two artwork source lists of Settings → Metadata a game uses.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ArtList {
    /// PC games (every store / launcher game).
    #[default]
    Pc,
    /// Emulated games and games on a console platform.
    Emulated,
}

impl ArtList {
    pub const ALL: [ArtList; 2] = [ArtList::Pc, ArtList::Emulated];
    pub fn label(self) -> &'static str {
        match self {
            ArtList::Pc => "PC games",
            ArtList::Emulated => "Emulated games",
        }
    }
    /// The built-in priority order of this list.
    pub fn default_order(self) -> &'static [SourceKind] {
        match self {
            ArtList::Pc => &PC_ORDER,
            ArtList::Emulated => &EMU_ORDER,
        }
    }
    /// The user's order (names) of this list.
    pub fn order(self, cfg: &MetadataConfig) -> &[String] {
        match self {
            ArtList::Pc => &cfg.source_order,
            ArtList::Emulated => &cfg.emu_source_order,
        }
    }
    pub fn order_mut(self, cfg: &mut MetadataConfig) -> &mut Vec<String> {
        match self {
            ArtList::Pc => &mut cfg.source_order,
            ArtList::Emulated => &mut cfg.emu_source_order,
        }
    }
    /// Names switched off in this list (the built-in choice when the user never changed it).
    pub fn off(self, cfg: &MetadataConfig) -> Vec<String> {
        match self {
            ArtList::Pc => cfg.sources_off.clone(),
            ArtList::Emulated => match &cfg.emu_sources_off {
                Some(v) => v.clone(),
                None => EMU_OFF.iter().map(|k| k.name().to_string()).collect(),
            },
        }
    }
    pub fn set_off(self, cfg: &mut MetadataConfig, off: Vec<String>) {
        match self {
            ArtList::Pc => cfg.sources_off = off,
            ArtList::Emulated => cfg.emu_sources_off = Some(off),
        }
    }
}

/// The built-in order for PC games (every source but the console-only GameTDB).
pub const PC_ORDER: [SourceKind; 8] = [
    SourceKind::Steam,
    SourceKind::SteamGridDb,
    SourceKind::Gog,
    SourceKind::Igdb,
    SourceKind::TheGamesDb,
    SourceKind::Libretro,
    SourceKind::Lutris,
    SourceKind::PcGamingWiki,
];

/// The built-in order for emulated / console games: console art first.
pub const EMU_ORDER: [SourceKind; 9] = [
    SourceKind::Libretro,
    SourceKind::GameTdb,
    SourceKind::SteamGridDb,
    SourceKind::Igdb,
    SourceKind::TheGamesDb,
    SourceKind::Steam,
    SourceKind::Gog,
    SourceKind::PcGamingWiki,
    SourceKind::Lutris,
];
/// PC-only sources, off for emulated games until the user switches them on.
pub const EMU_OFF: [SourceKind; 4] = [SourceKind::Steam, SourceKind::Gog, SourceKind::PcGamingWiki, SourceKind::Lutris];

/// The list a game's artwork comes from: emulated games and games on a console platform use
/// the "Emulated games" list.
pub fn art_list(g: &crate::model::Game) -> ArtList {
    if g.emu.is_some() || console_platform(art_platform(g).as_deref()).is_some() {
        ArtList::Emulated
    } else {
        ArtList::Pc
    }
}

/// Every source of the PC list in the user's priority order, with whether it is switched on.
#[cfg(test)]
pub fn ordered_sources(cfg: &MetadataConfig) -> Vec<(SourceKind, bool)> {
    ordered_sources_for(cfg, ArtList::Pc)
}

/// Every source of `list` in the user's priority order, with whether it is switched on.
pub fn ordered_sources_for(cfg: &MetadataConfig, list: ArtList) -> Vec<(SourceKind, bool)> {
    let mut out: Vec<SourceKind> = vec![];
    for n in list.order(cfg) {
        if let Some(k) = SourceKind::from_name(n)
            && !out.contains(&k)
        {
            out.push(k);
        }
    }
    // Sources the list doesn't name (new ones, hand edits) follow in their built-in order.
    for k in list.default_order() {
        if !out.contains(k) {
            out.push(*k);
        }
    }
    let off = list.off(cfg);
    out.into_iter()
        .map(|k| {
            let is_off = off.iter().any(|n| n.eq_ignore_ascii_case(k.name()));
            (k, !is_off)
        })
        .collect()
}

/// A real Steam store appid (non-Steam shortcuts have the top bit set).
pub fn store_appid(appid: u32) -> Option<u32> {
    (appid != 0 && appid & 0x8000_0000 == 0).then_some(appid)
}

// ---------------------------------------------------------------------------------------------
// Name matching

/// Name normalised for comparison: lowercase, ™®© dropped, `&` → "and", bracketed suffixes
/// ("(2019)", "[GOG]") removed, punctuation → spaces, whitespace collapsed.
pub fn norm_name(s: &str) -> String {
    let s = crate::metadata::parse::search_name(s);
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '&' {
            out.push_str(" and ");
        } else if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if c == '\'' || c == '’' {
            // "Assassin's" == "Assassins"
        } else {
            out.push(' ');
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A hit counts as the same game only when the normalised names are equal.
pub fn same_name(a: &str, b: &str) -> bool {
    let (a, b) = (norm_name(a), norm_name(b));
    !a.is_empty() && a == b
}

/// The confident Steam match for `name` among search hits.
pub fn confident_steam(name: &str, hits: &[StoreHit]) -> Option<u32> {
    hits.iter().find(|h| same_name(name, &h.name)).map(|h| h.appid)
}

pub fn confident_sgdb(name: &str, games: &[SgdbGame]) -> Option<u64> {
    games.iter().find(|g| same_name(name, &g.name)).map(|g| g.id)
}

// ---------------------------------------------------------------------------------------------
// Steam store

#[derive(Clone, Debug, PartialEq)]
pub struct StoreHit {
    pub appid: u32,
    pub name: String,
}

pub fn storesearch_url(term: &str) -> String {
    format!("{STORE_SEARCH}?term={}&l=english&cc=US", encode(term.trim()))
}

/// `storesearch` → apps (DLC/bundles/etc. skipped). None = malformed.
pub fn parse_storesearch(body: &[u8]) -> Option<Vec<StoreHit>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let items = v.get("items")?.as_array()?;
    Some(
        items
            .iter()
            .filter(|i| i.get("type").and_then(Value::as_str).is_none_or(|t| t == "app"))
            .filter_map(|i| {
                let appid = u32::try_from(i.get("id")?.as_u64()?).ok().and_then(store_appid)?;
                let name = i.get("name")?.as_str()?.trim().to_string();
                Some(StoreHit { appid, name })
            })
            .collect(),
    )
}

/// One app's asset file names from `GetItems`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SteamAssets {
    pub appid: u32,
    pub name: String,
    /// URL prefix the file names are appended to.
    pub base: String,
    /// key (`library_capsule_2x`, `header` …) → file name (possibly `<hash>/<file>`).
    pub files: HashMap<String, String>,
    /// Hash of the community icon (jpg on the community CDN).
    pub community_icon: Option<String>,
}

impl SteamAssets {
    pub fn url(&self, key: &str) -> Option<String> {
        self.files.get(key).map(|f| format!("{}{f}", self.base))
    }
}

pub fn get_items_url(appids: &[u32]) -> String {
    let ids: Vec<String> = appids.iter().map(|a| format!("{{\"appid\":{a}}}")).collect();
    let json = format!(
        "{{\"ids\":[{}],\"context\":{{\"language\":\"english\",\"country_code\":\"US\"}},\"data_request\":{{\"include_assets\":true}}}}",
        ids.join(",")
    );
    format!("{GET_ITEMS}?input_json={}", encode(&json))
}

fn safe_file(f: &str) -> bool {
    !f.is_empty()
        && !f.contains("..")
        && !f.starts_with('/')
        && f.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.'))
}

/// `GetItems` → assets per requested appid (`None` = unknown / not visible app).
/// Returns None when the response is malformed.
pub fn parse_get_items(body: &[u8]) -> Option<HashMap<u32, Option<SteamAssets>>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let items = v.get("response")?.get("store_items")?.as_array()?;
    let mut out = HashMap::new();
    for it in items {
        let Some(id) = it.get("id").and_then(Value::as_u64).and_then(|i| u32::try_from(i).ok()) else {
            continue;
        };
        if it.get("success").and_then(Value::as_u64) != Some(1) {
            out.insert(id, None);
            continue;
        }
        let Some(assets) = it.get("assets").filter(|a| a.is_object()) else {
            out.insert(id, None);
            continue;
        };
        // "steam/apps/<id>/${FILENAME}?t=…" → prefix
        let base = assets
            .get("asset_url_format")
            .and_then(Value::as_str)
            .and_then(|f| f.split_once("${FILENAME}").map(|(pre, _)| pre))
            .filter(|pre| pre.starts_with("steam/") && safe_file(pre.trim_end_matches('/')))
            .map(|pre| format!("{STEAM_ASSETS}{pre}"))
            .unwrap_or_else(|| format!("{STEAM_CDN}/{id}/"));
        let mut files = HashMap::new();
        if let Some(obj) = assets.as_object() {
            for (k, v) in obj {
                if k == "asset_url_format" || k == "community_icon" || k.ends_with("_path") {
                    continue;
                }
                if let Some(f) = v.as_str().filter(|f| safe_file(f) && f.contains('.')) {
                    files.insert(k.clone(), f.to_string());
                }
            }
        }
        let community_icon = assets
            .get("community_icon")
            .and_then(Value::as_str)
            .filter(|h| !h.is_empty() && h.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(str::to_string);
        let name = it.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string();
        out.insert(id, Some(SteamAssets { appid: id, name, base, files, community_icon }));
    }
    Some(out)
}

pub fn appdetails_url(appid: u32) -> String {
    format!("{STORE_APPDETAILS}?appids={appid}&l=english")
}

/// `appdetails` → `header_image` without its cache-busting query.
/// None = malformed / rate limited (`null`), Some(None) = no such app or no image.
pub fn parse_appdetails_header(body: &[u8], appid: u32) -> Option<Option<String>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    let e = v.get(appid.to_string())?;
    match e.get("success").and_then(Value::as_bool)? {
        false => Some(None),
        true => Some(
            e.get("data")
                .and_then(|d| d.get("header_image"))
                .and_then(Value::as_str)
                .filter(|u| u.starts_with("https://"))
                .map(strip_query),
        ),
    }
}

pub fn strip_query(u: &str) -> String {
    u.split(['?', '#']).next().unwrap_or(u).to_string()
}

/// One way to get a slot's picture from Steam.
#[derive(Clone, Debug, PartialEq)]
pub struct UrlOpt {
    pub url: String,
    /// A smaller copy of the same picture (only when known to exist).
    pub thumb: Option<String>,
    pub width: u32,
    pub height: u32,
}

/// Alternative URLs for the Steam picture of `slot`, best first (they show the same image;
/// the first that exists is used). `assets` from GetItems when known.
pub fn steam_urls(appid: u32, assets: Option<&SteamAssets>, slot: ArtSlot) -> Vec<UrlOpt> {
    let plain = |f: &str| format!("{STEAM_CDN}/{appid}/{f}");
    let opt = |url: String, thumb: Option<String>, w, h| UrlOpt { url, thumb, width: w, height: h };
    let a = |k: &str| assets.and_then(|s| s.url(k));
    let mut v: Vec<UrlOpt> = vec![];
    match slot {
        ArtSlot::Cover => {
            if let Some(u) = a("library_capsule_2x") {
                v.push(opt(u, a("library_capsule"), 600, 900));
            }
            if let Some(u) = a("library_capsule") {
                v.push(opt(u, None, 300, 450));
            }
            v.push(opt(plain("library_600x900_2x.jpg"), None, 600, 900));
            v.push(opt(plain("library_600x900.jpg"), None, 300, 450));
        }
        ArtSlot::Hero => {
            if let Some(u) = a("library_hero_2x") {
                v.push(opt(u, a("library_hero"), 3840, 1240));
            }
            if let Some(u) = a("library_hero") {
                v.push(opt(u, None, 1920, 620));
            }
            v.push(opt(plain("library_hero.jpg"), None, 1920, 620));
        }
        ArtSlot::Logo => {
            for k in ["library_logo_2x", "library_logo", "logo_2x", "logo"] {
                if let Some(u) = a(k) {
                    v.push(opt(u, None, 0, 0));
                }
            }
            v.push(opt(plain("logo_2x.png"), None, 0, 0));
            v.push(opt(plain("logo.png"), None, 0, 0));
        }
        ArtSlot::Banner => {
            if let Some(u) = a("header_2x") {
                v.push(opt(u, a("header"), 920, 430));
            }
            if let Some(u) = a("header") {
                v.push(opt(u, None, 460, 215));
            }
            v.push(opt(plain("header.jpg"), None, 460, 215));
        }
        ArtSlot::Icon => {
            if let Some(h) = assets.and_then(|s| s.community_icon.as_deref()) {
                v.push(opt(format!("{STEAM_ICON_CDN}/{appid}/{h}.jpg"), None, 32, 32));
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    v.retain(|o| seen.insert(o.url.clone()));
    v
}

// ---------------------------------------------------------------------------------------------
// SteamGridDB

#[derive(Clone, Debug, PartialEq)]
pub struct SgdbGame {
    pub id: u64,
    pub name: String,
}

pub fn sgdb_search_url(term: &str) -> String {
    format!("{SGDB_API}/search/autocomplete/{}", encode(term.trim()))
}

/// `search/autocomplete` → games. None = malformed / `success: false`.
pub fn parse_sgdb_games(body: &[u8]) -> Option<Vec<SgdbGame>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    if v.get("success").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    Some(
        v.get("data")?
            .as_array()?
            .iter()
            .filter_map(|g| {
                Some(SgdbGame {
                    id: g.get("id")?.as_u64()?,
                    name: g.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string(),
                })
            })
            .collect(),
    )
}

/// `target` is `game/<sgdb id>` or `steam/<appid>`.
pub fn sgdb_assets_url(target: &str, slot: ArtSlot) -> String {
    let (kind, query) = match slot {
        ArtSlot::Cover => ("grids", "dimensions=600x900,342x482,660x930&types=static&nsfw=false"),
        ArtSlot::Banner => ("grids", "dimensions=920x430,460x215&types=static&nsfw=false"),
        ArtSlot::Hero => ("heroes", "types=static&nsfw=false"),
        ArtSlot::Logo => ("logos", "types=static&nsfw=false"),
        ArtSlot::Icon => ("icons", "types=static&nsfw=false"),
    };
    format!("{SGDB_API}/{kind}/{target}?{query}")
}

#[derive(Clone, Debug, PartialEq)]
pub struct SgdbAsset {
    pub url: String,
    pub thumb: Option<String>,
    pub width: u32,
    pub height: u32,
    pub style: String,
}

/// `grids|heroes|logos|icons/...` → static, SFW images in API order (best scored first).
/// None = malformed / `success: false`.
pub fn parse_sgdb_assets(body: &[u8]) -> Option<Vec<SgdbAsset>> {
    let v: Value = serde_json::from_slice(body).ok()?;
    if v.get("success").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let https = |s: &str| s.starts_with("https://");
    Some(
        v.get("data")?
            .as_array()?
            .iter()
            .filter(|a| a.get("nsfw").and_then(Value::as_bool) != Some(true))
            .filter(|a| a.get("epilepsy").and_then(Value::as_bool) != Some(true))
            .filter_map(|a| {
                let url = a.get("url")?.as_str().filter(|u| https(u))?;
                let lower = url.to_ascii_lowercase();
                if lower.ends_with(".webm") || lower.ends_with(".mp4") {
                    return None;
                }
                let mime = a.get("mime").and_then(Value::as_str).unwrap_or("");
                if mime.starts_with("video/") {
                    return None;
                }
                let dim = |k: &str| a.get(k).and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
                Some(SgdbAsset {
                    url: url.to_string(),
                    thumb: a.get("thumb").and_then(Value::as_str).filter(|t| https(t)).map(str::to_string),
                    width: dim("width"),
                    height: dim("height"),
                    style: a.get("style").and_then(Value::as_str).unwrap_or("").to_string(),
                })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------------------------
// Images

/// Image format of `bytes` as a file extension (raster by magic bytes, SVG by its root tag).
pub fn sniff_ext(bytes: &[u8]) -> Option<&'static str> {
    if let Some(e) = crate::metadata::parse::image_ext(bytes) {
        return Some(e);
    }
    if bytes.starts_with(b"BM") && bytes.len() > 26 {
        return Some("bmp");
    }
    if bytes.starts_with(&[0, 0, 1, 0]) && bytes.len() > 22 {
        return Some("ico");
    }
    if bytes.starts_with(b"qoif") {
        return Some("qoi");
    }
    let head = &bytes[..bytes.len().min(1024)];
    let head = head.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(head);
    let start = head.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(head.len());
    let head = &head[start..];
    if head.starts_with(b"<svg")
        || (head.starts_with(b"<?xml") || head.starts_with(b"<!--")) && head.windows(4).any(|w| w == b"<svg")
    {
        return Some("svg");
    }
    None
}
