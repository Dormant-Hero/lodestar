//! Legendary's and nile's own data, for people who use the CLIs without Heroic (or games only a
//! CLI knows). Declared in heroic.rs as `heroic::cli`; `heroic::scan` merges the result after
//! Heroic's own games, so the same appName from both is one game and Heroic's wins.
//!
//! * Legendary (Epic): `~/.config/legendary/installed.json` — the same shape as Heroic's copy,
//!   `{ "<app_name>": {app_name, title, install_path, install_size, platform, is_dlc, …} }` — and
//!   `metadata/<app_name>.json`, one per owned asset:
//!   `{app_name, app_title, metadata: {title, developer, description, namespace, categories: [{path}],
//!   customAttributes: {"com.epicgames.app.productSlug": {value}}, mainGameItem (DLC only), …}}`.
//!   Launch: `legendary launch <app_name>`. Owned-but-not-installed = the metadata files
//!   (with `scan.include_uninstalled`), DLC / Unreal Engine assets skipped.
//! * nile (Amazon): `~/.config/nile/installed.json` (`[{id, version, path, size}]`) and
//!   `library.json` (`[{id: <entitlement id>, product: {id, title, productDetail: {details:
//!   {developer, publisher, shortDescription, releaseDate, genres}}}}]`). Launch: `nile launch <id>`.
//! * gogdl keeps no list of its own (Heroic's `gog_store/installed.json` is it), so GOG games come
//!   from Heroic only.
//!
//! Neither CLI caches images, so these games get their art from the metadata fetcher.

use super::{b, first_nonempty, platform, read_json, read_json_opt, s, set_install};
use crate::model::*;
use crate::scan::ScanContext;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Most metadata files read from one folder (a huge or hostile folder must not stall a scan).
const MAX_METADATA_FILES: usize = 20_000;

pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    let mut out = vec![];
    legendary(ctx, &ctx.config_home().join("legendary"), &mut out)?;
    nile(ctx, &ctx.config_home().join("nile"), &mut out)?;
    Ok(out)
}

fn safe_id(id: &str) -> bool {
    !id.is_empty() && !id.contains('/') && !id.contains("..") && !id.contains('\0')
}

fn command(program: &str, args: &[&str]) -> LaunchSpec {
    LaunchSpec::Command {
        program: program.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: None,
        env: vec![],
    }
}

// ---------------------------------------------------------------------------------------------
// Legendary

fn legendary(ctx: &ScanContext, dir: &Path, out: &mut Vec<ScannedGame>) -> anyhow::Result<()> {
    let meta_dir = dir.join("metadata");
    let installed = read_json(&dir.join("installed.json"))?;
    if installed.is_none() && !meta_dir.is_dir() {
        return Ok(());
    }
    let mut ids = HashSet::new();
    if let Some(Value::Object(map)) = installed {
        for (id, inst) in map {
            if b(&inst, "is_dlc") || !safe_id(&id) {
                continue;
            }
            let meta = read_json_opt(&meta_dir.join(format!("{id}.json")));
            let title = first_nonempty(&[s(&inst, "title"), meta.as_ref().map(legendary_title).unwrap_or("")]);
            let mut g = legendary_game(&id, title, meta.as_ref());
            set_install(&mut g, s(&inst, "install_path"));
            g.platform = platform(s(&inst, "platform"));
            g.size_bytes = inst.get("install_size").and_then(Value::as_u64).filter(|&n| n > 0);
            ids.insert(id);
            out.push(g);
        }
    }
    if ctx.config.include_uninstalled {
        for (id, meta) in legendary_metadata(&meta_dir) {
            if ids.contains(&id) || !legendary_is_game(&meta) {
                continue;
            }
            let mut g = legendary_game(&id, legendary_title(&meta), Some(&meta));
            g.installed = false;
            out.push(g);
        }
    }
    Ok(())
}

/// Every `metadata/*.json`, sorted by app name. Unreadable files are skipped (logged).
fn legendary_metadata(meta_dir: &Path) -> Vec<(String, Value)> {
    let Ok(rd) = std::fs::read_dir(meta_dir) else {
        return vec![];
    };
    let mut names: Vec<String> = rd
        .flatten()
        .take(MAX_METADATA_FILES)
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".json")).map(str::to_string))
        .filter(|n| safe_id(n))
        .collect();
    names.sort();
    names
        .into_iter()
        .filter_map(|id| {
            let v = read_json_opt(&meta_dir.join(format!("{id}.json")))?;
            Some((id, v))
        })
        .collect()
}

fn legendary_title(meta: &Value) -> &str {
    let inner = meta.get("metadata").unwrap_or(&Value::Null);
    first_nonempty(&[s(meta, "app_title"), s(inner, "title")])
}

/// Games only: no DLC (`mainGameItem`), add-ons / extras, or Unreal Engine marketplace assets.
fn legendary_is_game(meta: &Value) -> bool {
    let inner = meta.get("metadata").unwrap_or(&Value::Null);
    if inner.get("mainGameItem").is_some_and(|v| !v.is_null()) || s(inner, "namespace") == "ue" {
        return false;
    }
    let cats: Vec<&str> =
        inner.get("categories").and_then(Value::as_array).into_iter().flatten().map(|c| s(c, "path")).collect();
    !cats.iter().any(|c| matches!(*c, "addons" | "digitalextras" | "plugins" | "engines" | "assets"))
}

fn legendary_game(id: &str, title: &str, meta: Option<&Value>) -> ScannedGame {
    let mut g = ScannedGame::new(Source::Epic, id, title);
    g.launch = Some(command("legendary", &["launch", id]));
    if let Some(inner) = meta.and_then(|m| m.get("metadata")) {
        let dev = s(inner, "developer");
        if !dev.is_empty() {
            g.meta.developer = Some(dev.to_string());
        }
        let desc = s(inner, "description");
        if !desc.is_empty() {
            g.meta.description = Some(desc.to_string());
        }
    }
    g
}

/// `customAttributes["com.epicgames.app.productSlug"].value` (e.g. `fortnite/home`) → store page.
fn legendary_store_url(meta: &Value) -> Option<String> {
    let slug = meta.pointer("/metadata/customAttributes/com.epicgames.app.productSlug/value")?.as_str()?;
    super::epic_store_url(slug)
}

// ---------------------------------------------------------------------------------------------
// nile

fn nile(ctx: &ScanContext, dir: &Path, out: &mut Vec<ScannedGame>) -> anyhow::Result<()> {
    let installed = read_json(&dir.join("installed.json"))?;
    let lib_file = dir.join("library.json");
    if installed.is_none() && !lib_file.is_file() {
        return Ok(());
    }
    let lib = index_nile_library(read_json_opt(&lib_file));
    let mut ids = HashSet::new();
    if let Some(Value::Array(list)) = installed {
        for inst in list {
            let id = s(&inst, "id").to_string();
            if !safe_id(&id) {
                continue;
            }
            let info = lib.get(&id);
            let fallback = super::folder_name(s(&inst, "path"));
            let title = first_nonempty(&[info.map(|i| s(i, "title")).unwrap_or(""), &fallback]);
            let mut g = nile_game(&id, title, info);
            set_install(&mut g, s(&inst, "path"));
            g.size_bytes = inst.get("size").and_then(Value::as_u64).filter(|&n| n > 0);
            ids.insert(id);
            out.push(g);
        }
    }
    if ctx.config.include_uninstalled {
        let mut rest: Vec<(&String, &Value)> = lib.iter().filter(|(k, _)| !ids.contains(*k)).collect();
        rest.sort_by(|a, b| a.0.cmp(b.0));
        for (id, info) in rest {
            let mut g = nile_game(id, s(info, "title"), Some(info));
            g.installed = false;
            out.push(g);
        }
    }
    Ok(())
}

fn nile_game(id: &str, title: &str, info: Option<&Value>) -> ScannedGame {
    let mut g = ScannedGame::new(Source::Amazon, id, title);
    g.launch = Some(command("nile", &["launch", id]));
    g.platform = Some("windows".into());
    if let Some(info) = info {
        for (k, slot) in [
            ("developer", &mut g.meta.developer),
            ("publisher", &mut g.meta.publisher),
            ("description", &mut g.meta.description),
        ] {
            let v = s(info, k);
            if !v.is_empty() {
                *slot = Some(v.to_string());
            }
        }
        g.meta.release_year = info
            .pointer("/extra/releaseDate")
            .and_then(Value::as_str)
            .and_then(|d| d.get(..4))
            .and_then(|y| y.parse().ok());
        g.meta.genres = info
            .get("genres")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|x| x.as_str().map(str::trim).filter(|x| !x.is_empty()).map(str::to_string))
            .collect();
    }
    g
}

/// Index nile's `library.json` (entitlements, each with a `product`) by product id, as
/// Heroic-style GameInfo values (`app_name`, `title`, `developer`, `publisher`, `description`,
/// `extra.releaseDate`, `genres`) so Heroic's own reader can use them too.
pub fn index_nile_library(v: Option<Value>) -> HashMap<String, Value> {
    let list = match v {
        Some(Value::Array(a)) => a,
        Some(Value::Object(mut o)) => match o.remove("library").or_else(|| o.remove("games")) {
            Some(Value::Array(a)) => a,
            _ => vec![],
        },
        _ => vec![],
    };
    let mut out = HashMap::new();
    for item in list {
        let product = item.get("product").unwrap_or(&item);
        let id = s(product, "id");
        let title = s(product, "title");
        if !safe_id(id) || title.is_empty() {
            continue;
        }
        let details =
            product.pointer("/productDetail/details").or_else(|| product.get("details")).unwrap_or(&Value::Null);
        let pick = |k: &str| first_nonempty(&[s(details, k), s(product, k)]).to_string();
        let genres: Vec<Value> = details.get("genres").and_then(Value::as_array).cloned().unwrap_or_default();
        out.insert(
            id.to_string(),
            json!({
                "app_name": id,
                "title": title,
                "developer": pick("developer"),
                "publisher": pick("publisher"),
                "description": pick("shortDescription"),
                "extra": { "releaseDate": pick("releaseDate") },
                "genres": genres,
            }),
        );
    }
    out
}

/// Epic store pages from Legendary's metadata (its own and Heroic's copy).
pub fn store_urls(ctx: &ScanContext) -> Vec<((Source, String), String)> {
    let mut dirs = vec![ctx.config_home().join("legendary/metadata")];
    for root in [ctx.config_home().join("heroic"), ctx.flatpak_app(super::FLATPAK_ID).join("config/heroic")] {
        dirs.push(root.join("legendaryConfig/legendary/metadata"));
    }
    let mut out = vec![];
    for d in dirs {
        for (id, meta) in legendary_metadata(&d) {
            if let Some(u) = legendary_store_url(&meta) {
                out.push(((Source::Epic, id), u));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::desktop::test_support::{copy_tree, ctx};
    use std::path::PathBuf;

    fn stores_fixture(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stores").join(rel)
    }

    /// Copy a fixture tree to `dst`, replacing `/HOME` in JSON files with the real home.
    fn install(rel: &str, dst: &Path, home: &Path) {
        copy_tree(&stores_fixture(rel), dst);
        fn fix(dir: &Path, home: &Path) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    fix(&p, home);
                } else if p.extension().is_some_and(|x| x == "json") {
                    let t = std::fs::read_to_string(&p).unwrap();
                    std::fs::write(&p, t.replace("/HOME", &home.to_string_lossy())).unwrap();
                }
            }
        }
        fix(dst, home);
    }

    fn setup(home: &Path) {
        install("legendary", &home.join(".config/legendary"), home);
        install("nile", &home.join(".config/nile"), home);
        for d in ["Games/legendary/RocketLeague", "Games/nile/Hitman"] {
            std::fs::create_dir_all(home.join(d)).unwrap();
        }
    }

    #[test]
    fn legendary_and_nile_cli() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        setup(home);
        let games = scan(&ctx(home)).unwrap();
        let ids: Vec<(Source, &str)> = games.iter().map(|g| (g.source, g.source_id.as_str())).collect();
        assert_eq!(ids, vec![(Source::Epic, "Sugar"), (Source::Amazon, "amzn1.adg.product.hitman")], "{games:#?}");

        let rl = &games[0];
        assert_eq!(rl.name, "Rocket League®");
        assert!(rl.installed);
        assert_eq!(rl.install_dir, Some(home.join("Games/legendary/RocketLeague")));
        assert_eq!(rl.size_bytes, Some(28_991_029_248));
        assert_eq!(rl.platform.as_deref(), Some("windows"));
        assert_eq!(rl.meta.developer.as_deref(), Some("Psyonix LLC"));
        assert_eq!(
            rl.launch,
            Some(LaunchSpec::Command {
                program: "legendary".into(),
                args: vec!["launch".into(), "Sugar".into()],
                cwd: None,
                env: vec![]
            })
        );

        let hm = &games[1];
        assert_eq!(hm.name, "HITMAN World of Assassination");
        assert!(hm.installed);
        assert_eq!(hm.meta.developer.as_deref(), Some("IO Interactive"));
        assert_eq!(hm.meta.release_year, Some(2023));
        assert_eq!(hm.meta.genres, vec!["Action".to_string(), "Stealth".to_string()]);
        assert_eq!(
            hm.launch,
            Some(LaunchSpec::Command {
                program: "nile".into(),
                args: vec!["launch".into(), "amzn1.adg.product.hitman".into()],
                cwd: None,
                env: vec![]
            })
        );
    }

    #[test]
    fn owned_not_installed_skips_dlc_and_assets() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        setup(home);
        let mut c = ctx(home);
        c.config.include_uninstalled = true;
        let games = scan(&c).unwrap();
        let mut ids: Vec<(&str, bool)> = games.iter().map(|g| (g.source_id.as_str(), g.installed)).collect();
        ids.sort();
        assert_eq!(
            ids,
            vec![
                ("Sugar", true),
                ("amzn1.adg.product.fallout76", false),
                ("amzn1.adg.product.hitman", true),
                ("d8a4c98b5020483881eb7f0c3fc4cea3", false),
            ],
            "DLC (mainGameItem), UE assets and add-ons are not games"
        );
        let owned = games.iter().find(|g| g.source_id == "d8a4c98b5020483881eb7f0c3fc4cea3").unwrap();
        assert_eq!(owned.name, "Control");
        assert_eq!(owned.source, Source::Epic);
    }

    #[test]
    fn store_slugs_and_missing_or_corrupt_data() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        assert!(scan(&ctx(home)).unwrap().is_empty(), "no CLI data: nothing, no error");
        setup(home);
        let urls: HashMap<_, _> = store_urls(&ctx(home)).into_iter().collect();
        assert_eq!(
            urls.get(&(Source::Epic, "Sugar".to_string())).map(String::as_str),
            Some("https://store.epicgames.com/p/rocket-league")
        );
        std::fs::write(home.join(".config/nile/installed.json"), "[{oops").unwrap();
        assert!(scan(&ctx(home)).is_err(), "unreadable installed list is an error (keeps the DB state)");
        std::fs::write(home.join(".config/nile/installed.json"), "[]").unwrap();
        std::fs::write(home.join(".config/legendary/metadata/Sugar.json"), "{broken").unwrap();
        let g = scan(&ctx(home)).unwrap();
        assert_eq!(g[0].name, "Rocket League®", "a broken metadata file only costs details");
    }
}
