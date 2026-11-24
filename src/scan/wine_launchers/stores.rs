//! Ubisoft Connect and Rockstar Games Launcher in a Wine prefix — all found through the
//! prefix registry (`system.reg`, HKLM; 32-bit launchers write under `Software\Wow6432Node`).
//!
//! * Ubisoft: `Software\…\Ubisoft\Launcher` `InstallDir` (the client), `…\Launcher\Installs\<id>`
//!   `InstallDir` per game; names from the `Uninstall\Uplay Install <id>` `DisplayName`, else the
//!   install folder. Launch: `UbisoftConnect.exe uplay://launch/<id>/0` (or `upc.exe`).
//! * EA app / Origin live in `ea.rs`.
//! * Rockstar: `Software\…\Rockstar Games\<title>` `InstallFolder`; the client is
//!   `…\Rockstar Games\Launcher` `InstallFolder\Launcher.exe`. Launch:
//!   `Launcher.exe -launchTitleInFolder <install folder>`.

use super::{Found, PrefixData};
use crate::model::Source;
use crate::scan::winprefix::RegKey;
use std::path::{Path, PathBuf};

/// `Software\Wow6432Node\<rest>` then `Software\<rest>`.
fn both(rest: &str) -> [String; 2] {
    [format!("Software\\Wow6432Node\\{rest}"), format!("Software\\{rest}")]
}

fn first_key<'a>(p: &'a PrefixData, rest: &str) -> Option<&'a RegKey> {
    both(rest).iter().find_map(|k| p.system().key(k))
}

fn subkeys<'a>(p: &'a PrefixData, rest: &str) -> Vec<&'a RegKey> {
    both(rest).iter().flat_map(|k| p.system().subkeys(k)).collect()
}

fn existing_dir(p: &PrefixData, win: &str) -> Option<PathBuf> {
    p.path_of(win).filter(|d| d.is_dir())
}

fn first_file(dir: &Path, names: &[&str]) -> Option<PathBuf> {
    names.iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

// ---------------------------------------------------------------------------------------------
// Ubisoft Connect

pub const UBISOFT_DIRS: &[&str] =
    &["C:\\Program Files (x86)\\Ubisoft\\Ubisoft Game Launcher", "C:\\Program Files\\Ubisoft\\Ubisoft Game Launcher"];

pub fn ubisoft_launcher(p: &PrefixData) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![];
    dirs.extend(first_key(p, "Ubisoft\\Launcher").and_then(|k| k.get_str("InstallDir")).and_then(|d| p.path_of(d)));
    dirs.extend(UBISOFT_DIRS.iter().filter_map(|d| p.path_of(d)));
    dirs.iter().find_map(|d| first_file(d, &["UbisoftConnect.exe", "upc.exe", "Uplay.exe"]))
}

pub fn ubisoft(p: &PrefixData) -> Vec<Found> {
    if !p.has_any(UBISOFT_DIRS) {
        return vec![];
    }
    let Some(client) = ubisoft_launcher(p) else {
        return vec![];
    };
    let mut out: Vec<Found> = vec![];
    for k in subkeys(p, "Ubisoft\\Launcher\\Installs") {
        let id = k.name().to_string();
        if id.is_empty() || out.iter().any(|f| f.id == id) {
            continue;
        }
        let Some(dir) = k.get_str("InstallDir").and_then(|d| existing_dir(p, d)) else {
            continue;
        };
        let name = both(&format!("Microsoft\\Windows\\CurrentVersion\\Uninstall\\Uplay Install {id}"))
            .iter()
            .find_map(|u| p.system().get_str(u, "DisplayName").map(str::to_string))
            .unwrap_or_else(|| super::folder_title(&dir));
        out.push(Found {
            source: Source::Ubisoft,
            args: vec![format!("uplay://launch/{id}/0")],
            id,
            name,
            install_dir: dir,
            exe: client.clone(),
            launch_ids: vec![],
        });
    }
    out
}

// ---------------------------------------------------------------------------------------------
// EA app / Origin: see `ea.rs`.

// ---------------------------------------------------------------------------------------------
// Rockstar Games Launcher

pub const ROCKSTAR_DIRS: &[&str] = &["C:\\Program Files\\Rockstar Games", "C:\\Program Files (x86)\\Rockstar Games"];

/// Rockstar's registry title → (id, name).
const ROCKSTAR_TITLES: &[(&str, &str, &str)] = &[
    ("grand theft auto v", "gta5", "Grand Theft Auto V"),
    ("gtav", "gta5", "Grand Theft Auto V"),
    ("grand theft auto v enhanced", "gta5e", "Grand Theft Auto V Enhanced"),
    ("red dead redemption 2", "rdr2", "Red Dead Redemption 2"),
    ("red dead redemption", "rdr", "Red Dead Redemption"),
    ("l.a. noire", "lanoire", "L.A. Noire"),
    ("l.a. noire: the vr case files", "lanoirevr", "L.A. Noire: The VR Case Files"),
    ("max payne 3", "mp3", "Max Payne 3"),
    ("grand theft auto iv", "gta4", "Grand Theft Auto IV"),
    ("gtaiv", "gta4", "Grand Theft Auto IV"),
    ("bully", "bully", "Bully: Scholarship Edition"),
    ("grand theft auto: san andreas", "gtasa", "Grand Theft Auto: San Andreas"),
    ("grand theft auto: vice city", "gtavc", "Grand Theft Auto: Vice City"),
    ("grand theft auto iii", "gta3", "Grand Theft Auto III"),
];

const ROCKSTAR_CLIENT_KEYS: &[&str] =
    &["launcher", "rockstar games launcher", "rockstar games social club", "social club"];

pub fn rockstar_launcher(p: &PrefixData) -> Option<PathBuf> {
    first_key(p, "Rockstar Games\\Launcher")
        .and_then(|k| k.get_str("InstallFolder"))
        .and_then(|d| p.path_of(&format!("{}\\Launcher.exe", d.trim_end_matches(['\\', '/']))))
        .into_iter()
        .chain(ROCKSTAR_DIRS.iter().filter_map(|d| p.path_of(&format!("{d}\\Launcher\\Launcher.exe"))))
        .find(|c| c.is_file())
}

pub fn rockstar(p: &PrefixData) -> Vec<Found> {
    if !p.has_any(ROCKSTAR_DIRS) {
        return vec![];
    }
    let Some(client) = rockstar_launcher(p) else {
        return vec![];
    };
    let mut out: Vec<Found> = vec![];
    for k in subkeys(p, "Rockstar Games") {
        let title = k.name().to_string();
        let lower = title.to_ascii_lowercase();
        if ROCKSTAR_CLIENT_KEYS.contains(&lower.as_str()) {
            continue;
        }
        let Some(win) = k.get_str("InstallFolder") else {
            continue;
        };
        let Some(dir) = existing_dir(p, win) else {
            continue;
        };
        let (id, name) = ROCKSTAR_TITLES
            .iter()
            .find(|(t, _, _)| *t == lower)
            .map_or_else(|| (super::slug(&title), title.clone()), |(_, id, n)| ((*id).to_string(), (*n).to_string()));
        if out.iter().any(|f| f.id == id) {
            continue;
        }
        out.push(Found {
            source: Source::Rockstar,
            id,
            name,
            install_dir: dir,
            exe: client.clone(),
            args: vec!["-launchTitleInFolder".into(), win.trim_end_matches(['\\', '/']).to_string()],
            launch_ids: vec![],
        });
    }
    out
}
