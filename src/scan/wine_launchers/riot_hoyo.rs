//! Riot Client and HoYoPlay in a Wine prefix.
//!
//! * Riot: `ProgramData/Riot Games/RiotClientInstalls.json`
//!   (`{"rc_default": "C:/Riot Games/Riot Client/RiotClientServices.exe", "associated_client":
//!   {"C:/Riot Games/League of Legends/": "…RiotClientServices.exe"}, …}`) and per product
//!   `ProgramData/Riot Games/Metadata/<product>.<patchline>/<product>.<patchline>.product_settings.yaml`
//!   (`product_install_full_path: "C:/Riot Games/League of Legends"`). Launch:
//!   `RiotClientServices.exe --launch-product=<product> --launch-patchline=<patchline>`.
//!   VALORANT's Vanguard anti-cheat doesn't run under Wine; it is still listed.
//! * HoYoPlay: `user.reg` `Software\Cognosphere\HYP\<ver>\<game_biz>` (global) /
//!   `Software\miHoYo\HYP\<ver>\<game_biz>` (China) → `GameInstallPath`, plus the default
//!   `Program Files/HoYoPlay/games/*` folders. The game's own exe is started directly (HoYoPlay's
//!   `launcher.exe` when the exe isn't found).

use super::{Found, PrefixData};
use crate::model::Source;
use serde_json::Value;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------------
// Riot

const RIOT_PRODUCTS: &[(&str, &str, &[&str])] = &[
    // (product, name, install folder names)
    ("league_of_legends", "League of Legends", &["league of legends"]),
    ("valorant", "VALORANT", &["valorant"]),
    ("bacon", "Legends of Runeterra", &["lor", "legends of runeterra"]),
    ("lion", "2XKO", &["2xko"]),
];

fn installs_json(p: &PrefixData) -> Option<Value> {
    let f = p.path_of("C:\\ProgramData\\Riot Games\\RiotClientInstalls.json")?;
    serde_json::from_str(&crate::scan::util::read_text(f).ok()?).ok()
}

pub fn riot_client(p: &PrefixData) -> Option<PathBuf> {
    let v = installs_json(p)?;
    let mut cands: Vec<&str> =
        ["rc_default", "rc_live", "rc_beta"].iter().filter_map(|k| v.get(*k)?.as_str()).collect();
    if let Some(m) = v.get("associated_client").and_then(Value::as_object) {
        cands.extend(m.values().filter_map(Value::as_str));
    }
    cands.into_iter().filter_map(|c| p.path_of(c)).find(|c| c.is_file())
}

/// `key: "value"` from a product_settings.yaml (top-level scalars only).
fn yaml_scalar(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let rest = l.strip_prefix(key)?.strip_prefix(':')?;
        Some(rest.trim().trim_matches(|c| c == '"' || c == '\'').to_string()).filter(|s| !s.is_empty())
    })
}

pub fn riot(p: &PrefixData) -> Vec<Found> {
    let Some(client) = riot_client(p) else {
        return vec![];
    };
    let mut out: Vec<Found> = vec![];
    let push = |product: &str, patchline: &str, dir: PathBuf, out: &mut Vec<Found>| {
        if product == "riot_client" || product.is_empty() || out.iter().any(|f| f.install_dir == dir) {
            return;
        }
        let name = RIOT_PRODUCTS
            .iter()
            .find(|(id, _, _)| *id == product)
            .map_or_else(|| super::folder_title(&dir), |(_, n, _)| (*n).to_string());
        let id = if patchline == "live" { product.to_string() } else { format!("{product}.{patchline}") };
        out.push(Found {
            source: Source::Riot,
            id,
            name,
            install_dir: dir,
            exe: client.clone(),
            args: vec![format!("--launch-product={product}"), format!("--launch-patchline={patchline}")],
            launch_ids: vec![],
        });
    };
    if let Some(meta) = p.path_of("C:\\ProgramData\\Riot Games\\Metadata") {
        for d in super::dirs_in(&meta) {
            let dn = d.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let (product, patchline) = dn.split_once('.').unwrap_or((dn.as_str(), "live"));
            let Ok(text) = crate::scan::util::read_text(d.join(format!("{dn}.product_settings.yaml"))) else {
                continue;
            };
            let Some(dir) =
                yaml_scalar(&text, "product_install_full_path").and_then(|w| p.path_of(&w)).filter(|d| d.is_dir())
            else {
                continue;
            };
            push(product, patchline, dir, &mut out);
        }
    }
    // Without metadata: the install folders the client is associated with.
    if let Some(m) = installs_json(p).as_ref().and_then(|v| v.get("associated_client")).and_then(Value::as_object) {
        for k in m.keys() {
            let Some(dir) = p.path_of(k).filter(|d| d.is_dir()) else {
                continue;
            };
            let folder = super::folder_title(&dir).to_lowercase();
            if let Some((product, _, _)) = RIOT_PRODUCTS.iter().find(|(_, _, f)| f.contains(&folder.as_str())) {
                push(product, "live", dir, &mut out);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// HoYoPlay

/// (game_biz prefix, name, exe names).
const HOYO_GAMES: &[(&str, &str, &[&str])] = &[
    ("hk4e", "Genshin Impact", &["GenshinImpact.exe", "YuanShen.exe"]),
    ("hkrpg", "Honkai: Star Rail", &["StarRail.exe"]),
    ("nap", "Zenless Zone Zero", &["ZenlessZoneZero.exe"]),
    ("bh3", "Honkai Impact 3rd", &["BH3.exe"]),
];

pub const HOYO_DIRS: &[&str] = &["C:\\Program Files\\HoYoPlay", "C:\\Program Files\\miHoYo Launcher"];

pub fn hoyo_launcher(p: &PrefixData) -> Option<PathBuf> {
    HOYO_DIRS.iter().filter_map(|d| p.path_of(&format!("{d}\\launcher.exe"))).find(|f| f.is_file())
}

fn hoyo_exe(dir: &Path, exes: &[&str]) -> Option<PathBuf> {
    exes.iter().map(|e| dir.join(e)).find(|p| p.is_file())
}

pub fn hoyoplay(p: &PrefixData) -> Vec<Found> {
    let launcher = hoyo_launcher(p);
    let has_config = p.users_appdata("Roaming\\Cognosphere\\HYP") || p.users_appdata("Roaming\\miHoYo\\HYP");
    if launcher.is_none() && !has_config {
        return vec![];
    }
    let mut cands: Vec<(String, PathBuf)> = vec![]; // (game_biz, install dir)
    for vendor in ["Cognosphere", "miHoYo"] {
        for ver in p.user().child_paths(&format!("Software\\{vendor}\\HYP")) {
            for k in p.user().subkeys(&ver) {
                if let Some(dir) = k.get_str("GameInstallPath").and_then(|w| p.path_of(w)).filter(|d| d.is_dir()) {
                    cands.push((k.name().to_string(), dir));
                }
            }
        }
    }
    for d in HOYO_DIRS.iter().filter_map(|d| p.path_of(&format!("{d}\\games"))) {
        for g in super::dirs_in(&d) {
            for (biz, _, exes) in HOYO_GAMES {
                if let Some(exe) = hoyo_exe(&g, exes) {
                    let cn = exe.file_name().is_some_and(|n| n == "YuanShen.exe");
                    cands.push((format!("{biz}_{}", if cn { "cn" } else { "global" }), g.clone()));
                }
            }
        }
    }
    let mut out: Vec<Found> = vec![];
    for (biz, dir) in cands {
        if out.iter().any(|f| f.id == biz || f.install_dir == dir) {
            continue;
        }
        let code = biz.split('_').next().unwrap_or("");
        let known = HOYO_GAMES.iter().find(|(c, _, _)| *c == code);
        let exe = known.and_then(|(_, _, exes)| hoyo_exe(&dir, exes));
        let (exe, args) = match (exe, &launcher) {
            (Some(e), _) => (e, vec![]),
            (None, Some(l)) => (l.clone(), vec![]),
            (None, None) => continue,
        };
        let name = known.map_or_else(|| super::folder_title(&dir), |(_, n, _)| (*n).to_string());
        out.push(Found { source: Source::HoYoPlay, id: biz, name, install_dir: dir, exe, args, launch_ids: vec![] });
    }
    out
}
