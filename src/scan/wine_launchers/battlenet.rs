//! Battle.net in a Wine prefix: `drive_c/ProgramData/Battle.net/Agent/product.db` is a protobuf
//! `Database` written by the Blizzard Agent. Only the parts needed here are decoded:
//!
//! ```text
//! message Database        { repeated ProductInstall product_install = 1; … }
//! message ProductInstall  { string uid = 1; string product_code = 2; UserSettings settings = 3;
//!                           CachedProductState cached_product_state = 4; … }
//! message UserSettings    { string install_path = 1; string play_region = 2; … }
//! message CachedProductState { BaseProductState base_product_state = 1; … }
//! message BaseProductState   { bool installed = 1; bool playable = 2; … }
//! ```
//!
//! Games launch through the client: `Battle.net.exe --exec="launch <CODE>"`.

use super::Found;
use crate::model::Source;
use crate::scan::winprefix::win_to_unix;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------------------------
// Minimal protobuf wire-format reader

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wire<'a> {
    Varint(u64),
    Fixed64(u64),
    Bytes(&'a [u8]),
    Fixed32(u32),
}

fn varint(b: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v: u64 = 0;
    for shift in (0..64).step_by(7) {
        let byte = *b.get(*pos)?;
        *pos += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

/// The fields of one message, in order. `None` when the bytes aren't a well-formed message
/// (truncated, groups, field number 0) — so a corrupt file never yields garbage.
pub fn fields(b: &[u8]) -> Option<Vec<(u32, Wire<'_>)>> {
    let mut out = vec![];
    let mut pos = 0;
    while pos < b.len() {
        let key = varint(b, &mut pos)?;
        let field = u32::try_from(key >> 3).ok().filter(|f| *f != 0)?;
        let w = match key & 7 {
            0 => Wire::Varint(varint(b, &mut pos)?),
            1 => {
                let s = b.get(pos..pos + 8)?;
                pos += 8;
                Wire::Fixed64(u64::from_le_bytes(s.try_into().ok()?))
            }
            2 => {
                let len = usize::try_from(varint(b, &mut pos)?).ok()?;
                let end = pos.checked_add(len)?;
                let s = b.get(pos..end)?;
                pos = end;
                Wire::Bytes(s)
            }
            5 => {
                let s = b.get(pos..pos + 4)?;
                pos += 4;
                Wire::Fixed32(u32::from_le_bytes(s.try_into().ok()?))
            }
            _ => return None,
        };
        out.push((field, w));
    }
    Some(out)
}

fn bytes_of<'a>(f: &[(u32, Wire<'a>)], n: u32) -> Option<&'a [u8]> {
    f.iter().find_map(|(k, w)| match w {
        Wire::Bytes(b) if *k == n => Some(*b),
        _ => None,
    })
}

fn string_of(f: &[(u32, Wire<'_>)], n: u32) -> String {
    bytes_of(f, n).map(|b| String::from_utf8_lossy(b).trim().to_string()).unwrap_or_default()
}

fn bool_of(f: &[(u32, Wire<'_>)], n: u32) -> Option<bool> {
    f.iter().find_map(|(k, w)| match w {
        Wire::Varint(v) if *k == n => Some(*v != 0),
        _ => None,
    })
}

// ---------------------------------------------------------------------------------------------
// product.db

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Install {
    pub uid: String,
    pub product_code: String,
    /// Windows path as the Agent stores it (`C:/Program Files (x86)/Overwatch`).
    pub install_path: String,
    /// `cached_product_state.base_product_state.installed` when present.
    pub installed: Option<bool>,
}

/// Decode the product installs of a `product.db`. Errors only when the file isn't a protobuf
/// message at all; individual broken installs are skipped.
pub fn decode_product_db(bytes: &[u8]) -> anyhow::Result<Vec<Install>> {
    let top = fields(bytes).ok_or_else(|| anyhow::anyhow!("product.db is not a protobuf message"))?;
    let mut out = vec![];
    for (n, w) in &top {
        let (1, Wire::Bytes(b)) = (n, w) else {
            continue;
        };
        let Some(f) = fields(b) else { continue };
        let install_path = bytes_of(&f, 3).and_then(fields).map(|s| string_of(&s, 1)).unwrap_or_default();
        let installed = bytes_of(&f, 4)
            .and_then(fields)
            .and_then(|c| bytes_of(&c, 1).and_then(fields))
            .and_then(|b| bool_of(&b, 1));
        out.push(Install { uid: string_of(&f, 1), product_code: string_of(&f, 2), install_path, installed });
    }
    Ok(out)
}

/// (launch code, product.db uids, name). Codes are what `--exec="launch …"` takes.
const PRODUCTS: &[(&str, &[&str], &str)] = &[
    ("Pro", &["prometheus"], "Overwatch 2"),
    ("WoW", &["wow"], "World of Warcraft"),
    ("WoWC", &["wow_classic"], "World of Warcraft Classic"),
    ("D3", &["diablo3"], "Diablo III"),
    ("Fen", &["fenris"], "Diablo IV"),
    ("OSI", &["osi"], "Diablo II: Resurrected"),
    ("ANBS", &["anbs"], "Diablo Immortal"),
    ("Hero", &["heroes"], "Heroes of the Storm"),
    ("WTCG", &["hs_beta", "hs"], "Hearthstone"),
    ("S2", &["s2"], "StarCraft II"),
    ("S1", &["s1"], "StarCraft"),
    ("W3", &["w3"], "Warcraft III: Reforged"),
    ("GRY", &["gryphon"], "Warcraft Rumble"),
    ("RTRO", &["rtro"], "Blizzard Arcade Collection"),
    ("WLBY", &["wlby"], "Crash Bandicoot 4: It's About Time"),
    ("VIPR", &["viper"], "Call of Duty: Black Ops 4"),
    ("ODIN", &["odin"], "Call of Duty: Modern Warfare"),
    ("LAZR", &["lazarus"], "Call of Duty: Modern Warfare 2 Campaign Remastered"),
    ("ZEUS", &["zeus"], "Call of Duty: Black Ops Cold War"),
    ("FORE", &["fore"], "Call of Duty: Vanguard"),
    ("AUKS", &["auks"], "Call of Duty"),
];

/// Products that are the client / agent itself, not games.
fn is_client(i: &Install) -> bool {
    let c = i.product_code.to_ascii_lowercase();
    let u = i.uid.to_ascii_lowercase();
    ["agent", "bna", "battle.net"].iter().any(|x| c == *x || u == *x)
}

/// Canonical launch code and known name of an install.
pub fn product_info(i: &Install) -> (String, Option<&'static str>) {
    let hit = PRODUCTS.iter().find(|(code, uids, _)| {
        code.eq_ignore_ascii_case(&i.product_code) || uids.iter().any(|u| u.eq_ignore_ascii_case(&i.uid))
    });
    match hit {
        Some((code, _, name)) => ((*code).to_string(), Some(*name)),
        None => {
            let code = if i.product_code.is_empty() { i.uid.clone() } else { i.product_code.clone() };
            (code, None)
        }
    }
}

/// The client: next to the `bna` install the Agent knows, else the default folders.
pub fn client_exe(prefix: &Path, installs: &[Install]) -> Option<PathBuf> {
    let mut cands: Vec<PathBuf> = installs
        .iter()
        .filter(|i| is_client(i) && !i.install_path.is_empty())
        .filter_map(|i| win_to_unix(prefix, &format!("{}/Battle.net.exe", i.install_path)))
        .collect();
    for dir in ["C:\\Program Files (x86)\\Battle.net", "C:\\Program Files\\Battle.net"] {
        cands.extend(win_to_unix(prefix, &format!("{dir}\\Battle.net.exe")));
    }
    cands.into_iter().find(|p| p.is_file())
}

fn db_path(prefix: &Path) -> Option<PathBuf> {
    win_to_unix(prefix, "C:\\ProgramData\\Battle.net\\Agent\\product.db").filter(|p| p.is_file())
}

/// The client, when this prefix has Battle.net.
pub fn launcher(prefix: &Path) -> Option<PathBuf> {
    let db = db_path(prefix);
    let installs = db
        .and_then(|p| crate::scan::util::read_bytes(p).ok())
        .and_then(|b| decode_product_db(&b).ok())
        .unwrap_or_default();
    client_exe(prefix, &installs)
}

/// Installed Battle.net games of a prefix.
pub fn scan(prefix: &Path) -> anyhow::Result<Vec<Found>> {
    let Some(db) = db_path(prefix) else {
        return Ok(vec![]);
    };
    let installs = decode_product_db(&crate::scan::util::read_bytes(&db)?)?;
    let Some(client) = client_exe(prefix, &installs) else {
        return Ok(vec![]);
    };
    let mut out: Vec<Found> = vec![];
    for i in &installs {
        if is_client(i) || i.install_path.is_empty() || i.installed == Some(false) {
            continue;
        }
        let Some(dir) = win_to_unix(prefix, &i.install_path).filter(|d| d.is_dir()) else {
            continue;
        };
        let (code, name) = product_info(i);
        if code.is_empty() || out.iter().any(|f| f.id == code) {
            continue;
        }
        let name = name.map(str::to_string).unwrap_or_else(|| super::folder_title(&dir));
        out.push(Found {
            source: Source::BattleNet,
            id: code.clone(),
            name,
            install_dir: dir,
            exe: client.clone(),
            args: vec![format!("--exec=launch {code}")],
            launch_ids: vec![],
        });
    }
    Ok(out)
}
