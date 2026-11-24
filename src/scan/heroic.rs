//! Heroic Games Launcher scanner (Epic via legendary, GOG, Amazon via nile, sideloaded apps).
//!
//! Config roots: `~/.config/heroic` (native) and
//! `~/.var/app/com.heroicgameslauncher.hgl/config/heroic` (Flatpak). Per root:
//! * Epic:     `legendaryConfig/legendary/installed.json` (or `~/.config/legendary/installed.json`)
//!   — `{ "<app_name>": {app_name, title, install_path, install_size, platform, is_dlc, …} }`;
//!   titles/art from `store_cache/legendary_library.json` (`{"library": [GameInfo…]}`).
//! * GOG:      `gog_store/installed.json` (`{"installed": [{appName, install_path, platform, install_size: "1.2 GiB", is_dlc}]}`)
//!   + `gog_store/library.json` / `store_cache/gog_library.json` (`{"games": [GameInfo…]}`).
//! * Amazon:   `nile_config/nile/installed.json` (`[{id, path, size, version}]`) + `store_cache/nile_library.json`.
//! * Sideload: `sideload_apps/library.json` (`{"games": [GameInfo…]}`).
//! * Playtime: `store/timestamp.json` (`{ "<appName>": {firstPlayed, lastPlayed (ISO, UTC), totalPlayed (minutes)} }`).
//! * Per-game options: `GamesConfig/<appName>.json` (`launcherArgs`, `enviromentOptions`, `wrapperOptions`).
//! * Art: Heroic caches remote images as `images-cache/<sha256 hex of the URL>` (no extension).
//!   Only cached files are used (no network); local paths / `file://` URLs (sideload) are used directly.
//!
//! `source_id` = Heroic's appName (unique across runners), matching `heroic://launch?appName=…`.
//! Source per runner: legendary → `Source::Epic`, gog → `Source::Gog`, nile → `Source::Amazon`,
//! sideload → `Source::Heroic`.
//!
//! Without Heroic (or for games only the CLIs know): Legendary's and nile's own data, see
//! `heroic/cli.rs`. The same appName from Heroic and a CLI is one game; Heroic wins.
//!
//! CONTRACT: `pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>>` (see scan/mod.rs).

use super::ScanContext;
use super::desktop::common;
use crate::model::*;
use anyhow::Context as _;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const FLATPAK_ID: &str = "com.heroicgameslauncher.hgl";

pub mod cli;

/// The store a Heroic runner stands for.
pub fn source_for(runner: &str) -> Source {
    match runner {
        "legendary" => Source::Epic,
        "gog" => Source::Gog,
        "nile" => Source::Amazon,
        _ => Source::Heroic,
    }
}

/// Heroic's runner name for a store source (sideloaded apps: `Source::Heroic`).
pub fn runner_for(source: Source) -> Option<&'static str> {
    Some(match source {
        Source::Epic => "legendary",
        Source::Gog => "gog",
        Source::Amazon => "nile",
        Source::Heroic => "sideload",
        _ => return None,
    })
}

/// Heroic's config roots (native, Flatpak) with the legendary config dir each may fall back to.
fn roots(ctx: &ScanContext) -> [(PathBuf, PathBuf); 2] {
    [
        (ctx.config_home().join("heroic"), ctx.config_home().join("legendary")),
        (ctx.flatpak_app(FLATPAK_ID).join("config/heroic"), ctx.flatpak_app(FLATPAK_ID).join("config/legendary")),
    ]
}

pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    // Store pages are indexed from the same caches; let the next lookup re-read them.
    crate::store_app::invalidate_store_urls();
    let mut out = vec![];
    let mut seen = HashSet::new();
    for (heroic, legendary) in roots(ctx) {
        if !heroic.is_dir() {
            continue;
        }
        for g in scan_root(ctx, &heroic, &legendary)? {
            if seen.insert(g.source_id.clone()) {
                out.push(g);
            }
        }
    }
    // The CLIs' own data: only games Heroic didn't report (Heroic wins).
    for g in cli::scan(ctx)? {
        if seen.insert(g.source_id.clone()) {
            out.push(g);
        }
    }
    Ok(out)
}

/// Read a JSON file. Missing → Ok(None); unreadable/corrupt → Err.
pub(crate) fn read_json(p: &Path) -> anyhow::Result<Option<Value>> {
    match crate::scan::util::read_text(p) {
        Ok(t) if t.trim().is_empty() => Ok(None),
        Ok(t) => Ok(Some(serde_json::from_str(&t).with_context(|| format!("parsing {}", p.display()))?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", p.display())),
    }
}

/// Optional caches: problems are logged, never fatal.
pub(crate) fn read_json_opt(p: &Path) -> Option<Value> {
    read_json(p).unwrap_or_else(|e| {
        log::warn!("heroic: {e:#}");
        None
    })
}

pub(crate) fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).map(str::trim).unwrap_or("")
}

pub(crate) fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(|x| x.as_bool()).unwrap_or(false)
}

/// Index a Heroic library cache (`{"library": [...]}`, `{"games": [...]}` or a bare array) by app_name.
fn index_library(v: Option<Value>) -> HashMap<String, Value> {
    let list = match v {
        Some(Value::Array(a)) => a,
        Some(Value::Object(mut o)) => match o.remove("library").or_else(|| o.remove("games")) {
            Some(Value::Array(a)) => a,
            _ => vec![],
        },
        _ => vec![],
    };
    list.into_iter()
        .filter_map(|g| {
            let id = s(&g, "app_name").to_string();
            (!id.is_empty()).then_some((id, g))
        })
        .collect()
}

struct Root<'a> {
    ctx: &'a ScanContext,
    dir: &'a Path,
    images: PathBuf,
    times: HashMap<String, (Option<u64>, Option<i64>)>,
}

fn scan_root(ctx: &ScanContext, dir: &Path, legendary_dir: &Path) -> anyhow::Result<Vec<ScannedGame>> {
    let root = Root { ctx, dir, images: dir.join("images-cache"), times: read_timestamps(dir) };
    let mut out = vec![];

    // --- Epic (legendary)
    let installed_path =
        common_first(&[dir.join("legendaryConfig/legendary/installed.json"), legendary_dir.join("installed.json")]);
    let library = index_library(read_json_opt(&dir.join("store_cache/legendary_library.json")));
    let mut installed_ids = HashSet::new();
    if let Some(p) = installed_path
        && let Some(Value::Object(map)) = read_json(&p)?
    {
        for (id, inst) in map {
            if b(&inst, "is_dlc") {
                continue;
            }
            let info = library.get(&id);
            let title = first_nonempty(&[s(&inst, "title"), info.map(|i| s(i, "title")).unwrap_or("")]);
            let mut g = root.game(&id, title, "legendary", info);
            set_install(&mut g, s(&inst, "install_path"));
            g.platform = platform(s(&inst, "platform"));
            g.size_bytes = inst.get("install_size").and_then(Value::as_u64).filter(|&n| n > 0);
            installed_ids.insert(id);
            out.push(g);
        }
    }
    root.add_uninstalled(&library, &installed_ids, "legendary", &mut out);

    // --- GOG
    let gog_lib = {
        let mut m = index_library(read_json_opt(&dir.join("store_cache/gog_library.json")));
        for (k, v) in index_library(read_json_opt(&dir.join("gog_store/library.json"))) {
            m.entry(k).or_insert(v);
        }
        m
    };
    let mut gog_ids = HashSet::new();
    if let Some(v) = read_json(&dir.join("gog_store/installed.json"))? {
        let list = v.get("installed").and_then(Value::as_array).cloned().unwrap_or_default();
        for inst in list {
            let id = s(&inst, "appName").to_string();
            if id.is_empty() || b(&inst, "is_dlc") {
                continue;
            }
            let info = gog_lib.get(&id);
            let title = info.map(|i| s(i, "title")).unwrap_or("");
            let fallback = folder_name(s(&inst, "install_path"));
            let mut g = root.game(&id, first_nonempty(&[title, &fallback]), "gog", info);
            set_install(&mut g, s(&inst, "install_path"));
            g.platform = platform(s(&inst, "platform"));
            g.size_bytes = common::parse_human_size(s(&inst, "install_size")).filter(|&n| n > 0);
            gog_ids.insert(id);
            out.push(g);
        }
    }
    root.add_uninstalled(&gog_lib, &gog_ids, "gog", &mut out);

    // --- Amazon (nile)
    let nile_lib = {
        let mut m = index_library(read_json_opt(&dir.join("store_cache/nile_library.json")));
        for (k, v) in cli::index_nile_library(read_json_opt(&dir.join("nile_config/nile/library.json"))) {
            m.entry(k).or_insert(v);
        }
        m
    };
    let mut nile_ids = HashSet::new();
    if let Some(Value::Array(list)) = read_json(&dir.join("nile_config/nile/installed.json"))? {
        for inst in list {
            let id = s(&inst, "id").to_string();
            if id.is_empty() {
                continue;
            }
            let info = nile_lib.get(&id);
            let fallback = folder_name(s(&inst, "path"));
            let title = first_nonempty(&[info.map(|i| s(i, "title")).unwrap_or(""), &fallback]);
            let mut g = root.game(&id, title, "nile", info);
            set_install(&mut g, s(&inst, "path"));
            g.platform = Some("windows".into());
            g.size_bytes = inst.get("size").and_then(Value::as_u64).filter(|&n| n > 0);
            nile_ids.insert(id);
            out.push(g);
        }
    }
    root.add_uninstalled(&nile_lib, &nile_ids, "nile", &mut out);

    // --- Sideloaded apps
    for (id, info) in index_library(read_json(&dir.join("sideload_apps/library.json"))?) {
        let mut g = root.game(&id, s(&info, "title"), "sideload", Some(&info));
        let install = info.get("install").cloned().unwrap_or(Value::Null);
        let exe = s(&install, "executable");
        let folder = s(&info, "folder_name");
        if !folder.is_empty() {
            set_install(&mut g, folder);
        } else if let Some(parent) = Path::new(exe).parent().filter(|_| exe.starts_with('/')) {
            set_install(&mut g, &parent.to_string_lossy());
        }
        if exe.starts_with('/') {
            g.installed = Path::new(exe).exists();
        }
        g.platform =
            platform(s(&install, "platform")).or_else(|| common::is_windows_exe(exe).then(|| "windows".into()));
        out.push(g);
    }

    out.retain(|g| !g.name.trim().is_empty());
    Ok(out)
}

impl Root<'_> {
    fn game(&self, id: &str, title: &str, runner: &str, info: Option<&Value>) -> ScannedGame {
        let mut g = ScannedGame::new(source_for(runner), id, title);
        g.launch = Some(LaunchSpec::Url(launch_url(id, runner)));
        if let Some(&(pt, lp)) = self.times.get(id) {
            g.playtime_secs = pt;
            g.last_played = lp;
        }
        if let Some(info) = info {
            self.fill_info(&mut g, info);
        }
        g.source_launch_options = read_game_options(self.dir, id);
        g
    }

    fn fill_info(&self, g: &mut ScannedGame, info: &Value) {
        let art = |k: &str| self.art(s(info, k));
        g.art.cover = art("art_square");
        g.art.banner = art("art_cover");
        g.art.hero = art("art_background").or_else(|| g.art.banner.clone());
        g.art.logo = art("art_logo");
        g.art.icon = art("art_icon");
        let dev = s(info, "developer");
        if !dev.is_empty() {
            g.meta.developer = Some(dev.to_string());
        }
        let desc = info
            .pointer("/extra/about/description")
            .and_then(Value::as_str)
            .or_else(|| info.get("description").and_then(Value::as_str))
            .map(str::trim)
            .unwrap_or("");
        if !desc.is_empty() {
            g.meta.description = Some(desc.to_string());
        }
        if let Some(y) = info.pointer("/extra/releaseDate").and_then(Value::as_str).and_then(|d| d.get(..4)) {
            g.meta.release_year = y.parse().ok();
        }
        if g.platform.is_none()
            && let Some(p) = info.pointer("/install/platform").and_then(Value::as_str)
        {
            g.platform = platform(p);
        }
    }

    /// Owned-but-not-installed library entries (only with `scan.include_uninstalled`).
    fn add_uninstalled(
        &self,
        lib: &HashMap<String, Value>,
        installed: &HashSet<String>,
        runner: &str,
        out: &mut Vec<ScannedGame>,
    ) {
        if !self.ctx.config.include_uninstalled {
            return;
        }
        let mut ids: Vec<&String> = lib.keys().filter(|k| !installed.contains(*k)).collect();
        ids.sort();
        for id in ids {
            let info = &lib[id];
            if b(info, "is_dlc")
                || info.get("install").and_then(|i| i.get("is_dlc")).and_then(Value::as_bool) == Some(true)
            {
                continue;
            }
            let mut g = self.game(id, s(info, "title"), runner, Some(info));
            g.installed = false;
            out.push(g);
        }
    }

    fn art(&self, url: &str) -> Option<PathBuf> {
        if url.is_empty() {
            return None;
        }
        if let Some(p) = common::local_art(url) {
            return Some(p);
        }
        if url.starts_with("http://") || url.starts_with("https://") {
            return super::util::file_if_exists(self.images.join(common::sha256_hex(url.as_bytes())));
        }
        None
    }
}

fn common_first(paths: &[PathBuf]) -> Option<PathBuf> {
    paths.iter().find(|p| p.is_file()).cloned()
}

pub(crate) fn first_nonempty<'a>(v: &[&'a str]) -> &'a str {
    v.iter().copied().find(|s| !s.trim().is_empty()).unwrap_or("")
}

pub(crate) fn folder_name(path: &str) -> String {
    Path::new(path).file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
}

pub(crate) fn set_install(g: &mut ScannedGame, path: &str) {
    if path.starts_with('/') {
        let p = PathBuf::from(path);
        g.installed = p.exists();
        g.install_dir = Some(p);
    }
}

pub(crate) fn platform(p: &str) -> Option<String> {
    let l = p.trim().to_ascii_lowercase();
    Some(
        match l.as_str() {
            "" => return None,
            "windows" | "win" => "windows",
            "linux" | "native" => "linux",
            "mac" | "osx" | "macos" => "mac",
            "browser" => "browser",
            other => other,
        }
        .to_string(),
    )
}

/// `heroic://launch?appName=<id>&runner=<legendary|gog|nile|sideload>`
pub fn launch_url(app_name: &str, runner: &str) -> String {
    format!("heroic://launch?appName={}&runner={}", common::percent_encode(app_name), runner)
}

fn read_timestamps(dir: &Path) -> HashMap<String, (Option<u64>, Option<i64>)> {
    let Some(Value::Object(map)) = read_json_opt(&dir.join("store/timestamp.json")) else {
        return HashMap::new();
    };
    map.into_iter()
        .map(|(id, v)| {
            let mins = v.get("totalPlayed").and_then(|x| x.as_f64()).filter(|m| *m > 0.0);
            let last = v.get("lastPlayed").and_then(Value::as_str).and_then(|t| common::parse_datetime(t, false));
            (id, (mins.map(|m| (m * 60.0) as u64), last))
        })
        .collect()
}

/// Heroic per-game settings as a Steam-style options string:
/// `VAR=value wrapper args %command% <launcherArgs>`.
fn read_game_options(dir: &Path, app_name: &str) -> Option<String> {
    if app_name.contains('/') || app_name.contains("..") {
        return None;
    }
    let v = read_json_opt(&dir.join("GamesConfig").join(format!("{app_name}.json")))?;
    let cfg = v.get(app_name).unwrap_or(&v);
    let mut pre: Vec<String> = vec![];
    for e in cfg.get("enviromentOptions").and_then(Value::as_array).into_iter().flatten() {
        let (k, val) = (s(e, "key"), s(e, "value"));
        if !k.is_empty() {
            pre.push(format!("{k}={}", quote_if_needed(val)));
        }
    }
    for w in cfg.get("wrapperOptions").and_then(Value::as_array).into_iter().flatten() {
        let exe = s(w, "exe");
        if !exe.is_empty() {
            pre.push(quote_if_needed(exe));
            let args = s(w, "args");
            if !args.is_empty() {
                pre.push(args.to_string());
            }
        }
    }
    let post = s(cfg, "launcherArgs");
    match (pre.is_empty(), post.is_empty()) {
        (true, true) => None,
        (true, false) => Some(post.to_string()),
        (false, true) => Some(format!("{} %command%", pre.join(" "))),
        (false, false) => Some(format!("{} %command% {post}", pre.join(" "))),
    }
}

fn quote_if_needed(s: &str) -> String {
    if s.is_empty() || s.contains(|c: char| c.is_whitespace() || c == '"' || c == '\'') {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Store pages Heroic's library caches know (Epic `extra.storeUrl`, GOG `store_url`) plus the
/// CLIs' (Legendary's product slugs). Read lazily off the UI thread by `store_app`.
pub fn store_urls(ctx: &ScanContext) -> Vec<((Source, String), String)> {
    let mut out = vec![];
    for (dir, _) in roots(ctx) {
        if !dir.is_dir() {
            continue;
        }
        for (id, info) in index_library(read_json_opt(&dir.join("store_cache/legendary_library.json"))) {
            let url = info
                .pointer("/extra/storeUrl")
                .and_then(Value::as_str)
                .or_else(|| info.get("store_url").and_then(Value::as_str))
                .and_then(epic_store_url);
            if let Some(u) = url {
                out.push(((Source::Epic, id), u));
            }
        }
        for file in ["store_cache/gog_library.json", "gog_store/library.json"] {
            for (id, info) in index_library(read_json_opt(&dir.join(file))) {
                if let Some(u) = gog_store_url(s(&info, "store_url")) {
                    out.push(((Source::Gog, id), u));
                }
            }
        }
    }
    out.extend(cli::store_urls(ctx));
    out
}

/// Path segments of a URL (after the host), without query / fragment.
fn url_segments(url: &str) -> Option<Vec<&str>> {
    let rest = url.trim().split_once("://")?.1;
    let path = rest.split_once('/').map(|(_, p)| p).unwrap_or("");
    let path = path.split(['?', '#']).next().unwrap_or("");
    Some(path.split('/').filter(|x| !x.is_empty()).collect())
}

fn valid_slug(s: &str) -> bool {
    s.len() <= 200
        && s.starts_with(|c: char| c.is_ascii_alphanumeric())
        && !s.contains("..")
        && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Epic store page from a store URL or product slug (`https://www.epicgames.com/store/en-US/product/
/// fortnite/home`, `https://store.epicgames.com/p/fortnite`, `fortnite/home`) →
/// `https://store.epicgames.com/p/<slug>`.
pub fn epic_store_url(s: &str) -> Option<String> {
    let s = s.trim();
    let slug = if s.contains("://") {
        let segs = url_segments(s)?;
        let i = segs.iter().position(|x| *x == "p" || *x == "product")?;
        segs.get(i + 1)?.to_string()
    } else {
        s.split('/').next().unwrap_or("").to_string()
    };
    valid_slug(&slug).then(|| format!("https://store.epicgames.com/p/{slug}"))
}

/// GOG store page from a store URL (`https://www.gog.com/en/game/cyberpunk_2077`) or a slug →
/// `https://www.gog.com/game/<slug>`.
pub fn gog_store_url(s: &str) -> Option<String> {
    let s = s.trim();
    let slug = if s.contains("://") {
        let segs = url_segments(s)?;
        let i = segs.iter().position(|x| *x == "game")?;
        segs.get(i + 1)?.to_string()
    } else {
        s.to_string()
    };
    valid_slug(&slug).then(|| format!("https://www.gog.com/game/{slug}"))
}

#[cfg(test)]
mod tests {
    use super::super::desktop::test_support::*;
    use super::*;

    fn setup(home: &Path) -> PathBuf {
        let heroic = home.join(".config/heroic");
        copy_tree(&fixture("heroic/heroic"), &heroic);
        // Fixture paths use /HOME; make them real.
        for rel in [
            "legendaryConfig/legendary/installed.json",
            "gog_store/installed.json",
            "nile_config/nile/installed.json",
            "sideload_apps/library.json",
        ] {
            let p = heroic.join(rel);
            let t = std::fs::read_to_string(&p).unwrap();
            std::fs::write(&p, t.replace("/HOME", &home.to_string_lossy())).unwrap();
        }
        for d in ["Games/Heroic/Fortnite", "Games/Heroic/Cyberpunk 2077", "Games/Heroic/Tomb Raider"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
        std::fs::create_dir_all(home.join("Games/itchy")).unwrap();
        std::fs::write(home.join("Games/itchy/run.sh"), "#!/bin/sh").unwrap();
        // Heroic's image cache: sha256(url).
        let cache = heroic.join("images-cache");
        fake_png(&cache.join(sha256_of("https://cdn1.epicgames.com/offer/fn/FNBR_Tall_1200x1600.jpg")), 600, 900);
        fake_png(&cache.join(sha256_of("https://cdn1.epicgames.com/offer/fn/FNBR_Wide_2560x1440.jpg")), 1920, 1080);
        fake_png(&cache.join(sha256_of("https://images.gog-statics.com/c75e674590b8947542c809924df30bbef2190341163dd08668e243c266be70c5_glx_vertical_cover.webp")), 342, 482);
        fake_png(&home.join("Pictures/itchy.png"), 300, 400);
        heroic
    }

    fn sha256_of(s: &str) -> String {
        common::sha256_hex(s.as_bytes())
    }

    #[test]
    fn parses_fixture() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let heroic = setup(home);
        let mut games = scan(&ctx(home)).unwrap();
        games.sort_by(|a, b| a.source_id.cmp(&b.source_id));
        let ids: Vec<&str> = games.iter().map(|g| g.source_id.as_str()).collect();
        assert_eq!(ids, vec!["1423049311", "Fortnite", "amzn1.adg.product.tombraider", "sl_itchy"], "{games:#?}");

        let sources: Vec<Source> = games.iter().map(|g| g.source).collect();
        assert_eq!(sources, vec![Source::Gog, Source::Epic, Source::Amazon, Source::Heroic]);

        let fnbr = &games[1];
        assert_eq!(fnbr.name, "Fortnite");
        assert!(fnbr.installed);
        assert_eq!(fnbr.install_dir, Some(home.join("Games/Heroic/Fortnite")));
        assert_eq!(fnbr.platform.as_deref(), Some("windows"));
        assert_eq!(fnbr.size_bytes, Some(31_457_280_000));
        assert_eq!(fnbr.launch, Some(LaunchSpec::Url("heroic://launch?appName=Fortnite&runner=legendary".into())));
        assert_eq!(fnbr.playtime_secs, Some(125 * 60));
        assert_eq!(fnbr.last_played, Some(1_682_971_200));
        assert_eq!(fnbr.meta.developer.as_deref(), Some("Epic Games"));
        assert_eq!(
            fnbr.art.cover,
            Some(
                heroic
                    .join("images-cache")
                    .join(sha256_of("https://cdn1.epicgames.com/offer/fn/FNBR_Tall_1200x1600.jpg"))
            )
        );
        assert!(fnbr.art.banner.is_some() && fnbr.art.hero == fnbr.art.banner);
        assert_eq!(fnbr.art.logo, None, "logo not cached");
        assert_eq!(
            fnbr.source_launch_options.as_deref(),
            Some("DXVK_HUD=fps PROTON_LOG_DIR=\"/tmp/a b\" gamemoderun mangohud --dlsym %command% -nosplash")
        );

        let cp = &games[0];
        assert_eq!(cp.name, "Cyberpunk 2077");
        assert_eq!(cp.launch, Some(LaunchSpec::Url("heroic://launch?appName=1423049311&runner=gog".into())));
        assert!(cp.art.cover.is_some());
        assert_eq!(cp.size_bytes, Some(67_108_864_000));
        assert_eq!(cp.platform.as_deref(), Some("windows"));
        assert_eq!(cp.meta.release_year, Some(2020));

        let tr = &games[2];
        assert_eq!(tr.name, "Tomb Raider");
        assert_eq!(
            tr.launch,
            Some(LaunchSpec::Url("heroic://launch?appName=amzn1.adg.product.tombraider&runner=nile".into()))
        );
        assert!(tr.installed);

        let sl = &games[3];
        assert_eq!(sl.name, "Itchy Game");
        assert_eq!(sl.platform.as_deref(), Some("linux"));
        assert_eq!(sl.art.cover, Some(home.join("Pictures/itchy.png")));
        assert!(sl.installed);
        assert_eq!(sl.launch, Some(LaunchSpec::Url("heroic://launch?appName=sl_itchy&runner=sideload".into())));
    }

    #[test]
    fn uninstalled_when_enabled() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        setup(home);
        let mut c = ctx(home);
        c.config.include_uninstalled = true;
        let games = scan(&c).unwrap();
        let owned = games.iter().find(|g| g.source_id == "Hades").expect("owned epic game");
        assert!(!owned.installed);
        assert!(games.iter().all(|g| g.source_id != "dlc_thing"));
    }

    /// The same appName from Heroic and the Legendary CLI is one game, Heroic's; CLI-only games
    /// launch through the CLI.
    #[test]
    fn heroic_wins_over_cli_data() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        setup(home);
        let legendary = home.join(".config/legendary");
        std::fs::create_dir_all(legendary.join("metadata")).unwrap();
        let fortnite = home.join("Games/Heroic/Fortnite").to_string_lossy().into_owned();
        let only_cli = home.join("Games/cli/Other").to_string_lossy().into_owned();
        std::fs::create_dir_all(&only_cli).unwrap();
        std::fs::write(
            legendary.join("installed.json"),
            serde_json::json!({
                "Fortnite": {"app_name": "Fortnite", "title": "Fortnite (CLI)", "install_path": fortnite, "platform": "Windows"},
                "Other": {"app_name": "Other", "title": "Other Game", "install_path": only_cli, "platform": "Windows"}
            })
            .to_string(),
        )
        .unwrap();
        let games = scan(&ctx(home)).unwrap();
        let fnbr: Vec<&ScannedGame> = games.iter().filter(|g| g.source_id == "Fortnite").collect();
        assert_eq!(fnbr.len(), 1);
        assert_eq!(fnbr[0].name, "Fortnite");
        assert!(matches!(&fnbr[0].launch, Some(LaunchSpec::Url(u)) if u.starts_with("heroic://")));
        let other = games.iter().find(|g| g.source_id == "Other").expect("CLI-only game");
        assert_eq!(other.source, Source::Epic);
        assert!(matches!(&other.launch, Some(LaunchSpec::Command { program, .. }) if program == "legendary"));
    }

    #[test]
    fn store_pages_from_caches() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        setup(home);
        let urls: HashMap<(Source, String), String> = store_urls(&ctx(home)).into_iter().collect();
        assert_eq!(
            urls.get(&(Source::Epic, "Fortnite".into())).map(String::as_str),
            Some("https://store.epicgames.com/p/fortnite")
        );
        assert_eq!(
            urls.get(&(Source::Gog, "1423049311".into())).map(String::as_str),
            Some("https://www.gog.com/game/cyberpunk_2077")
        );
        assert_eq!(
            epic_store_url("https://store.epicgames.com/en-US/p/hades?x=1").as_deref(),
            Some("https://store.epicgames.com/p/hades")
        );
        assert_eq!(epic_store_url("https://www.epicgames.com/store/product/"), None);
        assert_eq!(epic_store_url("../../evil"), None);
        assert_eq!(
            gog_store_url("https://www.gog.com/en/game/the_witcher_3_wild_hunt").as_deref(),
            Some("https://www.gog.com/game/the_witcher_3_wild_hunt")
        );
        assert_eq!(gog_store_url(""), None);
    }

    #[test]
    fn flatpak_root_and_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        assert!(scan(&ctx(home)).unwrap().is_empty());
        let root = home.join(".var/app").join(FLATPAK_ID).join("config/heroic/gog_store");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("installed.json"),
            r#"{"installed":[{"appName":"1","install_path":"/nope/Some Game","platform":"linux"}]}"#,
        )
        .unwrap();
        let g = scan(&ctx(home)).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].name, "Some Game", "falls back to folder name");
        assert!(!g[0].installed);
        std::fs::write(root.join("installed.json"), "{oops").unwrap();
        assert!(scan(&ctx(home)).is_err());
    }
}
