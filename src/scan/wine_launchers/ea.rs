//! EA app (and Origin) games in a Wine prefix.
//!
//! Learned from a real EA app prefix (Lutris `ea-app` installer, The Sims 4):
//! * Client: `C:\Program Files\Electronic Arts\EA Desktop\EA Desktop\EADesktop.exe`
//!   (`Software\[Wow6432Node\]Electronic Arts\EA Desktop` `ClientPath`). `EALauncher.exe` next to
//!   it is the registered `origin2://` handler (and what Lutris runs to open the client);
//!   `Link2EA.exe` handles `link2ea://` (Steam / Epic copies of EA games).
//! * Every EA-installer game folder holds `__Installer/installerdata.xml`: `<contentIDs>`,
//!   `<gameTitles><gameTitle locale=…>`, `<runtime><launcher><filePath>` (the game's exe, written
//!   as `[HKEY_LOCAL_MACHINE\SOFTWARE\<publisher>\<title>\Install Dir]Game\Bin\x.exe`).
//! * The installer writes `Install Dir` under the game's *publisher* key, not EA's:
//!   `Software\[Wow6432Node\]Maxis\The Sims 4`, `…\BioWare\…`, `…\Respawn\…`, `…\EA Games\…`;
//!   plus an Uninstall entry (`InstallLocation`) and `Origin Games\<content id>` (no path).
//! * `ProgramData\EA Desktop\InstallData\<title>\…` exists per installed title, and
//!   `machine.ini` `machine.downloadinplacedir=C:\Program Files\EA Games\` is the library folder.
//! * The offer id the client launches by (`OFB-EAST:109552408`) is only on disk in the client's
//!   logs: `ProgramData\EA Desktop\Logs\<offer>_install_log.txt` (UTF-16, `Install Location: …`)
//!   and `EADesktop.log` (`initiateDownload() called for offerId=[…] … installPath=[…]`).
//! * Origin era: `ProgramData\Origin\LocalContent\*\*.mfst` (`?id=<offer>&dipinstallpath=…`).
//!
//! Launch: with an offer id, `EALauncher.exe origin2://game/launch?offerIds=<id>&autoDownload=1`
//! (Origin: `Origin.exe` with the same link) through Lodestar's helper (`client_launch.rs`, kind
//! `ea`), `<id>` = the content id when known, else the offer id: the EA app looks a link's ids up
//! as *content* ids (its log on a real machine: `findByContentId … Could not find entitlement for
//! contentId [OFB-EAST:109552408]`, "You don't have access", while the same game started from its
//! Play button as `content[1015875]`). `Found::launch_ids` = [content id, offer id] for retries.
//! Without an offer id: the game's own launcher exe from `installerdata.xml` (what the installer's
//! desktop shortcut runs — it starts the EA app itself).
//! `source_id` is the content id (stable while the game is installed), else the offer id.
//! Install folders inside a Steam library (`steamapps/common`) are Steam's copies: skipped.

use super::{Found, PrefixData};
use crate::model::Source;
use std::path::{Path, PathBuf};

pub const EA_DIRS: &[&str] = &[
    "C:\\Program Files\\Electronic Arts\\EA Desktop",
    "C:\\ProgramData\\EA Desktop",
    "C:\\Program Files (x86)\\Origin",
    "C:\\ProgramData\\Origin",
];

/// EA's own vendor folders: a key there with an install folder is a game even when its
/// `installerdata.xml` is missing. Under any other publisher the file must be there.
const EA_VENDORS: &[&str] = &["ea games", "electronic arts", "origin games"];

/// Registry keys under the EA vendor folders that belong to the client, not to a game.
const EA_CLIENT_KEYS: &[&str] =
    &["ea desktop", "ea core", "eadm", "ea app", "origin", "ealauncher", "ea installer", "ea services"];

/// Default EA / Origin library folders (the EA app's `downloadinplacedir` is added to these).
const EA_LIBRARIES: &[&str] = &["C:\\Program Files\\EA Games", "C:\\Program Files (x86)\\Origin Games"];

/// (client exe, is the EA app — else Origin). The client itself, for Install / Uninstall.
pub fn ea_launcher(p: &PrefixData) -> Option<(PathBuf, bool)> {
    let reg = first_key(p, "Electronic Arts\\EA Desktop");
    let mut cands: Vec<PathBuf> = vec![];
    for v in ["ClientPath", "DesktopAppPath"] {
        cands.extend(reg.and_then(|k| k.get_str(v)).and_then(|c| p.path_of(c)));
    }
    cands.extend(
        reg.and_then(|k| k.get_str("InstallLocation"))
            .and_then(|d| p.path_of(&format!("{}\\EADesktop.exe", d.trim_end_matches('\\')))),
    );
    cands.extend(p.path_of("C:\\Program Files\\Electronic Arts\\EA Desktop\\EA Desktop\\EADesktop.exe"));
    if let Some(c) = cands.into_iter().find(|c| c.is_file()) {
        return Some((c, true));
    }
    let origin = first_key(p, "Origin")
        .and_then(|k| k.get_str("ClientPath"))
        .and_then(|c| p.path_of(c))
        .into_iter()
        .chain(p.path_of("C:\\Program Files (x86)\\Origin\\Origin.exe"))
        .find(|c| c.is_file() && !c.ends_with("EADesktop.exe"))?;
    Some((origin, false))
}

/// The program that takes `origin2://` links: the EA app's `EALauncher.exe` (its registered
/// handler), else the client itself.
fn link_handler(client: &Path, ea_app: bool) -> PathBuf {
    if ea_app && let Some(l) = client.parent().map(|d| d.join("EALauncher.exe")).filter(|l| l.is_file()) {
        return l;
    }
    client.to_path_buf()
}

fn first_key<'a>(p: &'a PrefixData, rest: &str) -> Option<&'a crate::scan::winprefix::RegKey> {
    [format!("Software\\Wow6432Node\\{rest}"), format!("Software\\{rest}")].iter().find_map(|k| p.system().key(k))
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct InstallerData {
    /// Every `<contentID>` (the base game's first).
    pub content_ids: Vec<String>,
    pub title: Option<String>,
    /// The first visible `<runtime><launcher><filePath>`, as written.
    pub launcher: Option<String>,
}

/// `__Installer/installerdata.xml`: `<contentID>`s, the en_US (else first) `<gameTitle>` and the
/// game's launcher exe.
pub fn parse_installerdata(text: &str) -> InstallerData {
    let Ok(doc) = roxmltree::Document::parse(text) else {
        return InstallerData::default();
    };
    let text_of = |n: roxmltree::Node| n.text().map(str::trim).filter(|t| !t.is_empty()).map(str::to_string);
    let mut content_ids: Vec<String> = vec![];
    for id in doc
        .descendants()
        .filter(|n| n.has_tag_name("contentID") && n.parent().is_some_and(|p| p.has_tag_name("contentIDs")))
        .filter_map(text_of)
    {
        if !content_ids.contains(&id) {
            content_ids.push(id);
        }
    }
    let titles: Vec<_> = doc.descendants().filter(|n| n.has_tag_name("gameTitle")).collect();
    let title = titles
        .iter()
        .find(|n| n.attribute("locale").is_some_and(|l| l.eq_ignore_ascii_case("en_US")))
        .or(titles.first())
        .and_then(|n| text_of(*n))
        .map(|t| clean_title(&t));
    let child_text = |n: roxmltree::Node, tag: &str| n.children().find(|c| c.has_tag_name(tag)).and_then(text_of);
    let launcher = doc
        .descendants()
        .filter(|n| n.has_tag_name("launcher") && n.ancestors().any(|a| a.has_tag_name("runtime")))
        .filter(|n| child_text(*n, "hidden").as_deref() != Some("1"))
        .filter(|n| child_text(*n, "trial").as_deref() != Some("1"))
        .find_map(|n| child_text(n, "filePath"));
    InstallerData { content_ids, title, launcher }
}

/// "Mass Effect™ Legendary Edition" → "Mass Effect Legendary Edition".
pub fn clean_title(t: &str) -> String {
    t.replace(['™', '®', '©'], "").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Percent-decode (`%3a` → `:`, `+` → space) a query value.
fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let (Some(h), Some(l)) = (b.get(i + 1).and_then(|c| hex(*c)), b.get(i + 2).and_then(|c| hex(*c)))
        {
            out.push(h << 4 | l);
            i += 3;
            continue;
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// An Origin `.mfst` (`?currentstate=kReadyToStart&id=Origin.OFR.50.0001&dipinstallpath=C%3a%5c…`)
/// → (offer id, Windows install path).
pub fn parse_mfst(text: &str) -> Option<(String, Option<String>)> {
    let q = text.trim().trim_start_matches('?');
    let mut id = None;
    let mut path = None;
    for kv in q.split('&') {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        match k {
            "id" => id = Some(pct_decode(v)).filter(|s| !s.is_empty()),
            "dipinstallpath" => path = Some(pct_decode(v)).filter(|s| !s.is_empty()),
            _ => {}
        }
    }
    Some((id?, path))
}

/// Text of a log that may be UTF-16LE (the EA installer's `*_install_log.txt`) or UTF-8.
fn decode_log(bytes: &[u8]) -> String {
    let utf16 =
        bytes.starts_with(&[0xff, 0xfe]) || (bytes.len() >= 4 && bytes[1] == 0 && bytes[3] == 0 && bytes[0] != 0);
    if utf16 {
        let body = bytes.strip_prefix(&[0xff, 0xfe]).unwrap_or(bytes);
        let units: Vec<u16> = body.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

/// `…  Install Location: C:\Program Files\EA Games\The Sims 4\` → the path.
pub fn install_log_location(text: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let (_, rest) = l.split_once("Install Location:")?;
        Some(rest.trim().to_string()).filter(|s| !s.is_empty())
    })
}

/// `initiateDownload() called for offerId=[OFB-EAST:1] slug[x] … installPath=[C:\…]` lines of
/// `EADesktop.log` → (offer, Windows path), in file order.
pub fn client_log_installs(text: &str) -> Vec<(String, String)> {
    let field = |l: &str, key: &str| -> Option<String> {
        let start = l.find(key)? + key.len();
        let end = l[start..].find(']')? + start;
        Some(l[start..end].trim().to_string()).filter(|s| !s.is_empty())
    };
    text.lines()
        .filter(|l| l.contains("initiateDownload"))
        .filter_map(|l| Some((field(l, "offerId=[")?, field(l, "installPath=[")?)))
        .collect()
}

/// Offer ids the EA app wrote next to install folders, latest last: (offer, folder).
fn offer_ids(p: &PrefixData) -> Vec<(String, PathBuf)> {
    let Some(logs) = p.path_of("C:\\ProgramData\\EA Desktop\\Logs").filter(|d| d.is_dir()) else {
        return vec![];
    };
    let mut out: Vec<(String, PathBuf)> = vec![];
    let mut install_logs: Vec<(i64, String, PathBuf)> = super::files_in(&logs)
        .into_iter()
        .filter_map(|f| {
            let name = f.file_name()?.to_str()?.to_string();
            let offer = name.strip_suffix("_install_log.txt")?.trim().to_string();
            (!offer.is_empty()).then(|| (crate::scan::util::mtime(&f).unwrap_or(0), offer, f))
        })
        .collect();
    install_logs.sort();
    for (_, offer, f) in install_logs {
        let Ok(bytes) = crate::scan::util::read_bytes(&f) else {
            continue;
        };
        if let Some(dir) = install_log_location(&decode_log(&bytes)).and_then(|w| p.path_of(&w)) {
            out.push((offer, dir));
        }
    }
    for name in ["EADesktop.log"] {
        let Ok(text) = crate::scan::util::read_text(logs.join(name)) else {
            continue;
        };
        for (offer, win) in client_log_installs(&text) {
            if let Some(dir) = p.path_of(&win) {
                out.push((offer, dir));
            }
        }
    }
    out
}

/// `machine.downloadinplacedir` of the EA app's `machine.ini`.
fn download_dir(p: &PrefixData) -> Option<String> {
    let f = p.path_of("C:\\ProgramData\\EA Desktop\\machine.ini")?;
    let text = crate::scan::util::read_text(f).ok()?;
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim().eq_ignore_ascii_case("machine.downloadinplacedir"))
            .then(|| v.trim().to_string())
            .filter(|v| !v.is_empty())
    })
}

fn existing_dir(p: &PrefixData, win: &str) -> Option<PathBuf> {
    p.path_of(win).filter(|d| d.is_dir())
}

fn installer_xml(dir: &Path) -> PathBuf {
    dir.join("__Installer").join("installerdata.xml")
}

/// A folder inside a Steam library belongs to Steam's copy of the game.
pub(crate) fn in_steam_library(dir: &Path) -> bool {
    let s = dir.to_string_lossy().to_ascii_lowercase();
    s.contains("/steamapps/common/")
}

struct Cand {
    dir: PathBuf,
    /// Registry key name / display name when the game came from the registry.
    name: Option<String>,
    offer: Option<String>,
}

/// Every EA install folder of the prefix, first source first.
fn candidates(p: &PrefixData) -> Vec<Cand> {
    let mut out: Vec<Cand> = vec![];
    let add = |out: &mut Vec<Cand>, dir: PathBuf, name: Option<String>, offer: Option<String>| {
        if in_steam_library(&dir) {
            return;
        }
        match out.iter_mut().find(|c| c.dir == dir) {
            Some(c) => {
                if c.offer.is_none() {
                    c.offer = offer;
                }
                if c.name.is_none() {
                    c.name = name;
                }
            }
            None => out.push(Cand { dir, name, offer }),
        }
    };
    // 1. `Software\<publisher>\<title>` keys with an install folder.
    let mut keys: Vec<_> = p.system().keys().collect();
    keys.sort_by_key(|k| k.path.to_lowercase());
    for k in keys {
        let norm = crate::scan::winprefix::registry::norm_key(&k.path);
        let Some(rest) = norm.strip_prefix("software\\") else {
            continue;
        };
        let rest = rest.strip_prefix("wow6432node\\").unwrap_or(rest);
        let mut parts = rest.split('\\');
        let (Some(vendor), Some(title), None) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let ea_vendor = EA_VENDORS.contains(&vendor);
        if ea_vendor && EA_CLIENT_KEYS.contains(&title) {
            continue;
        }
        let values: &[&str] = if ea_vendor {
            &["Install Dir", "InstallDir", "Install Location", "InstallLocation"]
        } else {
            &["Install Dir"]
        };
        let Some(dir) = values.iter().find_map(|v| k.get_str(v)).and_then(|d| existing_dir(p, d)) else {
            continue;
        };
        if !ea_vendor && !installer_xml(&dir).is_file() {
            continue;
        }
        // `Origin Games\<content id>` keys are named by id.
        let key_is_id = title.bytes().all(|b| b.is_ascii_digit());
        let name = k.get_str("DisplayName").map(clean_title).or_else(|| (!key_is_id).then(|| clean_title(k.name())));
        add(&mut out, dir, name, None);
    }
    // 2. Uninstall entries of EA-installer games.
    for base in [
        "Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall",
    ] {
        for k in p.system().subkeys(base) {
            let Some(dir) = k.get_str("InstallLocation").and_then(|d| existing_dir(p, d)) else {
                continue;
            };
            if installer_xml(&dir).is_file() {
                add(&mut out, dir, k.get_str("DisplayName").map(clean_title), None);
            }
        }
    }
    // 3. The EA app's library folders and its per-title InstallData folders.
    let mut libraries: Vec<PathBuf> = vec![];
    for lib in download_dir(p).iter().map(String::as_str).chain(EA_LIBRARIES.iter().copied()) {
        if let Some(d) = existing_dir(p, lib)
            && !libraries.contains(&d)
        {
            libraries.push(d);
        }
    }
    let install_data = p.path_of("C:\\ProgramData\\EA Desktop\\InstallData");
    let titles: Vec<String> = install_data
        .map(|d| super::dirs_in(&d))
        .unwrap_or_default()
        .iter()
        .filter_map(|d| d.file_name()?.to_str().map(str::to_string))
        .collect();
    for lib in &libraries {
        for t in &titles {
            let dir = lib.join(t);
            if installer_xml(&dir).is_file() {
                add(&mut out, dir, None, None);
            }
        }
        for dir in super::dirs_in(lib) {
            if installer_xml(&dir).is_file() {
                add(&mut out, dir, None, None);
            }
        }
    }
    // 4. Origin's local content manifests carry offer ids.
    if let Some(lc) = p.path_of("C:\\ProgramData\\Origin\\LocalContent") {
        for d in super::dirs_in(&lc) {
            for f in super::files_in(&d)
                .into_iter()
                .filter(|f| f.extension().is_some_and(|e| e.eq_ignore_ascii_case("mfst")))
            {
                let Some((offer, path)) = crate::scan::util::read_text(&f).ok().and_then(|t| parse_mfst(&t)) else {
                    continue;
                };
                if let Some(dir) = path.and_then(|w| existing_dir(p, &w)) {
                    add(&mut out, dir, None, Some(offer));
                }
            }
        }
    }
    // 5. The EA app's logs name the offer of each install folder (latest wins).
    let offers = offer_ids(p);
    for c in out.iter_mut() {
        if let Some((offer, _)) = offers.iter().rev().find(|(_, d)| *d == c.dir) {
            c.offer = Some(offer.clone());
        }
    }
    out
}

/// `[HKEY_LOCAL_MACHINE\SOFTWARE\Maxis\The Sims 4\Install Dir]Game\Bin\TS4_Launcher_x64.exe`
/// → `<install dir>/Game/Bin/TS4_Launcher_x64.exe` when it exists.
fn launcher_exe(p: &PrefixData, dir: &Path, file_path: &str) -> Option<PathBuf> {
    let rel = match file_path.trim().strip_prefix('[') {
        Some(r) => r.split_once(']')?.1,
        None => return p.path_of(file_path).filter(|f| f.is_file()),
    };
    let win = format!(
        "{}\\{}",
        crate::scan::winprefix::unix_to_win(&p.path, dir).trim_end_matches('\\'),
        rel.trim_start_matches(['\\', '/'])
    );
    p.path_of(&win).filter(|f| f.is_file())
}

pub fn ea(p: &PrefixData) -> Vec<Found> {
    if !p.has_any(EA_DIRS) {
        return vec![];
    }
    let Some((client, ea_app)) = ea_launcher(p) else {
        return vec![];
    };
    let handler = link_handler(&client, ea_app);
    // `Origin Games\<id>` keys name the content id the client tracks the game by.
    let origin_ids: Vec<String> = ["Software\\Wow6432Node\\Origin Games", "Software\\Origin Games"]
        .iter()
        .flat_map(|b| p.system().subkeys(b))
        .map(|k| k.name().to_string())
        .collect();
    let mut out: Vec<Found> = vec![];
    for c in candidates(p) {
        let data =
            crate::scan::util::read_text(installer_xml(&c.dir)).map(|t| parse_installerdata(&t)).unwrap_or_default();
        let content_id =
            data.content_ids.iter().find(|id| origin_ids.contains(id)).or(data.content_ids.first()).cloned();
        let name = data.title.clone().or(c.name).unwrap_or_else(|| super::folder_title(&c.dir));
        let link = |id: &str| vec![format!("origin2://game/launch?offerIds={id}&autoDownload=1")];
        let own_exe = data.launcher.as_deref().and_then(|f| launcher_exe(p, &c.dir, f));
        // The EA app looks a link's ids up as content ids (a real EA app answered the offer id
        // with `Could not find entitlement for contentId [OFB-EAST:…]`): content id first, the
        // offer id as the fallback the helper retries with.
        let mut launch_ids: Vec<String> = vec![];
        for id in [&content_id, &c.offer].into_iter().flatten() {
            if !launch_ids.contains(id) {
                launch_ids.push(id.clone());
            }
        }
        let (exe, args) = match (&c.offer, own_exe, &content_id) {
            (Some(offer), _, _) => (handler.clone(), link(content_id.as_ref().unwrap_or(offer))),
            (None, Some(exe), _) => {
                launch_ids.clear();
                (exe, vec![])
            }
            (None, None, Some(id)) => (handler.clone(), link(id)),
            (None, None, None) => (client.clone(), vec![]),
        };
        let id = content_id.or(c.offer).unwrap_or_else(|| super::slug(&name));
        if out.iter().any(|f| f.id == id) {
            continue;
        }
        out.push(Found { source: Source::Ea, id, name, install_dir: c.dir, exe, args, launch_ids });
    }
    out
}
