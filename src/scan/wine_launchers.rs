//! Windows store launchers that live in Wine prefixes on Linux: Battle.net, EA app, Ubisoft Connect,
//! Rockstar Games Launcher, Riot Client, HoYoPlay.
//!
//! CONTRACT: `pub fn scan(ctx) -> Result<Vec<ScannedGame>>` reporting Source::{BattleNet, Ea, Ubisoft,
//! Rockstar, Riot, HoYoPlay}. Prefixes come from `winprefix::find_prefixes`. Each game launches through its
//! launcher inside that prefix with the prefix's runner ; installed games only.
//!
//! Per prefix only a few `stat`s decide whether a launcher is there at all; the registry is read
//! (filtered to the launchers' keys) only when one is. Details per launcher: `battlenet.rs`,
//! `ea.rs` (EA), `stores.rs` (Ubisoft, Rockstar), `riot_hoyo.rs` (Riot, HoYoPlay).
//!
//! `source_id` = the store's game id (Battle.net launch code, Ubisoft install id, EA offer /
//! content id, Rockstar title id, Riot product, HoYoPlay game_biz). When the same game is in a
//! second prefix, that copy gets `@<prefix hash>` appended (the first one keeps the plain id).
//! Install / Uninstall go through the client: [`launcher_command`].

pub mod battlenet;
pub mod ea;
pub mod riot_hoyo;
pub mod stores;

use super::ScanContext;
use super::winprefix::{self, Hive, Prefix, Registry};
use crate::model::*;
use std::cell::OnceCell;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// One installed game found in a prefix: the Windows program to run there (`exe`, a Unix path)
/// with `args`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub source: Source,
    pub id: String,
    pub name: String,
    pub install_dir: PathBuf,
    pub exe: PathBuf,
    pub args: Vec<String>,
    /// The ids the store client may know the game by, in the order a launch request tries them
    /// (EA app: content id, then offer id). Empty for the other stores.
    pub launch_ids: Vec<String>,
}

/// A prefix being looked at, with its registry hives read on first use.
pub struct PrefixData {
    pub path: PathBuf,
    system: OnceCell<Registry>,
    user: OnceCell<Registry>,
}

/// Vendor folders under `Software\` (and `Software\Wow6432Node\`) the launchers read whole.
const VENDORS: &[&str] =
    &["ubisoft", "electronic arts", "ea games", "origin games", "origin", "rockstar games", "cognosphere", "mihoyo"];

/// Vendor folders that never hold a game's `Software\<vendor>\<title>` key.
const NOT_GAME_VENDORS: &[&str] = &["classes", "microsoft", "wine", "policies", "wow6432node"];

/// Registry keys the launchers read (normalised, lowercase): the vendors above, every
/// `Software\<publisher>\<title>` key (the EA installer writes `Install Dir` under the game's
/// publisher: `Maxis\The Sims 4`, `BioWare\…`, `Respawn\…`), and every Uninstall entry.
fn wanted_key(k: &str) -> bool {
    let Some(rest) = k.strip_prefix("software\\") else {
        return false;
    };
    let rest = rest.strip_prefix("wow6432node\\").unwrap_or(rest);
    if VENDORS.iter().any(|v| rest == *v || rest.strip_prefix(v).is_some_and(|r| r.starts_with('\\'))) {
        return true;
    }
    if let Some(u) = rest.strip_prefix("microsoft\\windows\\currentversion\\uninstall\\") {
        return !u.is_empty() && !u.contains('\\');
    }
    let mut parts = rest.split('\\');
    let (Some(vendor), Some(title), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    !vendor.is_empty() && !title.is_empty() && !NOT_GAME_VENDORS.contains(&vendor)
}

/// Could [`wanted_key`] accept this raw `.reg` header (escaped, after its `[`)? Lets the parser
/// skip unescaping the ~20k other keys of a Proton prefix's system.reg (10 ms → ~2 ms).
fn maybe_wanted_raw(raw: &str) -> bool {
    let name = raw.rfind(']').map_or(raw, |i| &raw[..i]);
    let mut parts = name.split("\\\\");
    let first = parts.next().unwrap_or("");
    if first.len() >= 2 && first[..2].eq_ignore_ascii_case("hk") {
        return true; // a hive-prefixed path (not written by Wine): let wanted_key decide
    }
    if !first.eq_ignore_ascii_case("software") {
        return false;
    }
    let mut next = parts.next().unwrap_or("");
    if next.eq_ignore_ascii_case("wow6432node") {
        next = parts.next().unwrap_or("");
    }
    if next.eq_ignore_ascii_case("microsoft") || VENDORS.iter().any(|v| next.eq_ignore_ascii_case(v)) {
        return true;
    }
    // `Software\<publisher>\<title>` exactly (escapes can only make a name look longer).
    parts.count() == 1 && !next.eq_ignore_ascii_case("classes")
}

impl PrefixData {
    pub fn new(path: &Path) -> PrefixData {
        PrefixData { path: path.to_path_buf(), system: OnceCell::new(), user: OnceCell::new() }
    }
    /// HKLM keys of the store launchers.
    pub fn system(&self) -> &Registry {
        self.system.get_or_init(|| Registry::load_prefiltered(&self.path, Hive::System, maybe_wanted_raw, wanted_key))
    }
    /// HKCU keys of the store launchers.
    pub fn user(&self) -> &Registry {
        self.user.get_or_init(|| Registry::load_prefiltered(&self.path, Hive::User, maybe_wanted_raw, wanted_key))
    }
    /// A Windows path (`C:\…`, `C:/…`, other drives via `dosdevices`) in this prefix.
    pub fn path_of(&self, win: &str) -> Option<PathBuf> {
        winprefix::win_to_unix(&self.path, win)
    }
    /// Any of these Windows folders exists.
    pub fn has_any(&self, dirs: &[&str]) -> bool {
        dirs.iter().any(|d| self.path_of(d).is_some_and(|p| p.is_dir()))
    }
    /// `drive_c/users/<any>/AppData/<rel>` exists.
    pub fn users_appdata(&self, rel: &str) -> bool {
        dirs_in(&self.path.join("drive_c/users")).iter().take(16).any(|u| {
            winprefix::win_to_unix(&self.path, &format!("C:\\users\\{}\\AppData\\{rel}", file_name(u)))
                .is_some_and(|p| p.is_dir())
        })
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Sub-folders of `d`, sorted (at most 500).
pub(crate) fn dirs_in(d: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(d) else {
        return vec![];
    };
    let mut v: Vec<PathBuf> = rd.flatten().take(500).map(|e| e.path()).filter(|p| p.is_dir()).collect();
    v.sort();
    v
}

/// Files in `d`, sorted (at most 500).
pub(crate) fn files_in(d: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(d) else {
        return vec![];
    };
    let mut v: Vec<PathBuf> = rd.flatten().take(500).map(|e| e.path()).filter(|p| p.is_file()).collect();
    v.sort();
    v
}

/// A game name from its install folder ("Assassin's Creed Origins").
pub(crate) fn folder_title(dir: &Path) -> String {
    file_name(dir).trim().to_string()
}

/// "Grand Theft Auto: V" → "grand-theft-auto-v".
pub(crate) fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Every installed launcher game of one prefix. Errors (an unreadable `product.db`) are returned
/// next to whatever else was found.
pub fn scan_prefix(prefix: &Path) -> (Vec<Found>, Vec<String>) {
    let p = PrefixData::new(prefix);
    let mut errors = vec![];
    let mut out = match battlenet::scan(prefix) {
        Ok(v) => v,
        Err(e) => {
            errors.push(format!("Battle.net in {}: {e:#}", prefix.display()));
            vec![]
        }
    };
    out.extend(ea::ea(&p));
    out.extend(stores::ubisoft(&p));
    out.extend(stores::rockstar(&p));
    out.extend(riot_hoyo::riot(&p));
    out.extend(riot_hoyo::hoyoplay(&p));
    (out, errors)
}

/// The store clients installed in a prefix.
pub fn launchers_in(prefix: &Path) -> Vec<(Source, PathBuf)> {
    let p = PrefixData::new(prefix);
    let mut v = vec![];
    v.extend(battlenet::launcher(prefix).map(|e| (Source::BattleNet, e)));
    if p.has_any(ea::EA_DIRS) {
        v.extend(ea::ea_launcher(&p).map(|(e, _)| (Source::Ea, e)));
    }
    if p.has_any(stores::UBISOFT_DIRS) {
        v.extend(stores::ubisoft_launcher(&p).map(|e| (Source::Ubisoft, e)));
    }
    if p.has_any(stores::ROCKSTAR_DIRS) {
        v.extend(stores::rockstar_launcher(&p).map(|e| (Source::Rockstar, e)));
    }
    v.extend(riot_hoyo::riot_client(&p).map(|e| (Source::Riot, e)));
    v.extend(riot_hoyo::hoyo_launcher(&p).map(|e| (Source::HoYoPlay, e)));
    v
}

/// How Lodestar's store-client helper (`client_launch.rs`) starts a found game: every launcher
/// game goes through it, because a request handed over through a new `umu-run` / `proton run`
/// never reaches a client that already runs (its own container), and Battle.net / the EA app
/// only act on a request once they are ready.
pub fn helper_args(prefix: &Prefix, f: &Found) -> crate::client_launch::Args {
    use crate::client_launch::{Args, Kind};
    let exe_name = f.exe.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let generic = |client: &str| {
        let mut a = Args::new(Kind::Generic, &prefix.path, &f.exe, &prefix.runner);
        a.client = client.into();
        a.args = f.args.clone();
        let mains = crate::client_launch::default_main(client);
        a.exe_is_game = !mains.iter().any(|m| m.eq_ignore_ascii_case(&exe_name));
        a
    };
    match f.source {
        Source::BattleNet => {
            let mut a = Args::new(Kind::BattleNet, &prefix.path, &f.exe, &prefix.runner);
            a.code = f.id.clone();
            a
        }
        // The EA app's link handler with ids to ask for (and to retry with).
        Source::Ea if exe_name == "ealauncher.exe" && !f.launch_ids.is_empty() => {
            let mut a = Args::new(Kind::Ea, &prefix.path, &f.exe, &prefix.runner);
            a.ids = f.launch_ids.clone();
            a
        }
        // Origin with a link: the client takes the request itself.
        Source::Ea if exe_name == "origin.exe" => {
            let mut a = generic("ea");
            a.exe_is_game = false;
            a
        }
        // The game's own launcher (it starts the EA app), or the client itself.
        Source::Ea => generic("ea"),
        other => generic(crate::client_launch::client_name(other).unwrap_or("")),
    }
}

fn to_game(_ctx: &ScanContext, prefix: &Prefix, f: Found, source_id: String) -> ScannedGame {
    let launch = helper_args(prefix, &f).spec();
    let mut g = ScannedGame::new(f.source, source_id, f.name);
    g.installed = true;
    g.platform = Some("windows".into());
    g.launch = Some(launch);
    g.install_dir = Some(f.install_dir);
    g
}

/// Scan every known prefix. Prefixes whose runner isn't known from their launcher get one from
/// [`winprefix::resolve_runners`] (only those that hold a store game).
pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    let prefixes = winprefix::find_prefixes(ctx);
    let mut per_prefix: Vec<(Prefix, Vec<Found>)> = vec![];
    // One thread per prefix: a prefix with a store client reads its registry hives (a Proton
    // prefix's system.reg is several MB), and those reads shouldn't add up. Order is kept.
    let scanned: Vec<(Vec<Found>, Vec<String>)> = std::thread::scope(|sc| {
        let handles: Vec<_> = prefixes.iter().map(|p| sc.spawn(|| scan_prefix(&p.path))).collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_else(|_| (vec![], vec!["a prefix scan panicked".into()])))
            .collect()
    });
    for (p, (found, errors)) in prefixes.into_iter().zip(scanned) {
        for e in errors {
            log::warn!("wine launchers: {e}");
        }
        if !found.is_empty() {
            per_prefix.push((p, found));
        }
    }
    let mut ps: Vec<Prefix> = per_prefix.iter().map(|(p, _)| p.clone()).collect();
    winprefix::resolve_runners(ctx, &mut ps);
    let mut seen: HashSet<(Source, String)> = HashSet::new();
    let mut out = vec![];
    for (prefix, (_, found)) in ps.iter().zip(per_prefix) {
        for f in found {
            let id = if seen.insert((f.source, f.id.clone())) {
                f.id.clone()
            } else {
                format!("{}@{}", f.id, winprefix::prefix_hash(&prefix.path))
            };
            if id != f.id && !seen.insert((f.source, id.clone())) {
                continue;
            }
            out.push(to_game(ctx, prefix, f, id));
        }
    }
    Ok(out)
}

/// The command that opens the store client holding `source_id` (for Install / Uninstall, which
/// the client does). `None` when the game's prefix or client is gone. Builds a spec only.
pub fn launcher_command(ctx: &ScanContext, source: Source, source_id: &str) -> Option<LaunchSpec> {
    let (id, hash) = match source_id.rsplit_once('@') {
        Some((id, h)) => (id, Some(h)),
        None => (source_id, None),
    };
    let mut prefixes = winprefix::find_prefixes(ctx);
    prefixes.retain(|p| hash.is_none_or(|h| winprefix::prefix_hash(&p.path) == h));
    let mut hit =
        prefixes.into_iter().find(|p| scan_prefix(&p.path).0.iter().any(|f| f.source == source && f.id == id))?;
    let exe = launchers_in(&hit.path).into_iter().find(|(s, _)| *s == source)?.1;
    winprefix::resolve_runners(ctx, std::slice::from_mut(&mut hit));
    Some(winprefix::run_in_prefix_ctx(ctx, &hit, &exe, &[]))
}

#[cfg(test)]
mod tests;
