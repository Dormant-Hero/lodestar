//! Adding games by hand: understand a picked program, Windows .exe via umu-run + Proton, the
//! "Scan for games" import checklist, duplicate detection, and merging a hand-added game into the
//! same game found later by a scan.
//!
//! CONTRACT (the desktop UI calls these; keep signatures, add freely):
//!   detect_program(&Path) -> anyhow::Result<ProgramInfo>     what a picked file is and how to run it
//!   proton_versions(&ScanContext) -> Vec<ProtonVersion>      Proton / GE / CachyOS builds found (Steam
//!                                                            compatibilitytools.d, steamapps/common/Proton*, umu)
//!   default_prefix(&Paths, name) -> PathBuf                  a new Wine prefix folder for a game
//!   windows_launch(exe, &ProtonVersion, prefix) -> LaunchSpec   umu-run command (GAMEID=umu-default)
//!   find_duplicates(&Library, name, Option<&LaunchSpec>, Option<&Path>) -> Vec<DupMatch>
//!   st.start_import_scan(ImportSource)   background; results in st.import_state (ImportState)
//!   st.cancel_import_scan()
//!   st.merge_suggestions: Vec<MergeSuggestion> (hand-added game ↔ same game found by a scan)
//!   st.merge_games(manual_id, into_id)   move user data (favourite, tags, playtime, edits, art,
//!                                        overrides) from the hand-added game into the scanned one, delete it
//!   st.dismiss_merge(manual_id)
//!
//! Also here:
//!   st.poll_addgame()                    call every frame (from AppState::poll_background): brings in
//!                                        import-scan results, keeps `existing` marks current, and
//!                                        refreshes `merge_suggestions` after each library scan
//!   flatpak_apps(&ScanContext) -> Vec<FlatpakApp>   installed Flatpak apps (Pick a program → Flatpak)
//!   proton_for_prefix(prefix, &[ProtonVersion])      the build a prefix was made with (else newest)
//!   ProtonVersion::flavor()              "CachyOS" / "GE" / "Experimental" / "UMU" / "Valve"
//!
//! Nothing here starts a program or writes outside Lodestar's own database.

use crate::app::AppState;
use crate::db::Db;
use crate::library::Library;
use crate::model::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

mod detect;
mod import;
#[cfg(test)]
mod tests;

pub use detect::{default_prefix, flatpak_apps, norm_name, proton_for_prefix, proton_versions, windows_launch};
pub(crate) use detect::{find_umu, windows_launch_with};
pub use import::prefix_of;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProgramKind {
    /// Native Linux executable (ELF).
    Linux,
    AppImage,
    /// Shell / Python / … script with a shebang.
    Script,
    /// Windows .exe / .bat / .msi — needs Proton / Wine.
    Windows,
    /// A `.desktop` launcher.
    Desktop,
    /// An installed Flatpak app (by app id).
    Flatpak,
}

impl ProgramKind {
    /// Short label for the import checklist's "type" column.
    #[cfg(test)]
    pub fn label(self) -> &'static str {
        match self {
            ProgramKind::Linux => "Linux",
            ProgramKind::AppImage => "AppImage",
            ProgramKind::Script => "Script",
            ProgramKind::Windows => "Windows",
            ProgramKind::Desktop => "Desktop entry",
            ProgramKind::Flatpak => "Flatpak",
        }
    }
}

#[derive(Clone, Debug)]
pub struct ProgramInfo {
    pub kind: ProgramKind,
    /// Suggested game name (from the .desktop Name, AppImage name, exe file name / version info, parent folder …).
    pub name: String,
    pub icon: Option<PathBuf>,
    pub install_dir: Option<PathBuf>,
    /// Ready-to-use launch for non-Windows kinds; None for Windows (needs Proton + prefix choice).
    pub launch: Option<LaunchSpec>,
    pub exe: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtonVersion {
    pub name: String,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportSource {
    /// Executables / AppImages / .exe below a folder.
    Folder,
    /// Windows programs in known Wine prefixes, .desktop games not in the library, Flatpak games.
    Installed,
}

#[derive(Clone, Debug)]
pub struct ImportCandidate {
    pub name: String,
    pub path: PathBuf,
    pub kind: ProgramKind,
    pub launch: Option<LaunchSpec>,
    pub icon: Option<PathBuf>,
    pub install_dir: Option<PathBuf>,
    /// Already in the library (same program or same name) → which game.
    pub existing: Option<GameId>,
    /// Where it was found ("~/Games/foo", "Lutris prefix …", "Flatpak").
    pub origin: String,
}

#[derive(Clone, Debug, Default)]
pub struct ImportState {
    pub running: bool,
    pub source: Option<ImportSource>,
    pub folder: Option<PathBuf>,
    pub candidates: Vec<ImportCandidate>,
    pub scanned_files: usize,
    pub error: Option<String>,
    /// Internal: the running scan's channel and stop flag.
    pub rx: Option<crossbeam_channel::Receiver<import::ImportMsg>>,
    pub cancel: Arc<AtomicBool>,
    /// Internal: library generation the `existing` marks were computed for.
    pub marked_generation: u64,
    /// Internal: merge-suggestion bookkeeping (kept across import scans).
    pub watch: MergeWatch,
}

impl std::fmt::Debug for import::ImportMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            import::ImportMsg::Progress(_) => "Progress",
            import::ImportMsg::Found(_) => "Found",
            import::ImportMsg::Error(_) => "Error",
            import::ImportMsg::Done => "Done",
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct MergeWatch {
    pub was_scanning: bool,
    pub checked_once: bool,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DupReason {
    SameProgram,
    SameName,
    HiddenCopy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DupMatch {
    pub id: GameId,
    pub reason: DupReason,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MergeSuggestion {
    pub manual_id: GameId,
    pub scanned_id: GameId,
}

/// What a picked file is and how to run it: ELF → Linux, AppImage magic / extension, shebang →
/// Script, MZ / .exe / .bat / .msi → Windows (launch left to the Proton + prefix choice), `.desktop`
/// → its Exec / Name / Icon. Reads a few bytes and one folder listing.
pub fn detect_program(path: &Path) -> anyhow::Result<ProgramInfo> {
    let ctx = crate::scan::ScanContext::from_config(&crate::config::Config::default());
    detect::detect_program_in(&ctx, path)
}

// ---------------------------------------------------------------------------------------------
// Duplicates

/// What a launch actually runs, for "same program" comparisons.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ProgKey {
    Exe(PathBuf),
    Flatpak(String),
    Faugus(String),
    Url(String),
    Steam(u32),
    Cmd(String),
}

pub(crate) fn program_key(l: &LaunchSpec) -> Option<ProgKey> {
    match l {
        LaunchSpec::Steam { appid } => Some(ProgKey::Steam(*appid)),
        LaunchSpec::Url(u) => (!u.trim().is_empty()).then(|| ProgKey::Url(u.trim().to_string())),
        LaunchSpec::Command { program, args, env, .. } => {
            if let Some(i) = args.iter().position(|a| a == "--game")
                && (detect::basename(program) == "faugus-launcher" || args.iter().any(|a| a.contains("Faugus")))
            {
                return args.get(i + 1).map(|id| ProgKey::Faugus(id.clone()));
            }
            if let Some(id) = detect::is_flatpak_launch(l) {
                return Some(ProgKey::Flatpak(id));
            }
            let b = detect::basename(program).to_ascii_lowercase();
            let wine_like =
                b == "umu-run" || b == "proton" || b.starts_with("wine") || env.iter().any(|(k, _)| k == "WINEPREFIX");
            if wine_like {
                let exe = args.iter().find(|a| {
                    let l = a.to_ascii_lowercase();
                    l.ends_with(".exe") || l.ends_with(".bat") || l.ends_with(".msi") || l.ends_with(".lnk")
                });
                if let Some(e) = exe {
                    return Some(ProgKey::Exe(PathBuf::from(e)));
                }
            }
            if Path::new(program).is_absolute() {
                return Some(ProgKey::Exe(PathBuf::from(program)));
            }
            let mut s = program.clone();
            for a in args {
                s.push(' ');
                s.push_str(a);
            }
            (!s.trim().is_empty()).then_some(ProgKey::Cmd(s))
        }
    }
}

/// A folder specific enough that two programs in it are the same game (not `~`, `~/Games`,
/// `Downloads`, a prefix's `drive_c` / `Program Files` …).
pub(crate) fn specific_dir(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let n = norm_name(name);
    const BROAD: &[&str] = &[
        "games",
        "goggames",
        "drivec",
        "programfiles",
        "programfilesx86",
        "common",
        "steamapps",
        "downloads",
        "desktop",
        "documents",
        "applications",
        "appimages",
        "bin",
        "opt",
        "tmp",
        "pfx",
        "home",
        "usr",
        "local",
        "share",
        "lutris",
        "heroic",
        "faugus",
        "prefixes",
        "default",
    ];
    p.components().count() >= 4 && !BROAD.contains(&n.as_str())
}

fn strip_slash(p: &Path) -> PathBuf {
    p.components().collect()
}

/// How a library game relates to something about to be added. `exe` is the program file and
/// `dir` its game folder (either may be missing).
fn dup_reason(
    g: &Game,
    name_norm: &str,
    key: Option<&ProgKey>,
    exe: Option<&Path>,
    dir: Option<&Path>,
) -> Option<(u8, DupReason)> {
    let gkey = g.launch.as_ref().and_then(program_key);
    let gdir = g.install_dir.as_deref().map(strip_slash).filter(|d| specific_dir(d));
    let mut program = key.is_some() && gkey.as_ref() == key;
    if let Some(exe) = exe {
        let exe = strip_slash(exe);
        program |= gkey.as_ref() == Some(&ProgKey::Exe(exe.clone()));
        if let Some(gd) = &gdir {
            program |= exe.starts_with(gd);
        }
    }
    if let (Some(d), Some(gd)) = (dir.map(strip_slash).filter(|d| specific_dir(d)), &gdir) {
        program |= d == *gd;
    }
    let same_name =
        !name_norm.is_empty() && (norm_name(g.display_name()) == name_norm || norm_name(&g.name) == name_norm);
    let strength = match (program, same_name) {
        (true, true) => 3,
        (true, false) => 2,
        (false, true) => 1,
        (false, false) => return None,
    };
    let reason = if g.hidden {
        DupReason::HiddenCopy
    } else if program {
        DupReason::SameProgram
    } else {
        DupReason::SameName
    };
    Some((strength, reason))
}

fn dup_matches<'a>(
    games: impl Iterator<Item = &'a Game>,
    name: &str,
    launch: Option<&LaunchSpec>,
    exe: Option<&Path>,
    dir: Option<&Path>,
) -> Vec<DupMatch> {
    let key = launch.and_then(program_key);
    let n = norm_name(name);
    let mut v: Vec<(u8, DupMatch)> = games
        .filter_map(|g| dup_reason(g, &n, key.as_ref(), exe, dir).map(|(s, reason)| (s, DupMatch { id: g.id, reason })))
        .collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.0));
    v.into_iter().map(|(_, m)| m).collect()
}

/// Library games that look like the one about to be added: same program (launch command, exe,
/// AppImage, Flatpak id, .desktop target, game folder), same normalised name; hidden ones are
/// reported as `HiddenCopy` (offer to restore instead). Strongest first. `exe` may be the program
/// file or its folder.
pub fn find_duplicates(lib: &Library, name: &str, launch: Option<&LaunchSpec>, exe: Option<&Path>) -> Vec<DupMatch> {
    let (file, dir) = match exe {
        Some(p) if p.is_dir() => (None, Some(p.to_path_buf())),
        Some(p) => (Some(p), p.parent().map(Path::to_path_buf)),
        None => (None, None),
    };
    // A loose program's folder (Downloads …) says nothing; `specific_dir` filters those.
    dup_matches(lib.games.iter(), name, launch, file, dir.as_deref())
}

/// Set `existing` on candidates from the library.
pub(crate) fn mark_existing(games: &[Game], cands: &mut [ImportCandidate]) {
    for c in cands {
        let exe =
            matches!(c.kind, ProgramKind::Linux | ProgramKind::AppImage | ProgramKind::Script | ProgramKind::Windows)
                .then_some(c.path.as_path());
        let dir = if c.kind == ProgramKind::AppImage { None } else { c.install_dir.as_deref() };
        c.existing = dup_matches(games.iter(), &c.name, c.launch.as_ref(), exe, dir).first().map(|m| m.id);
    }
}

// ---------------------------------------------------------------------------------------------
// Merging a hand-added game into a scanned one

const DISMISSED_KEY: &str = "addgame.dismissed_merges";

fn dismissed(db: &Db) -> HashSet<(GameId, GameId)> {
    db.kv_get(DISMISSED_KEY)
        .and_then(|s| serde_json::from_str::<Vec<(GameId, GameId)>>(&s).ok())
        .unwrap_or_default()
        .into_iter()
        .collect()
}

/// Hand-added games that a scan found too: same program / game folder / normalised name.
/// One suggestion per hand-added game (the best match), skipping dismissed pairs.
pub(crate) fn compute_merge_suggestions(games: &[Game], dismissed: &HashSet<(GameId, GameId)>) -> Vec<MergeSuggestion> {
    let scanned: Vec<&Game> = games.iter().filter(|g| g.source != Source::Manual && !g.hidden).collect();
    let mut out = vec![];
    // Cloud games are never the same game as an installed copy.
    for m in games.iter().filter(|g| g.source == Source::Manual && !g.is_cloud()) {
        let exe = match m.launch.as_ref().and_then(program_key) {
            Some(ProgKey::Exe(p)) => Some(p),
            _ => None,
        };
        let matches = dup_matches(
            scanned.iter().copied(),
            m.display_name(),
            m.launch.as_ref(),
            exe.as_deref(),
            m.install_dir.as_deref(),
        );
        if let Some(best) = matches.into_iter().find(|d| !dismissed.contains(&(m.id, d.id))) {
            out.push(MergeSuggestion { manual_id: m.id, scanned_id: best.id });
        }
    }
    out
}

/// Fill the fields of `into` that are still at their default from `from` (by JSON key), except
/// the install fields (the scanned game knows its own folder).
fn merge_edits(into: &UserEdits, from: &UserEdits) -> UserEdits {
    let (Ok(serde_json::Value::Object(mut a)), Ok(serde_json::Value::Object(b)), Ok(serde_json::Value::Object(d))) =
        (serde_json::to_value(into), serde_json::to_value(from), serde_json::to_value(UserEdits::default()))
    else {
        return into.clone();
    };
    for (k, v) in b {
        if matches!(k.as_str(), "install_dir" | "installed" | "install_size") {
            continue;
        }
        let default = d.get(&k);
        if a.get(&k) == default && Some(&v) != default {
            a.insert(k, v);
        }
    }
    serde_json::from_value(serde_json::Value::Object(a)).unwrap_or_else(|_| into.clone())
}

/// Move the user data of hand-added `manual` into scanned `into` and delete `manual`.
/// Favourite (either), hidden (the hand-added game's), tags (both), custom name, rating,
/// controller setting and launch options (the hand-added game's when the scanned one has none),
/// playtime and play count (summed), last played (latest), date added (earliest), hand edits and
/// user art (filling what the scanned game doesn't have).
pub(crate) fn merge_in_db(db: &Db, manual: &Game, into: &Game) -> anyhow::Result<()> {
    let id = into.id;
    db.set_favorite(id, into.favorite || manual.favorite)?;
    db.set_hidden(id, manual.hidden)?;
    let mut tags = into.tags.clone();
    tags.extend(manual.tags.iter().cloned());
    db.set_tags(id, &crate::library::normalize_tags(tags))?;
    if into.user_name.is_none()
        && let Some(n) = manual.user_name.as_deref()
    {
        db.set_user_name(id, Some(n))?;
    }
    if into.user_rating.is_none() && manual.user_rating.is_some() {
        db.set_rating(id, manual.user_rating)?;
    }
    if into.controller_override.is_none() && manual.controller_override.is_some() {
        db.set_controller_override(id, manual.controller_override)?;
    }
    if into.launch_override.is_none() && manual.launch_override.is_some() {
        db.set_launch_override(id, manual.launch_override.as_ref())?;
    }
    if manual.playtime_secs > 0 || manual.play_count > 0 || manual.last_played.is_some() {
        let last = into.last_played.max(manual.last_played);
        db.set_play_stats(id, into.playtime_secs + manual.playtime_secs, into.play_count + manual.play_count, last)?;
    }
    if manual.added > 0 && manual.added < into.added {
        db.set_added(id, manual.added)?;
    }
    if !manual.edits.is_empty() {
        db.set_edits(id, &merge_edits(&into.edits, &manual.edits))?;
    }
    let manual_art = db.user_art(manual.id)?;
    if !manual_art.is_empty() {
        let mut art = db.user_art(id)?;
        art.fill_from(&manual_art);
        db.set_user_art(id, &art)?;
    }
    crate::library::user::replace_game(db, manual.id, Some(id));
    db.delete(manual.id)?;
    Ok(())
}

impl AppState {
    /// Start "Scan for games" in the background (a running one is cancelled first). Folder
    /// scans need `folder`. Results stream into `import_state.candidates`.
    pub fn start_import_scan(&mut self, src: ImportSource, folder: Option<PathBuf>) {
        self.cancel_import_scan();
        let cancel = Arc::new(AtomicBool::new(false));
        let ctx = crate::scan::ScanContext::from_config(&self.config);
        let rx = import::spawn(src, folder.clone(), ctx, cancel.clone(), self.ctx().clone());
        let watch = std::mem::take(&mut self.import_state.watch);
        self.import_state = ImportState {
            running: true,
            source: Some(src),
            folder,
            rx: Some(rx),
            cancel,
            marked_generation: self.library.generation,
            watch,
            ..Default::default()
        };
    }

    /// Stop a running import scan (what it found so far stays listed).
    pub fn cancel_import_scan(&mut self) {
        self.import_state.cancel.store(true, Ordering::Relaxed);
        self.import_state.rx = None;
        self.import_state.running = false;
    }

    /// Per-frame bookkeeping (cheap when idle): import-scan messages, `existing` marks after
    /// library changes, merge suggestions after each library scan.
    pub fn poll_addgame(&mut self) {
        if let Some(rx) = self.import_state.rx.clone() {
            while let Ok(msg) = rx.try_recv() {
                match msg {
                    import::ImportMsg::Progress(n) => self.import_state.scanned_files = n,
                    import::ImportMsg::Found(mut v) => {
                        let have: HashSet<PathBuf> =
                            self.import_state.candidates.iter().map(|c| c.path.clone()).collect();
                        v.retain(|c| !have.contains(&c.path));
                        mark_existing(&self.library.games, &mut v);
                        self.import_state.candidates.extend(v);
                    }
                    import::ImportMsg::Error(e) => self.import_state.error = Some(e),
                    import::ImportMsg::Done => {
                        self.import_state.running = false;
                        self.import_state.rx = None;
                        // Not re-sorted: the UI takes new candidates by index (append-only).
                        break;
                    }
                }
            }
        }
        let generation = self.library.generation;
        if self.import_state.marked_generation != generation {
            self.import_state.marked_generation = generation;
            mark_existing(&self.library.games, &mut self.import_state.candidates);
        }
        let scanning = self.scan.running;
        let w = &mut self.import_state.watch;
        let scan_finished = w.was_scanning && !scanning;
        let first = !w.checked_once && !scanning;
        let lib_changed = w.generation != generation;
        w.was_scanning = scanning;
        w.generation = generation;
        if scan_finished || first {
            self.import_state.watch.checked_once = true;
            self.refresh_merge_suggestions();
        } else if lib_changed && !self.merge_suggestions.is_empty() {
            // A game of a suggestion was removed: drop it.
            let lib = &self.library;
            self.merge_suggestions.retain(|s| lib.get(s.manual_id).is_some() && lib.get(s.scanned_id).is_some());
        }
    }

    /// Recompute `merge_suggestions` from the library (a few hundred games: well under 1 ms).
    pub fn refresh_merge_suggestions(&mut self) {
        let d = dismissed(&self.db);
        self.merge_suggestions = compute_merge_suggestions(&self.library.games, &d);
    }

    pub fn merge_games(&mut self, manual_id: GameId, into: GameId) {
        let (Some(manual), Some(target)) = (self.library.get(manual_id).cloned(), self.library.get(into).cloned())
        else {
            return;
        };
        if manual_id == into {
            return;
        }
        match merge_in_db(&self.db, &manual, &target) {
            Ok(()) => {
                match self.db.load_all() {
                    Ok(games) => self.set_games(games),
                    Err(e) => self.notify_error(format!("Couldn't read library: {e:#}")),
                }
                self.merge_suggestions.retain(|s| s.manual_id != manual_id);
                self.notify(if manual.display_name() == target.display_name() {
                    format!("Merged the hand-added {} into the {} game", manual.display_name(), target.source.display())
                } else {
                    format!("Merged {} into {}", manual.display_name(), target.display_name())
                });
            }
            Err(e) => self.notify_error(format!("Couldn't merge {}: {e:#}", manual.display_name())),
        }
    }

    /// "Keep both": never suggest merging this hand-added game with the games suggested now.
    pub fn dismiss_merge(&mut self, manual_id: GameId) {
        let mut d = dismissed(&self.db);
        for s in self.merge_suggestions.iter().filter(|s| s.manual_id == manual_id) {
            d.insert((s.manual_id, s.scanned_id));
        }
        let mut v: Vec<(GameId, GameId)> = d.into_iter().collect();
        v.sort();
        if let Err(e) =
            serde_json::to_string(&v).map_err(anyhow::Error::from).and_then(|s| self.db.kv_set(DISMISSED_KEY, &s))
        {
            self.notify_error(format!("Couldn't save: {e:#}"));
        }
        self.merge_suggestions.retain(|s| s.manual_id != manual_id);
    }
}
