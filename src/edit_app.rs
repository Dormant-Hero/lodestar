//! AppState side of Edit Game Details, extra actions, links, shortcuts and adding games.
//!
//! CONTRACT (the desktop / controller UIs call these; keep signatures, add freely):
//!   st.set_edits(id, UserEdits)                      save hand edits (empty = clear); library updates at once
//!   st.set_completion(id, Option<CompletionStatus>)  quick setter (More menu)
//!   st.set_play_stats(id, playtime_secs, play_count, Option<last_played>)
//!   st.set_added(id, unix_secs)
//!   st.launch_action(id, index)                      run `edits.actions[index]` (tracked per its TrackingMode)
//!   st.calc_install_size(id)                         background folder walk → edits.install_size + toast
//!   st.size_calculating(id) -> bool
//!   st.create_desktop_shortcut(id)                   ~/Desktop/<name>.desktop running `lodestar --play <id>`
//!   st.open_url(&str)                                xdg-open in the background (links, manual)
//!   st.open_path(&Path)                              open a folder / file with the default app
//!   st.add_game(NewGame) -> Option<GameId>           add a hand-made game (Source::Manual), reload library
//!   st.test_script(&str)                             run a script now in the background; result as toast
//!   st.test_script_for(Option<GameId>, &str)         the same with that game's LODESTAR_* variables and folder
//! `desktop_dir`, `write_desktop_shortcut` and `dir_size` are pure helpers: the directory is a parameter, so
//! tests write to temp dirs.
//! Launching (launch.rs) honours: the Play action (Game::play_action) instead of the source launch, HDR env,
//! per-game + global scripts (Settings), TrackingMode::Folder, and sets completion to Playing on first launch
//! when it was unset / Not played / Plan to play (config.auto_playing, default on).

use crate::app::AppState;
use crate::model::*;
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// A game the user adds by hand (Add Game → Manually / Pick a program / Scan for games).
#[derive(Clone, Debug, Default)]
pub struct NewGame {
    pub name: String,
    pub launch: Option<LaunchSpec>,
    pub art: Art,
    pub install_dir: Option<PathBuf>,
    pub platform: Option<String>,
    pub edits: UserEdits,
}

/// Background work started from Edit Game Details / the More menu.
pub struct EditBg {
    tx: Sender<EditMsg>,
    rx: Receiver<EditMsg>,
    sizing: HashSet<GameId>,
}

impl Default for EditBg {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        EditBg { tx, rx, sizing: HashSet::new() }
    }
}

enum EditMsg {
    Size { id: GameId, result: Result<DirSize, String> },
}

impl AppState {
    /// Save the hand edits of a game (empty = back to the source's values). The game is reloaded
    /// from the DB, so the edits show everywhere at once.
    pub fn set_edits(&mut self, id: GameId, e: UserEdits) {
        self.save_edits(id, e, true);
    }

    /// `set_edits`; `announce`: say when ROM entries folded into the game (Add as ROM says it
    /// its own way).
    pub(crate) fn save_edits(&mut self, id: GameId, e: UserEdits, announce: bool) {
        let added_roms = e.roms.iter().any(|r| !r.path.trim().is_empty());
        if let Err(err) = self.db.set_edits(id, &e) {
            self.notify_error(format!("Couldn't save the changes: {err:#}"));
            return;
        }
        self.reload_game(id);
        // ROMs added to it that were games of their own: those entries fold into this one.
        if added_roms {
            self.fold_claimed_roms(id, announce);
        }
    }

    /// Read one game back from the DB (its edits applied) into the library.
    fn reload_game(&mut self, id: GameId) {
        match self.db.get(id) {
            Ok(Some(g)) => self.library.update(id, |old| *old = g),
            Ok(None) => {}
            Err(e) => self.notify_error(format!("Couldn't read the game back: {e:#}")),
        }
    }

    pub fn set_completion(&mut self, id: GameId, c: Option<CompletionStatus>) {
        let Some(mut e) = self.library.get(id).map(|g| g.edits.clone()) else { return };
        if e.completion == c {
            return;
        }
        e.completion = c;
        self.set_edits(id, e);
    }

    /// Completion statuses of several games in one save (multi-selection, its Undo).
    pub fn set_completion_many(&mut self, set: &[(GameId, Option<CompletionStatus>)]) {
        let edits: Vec<(GameId, UserEdits)> = set
            .iter()
            .filter_map(|&(id, c)| {
                let g = self.library.get(id)?;
                (g.edits.completion != c).then(|| {
                    let mut e = g.edits.clone();
                    e.completion = c;
                    (id, e)
                })
            })
            .collect();
        if edits.is_empty() {
            return;
        }
        if let Err(err) = self.db.set_edits_many(&edits) {
            self.notify_error(format!("Couldn't save the changes: {err:#}"));
            return;
        }
        for (id, e) in edits {
            self.library.update(id, |g| g.edits.completion = e.completion);
        }
    }

    /// User scores (0..=100, None = not set) of several games in one save (right-click →
    /// User score, its Undo).
    pub fn set_user_score_many(&mut self, set: &[(GameId, Option<u8>)]) {
        let edits: Vec<(GameId, UserEdits)> = set
            .iter()
            .filter_map(|&(id, s)| {
                let g = self.library.get(id)?;
                let s = s.map(|v| v.min(100));
                (g.edits.user_score != s).then(|| {
                    let mut e = g.edits.clone();
                    e.user_score = s;
                    (id, e)
                })
            })
            .collect();
        if edits.is_empty() {
            return;
        }
        if let Err(err) = self.db.set_edits_many(&edits) {
            self.notify_error(format!("Couldn't save the changes: {err:#}"));
            return;
        }
        for (id, e) in edits {
            self.library.update(id, |g| g.edits.user_score = e.user_score);
        }
    }

    pub fn set_play_stats(&mut self, id: GameId, playtime_secs: u64, play_count: u32, last_played: Option<i64>) {
        match self.db.set_play_stats(id, playtime_secs, play_count, last_played) {
            Ok(()) => self.library.update(id, |g| {
                g.playtime_secs = playtime_secs;
                g.play_count = play_count;
                g.last_played = last_played;
            }),
            Err(e) => self.notify_error(format!("Couldn't save the play statistics: {e:#}")),
        }
    }

    pub fn set_added(&mut self, id: GameId, added: i64) {
        match self.db.set_added(id, added) {
            Ok(()) => self.library.update(id, |g| g.added = added),
            Err(e) => self.notify_error(format!("Couldn't save the date added: {e:#}")),
        }
    }

    /// Run the game's action `index` (the Play action counts as Play).
    pub fn launch_action(&mut self, id: GameId, index: usize) {
        self.start_game(id, Some(index));
    }

    /// Add up the size of the install folder in the background; the result is saved as
    /// `edits.install_size` and shown as a toast.
    pub fn calc_install_size(&mut self, id: GameId) {
        let Some(g) = self.library.get(id) else { return };
        let name = g.display_name().to_string();
        let Some(dir) = g.install_dir.clone() else {
            self.notify_error(format!("{name}: no install folder known — set one on the Installation tab"));
            return;
        };
        if !dir.is_dir() {
            self.notify_error(format!("{name}: the install folder doesn't exist ({})", dir.display()));
            return;
        }
        if !self.edit_bg.sizing.insert(id) {
            return;
        }
        let tx = self.edit_bg.tx.clone();
        let ctx = self.ctx().clone();
        let spawned = std::thread::Builder::new().name("install-size".into()).spawn(move || {
            let result = dir_size(&dir, SIZE_ENTRY_CAP).map_err(|e| format!("{e:#}"));
            let _ = tx.send(EditMsg::Size { id, result });
            ctx.request_repaint();
        });
        if let Err(e) = spawned {
            self.edit_bg.sizing.remove(&id);
            self.notify_error(format!("Couldn't start the size calculation: {e}"));
        }
    }

    pub fn size_calculating(&self, id: GameId) -> bool {
        self.edit_bg.sizing.contains(&id)
    }

    /// A `.desktop` file on the desktop that starts the game through `lodestar --play <id>`.
    pub fn create_desktop_shortcut(&mut self, id: GameId) {
        let Some(g) = self.library.get(id).cloned() else { return };
        let exe = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => {
                self.notify_error(format!("Couldn't find Lodestar's own program: {e}"));
                return;
            }
        };
        let dir = desktop_dir(&self.paths.home);
        match write_desktop_shortcut(&dir, &g, &exe) {
            Ok(path) => {
                let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
                self.notify(format!("Desktop shortcut created: {file}"));
            }
            Err(e) => self.notify_error(format!("Couldn't create the desktop shortcut: {e:#}")),
        }
    }

    /// Open a web link (http / https) or a `file:` URL with the default application.
    pub fn open_url(&mut self, url: &str) {
        match checked_url(url) {
            Ok(u) if self.no_launch => self.skip_open(u),
            Ok(u) => spawn_xdg_open(u.into()),
            Err(e) => self.notify_error(e),
        }
    }

    /// Open a folder or file with the default application.
    pub fn open_path(&mut self, p: &Path) {
        if !p.exists() {
            self.notify_error(format!("Not found: {}", p.display()));
            return;
        }
        if self.no_launch {
            self.skip_open(&p.to_string_lossy());
            return;
        }
        spawn_xdg_open(p.as_os_str().to_os_string());
    }

    /// Screenshot runs never open anything (a stray injected click must not start a browser,
    /// or — through xdg-open on a game file — a game): log and say what would have opened.
    fn skip_open(&mut self, what: &str) {
        log::warn!("screenshot run: not opening {what}");
        self.notify(format!("Open skipped (screenshot run): {what}"));
    }

    /// Add a game by hand (Source::Manual). Its platform and install folder go into its edits
    /// (so they show as the user's values). Returns the new game's id.
    pub fn add_game(&mut self, g: NewGame) -> Option<GameId> {
        let name = g.name.trim().to_string();
        if name.is_empty() {
            self.notify_error("A game needs a name");
            return None;
        }
        // No command yet (Add Game → Manually): launching says what to set until it has one.
        let launch = g.launch.unwrap_or_else(|| LaunchSpec::Command {
            program: String::new(),
            args: vec![],
            cwd: None,
            env: vec![],
        });
        let id = match self.db.add_manual(&name, launch, g.art) {
            Ok(id) => id,
            Err(e) => {
                self.notify_error(format!("Couldn't add {name}: {e:#}"));
                return None;
            }
        };
        let mut e = g.edits;
        if e.platform.is_none() {
            e.platform = g.platform.filter(|p| !p.trim().is_empty());
        }
        if e.install_dir.is_none() {
            e.install_dir = g.install_dir;
        }
        if !e.is_empty()
            && let Err(err) = self.db.set_edits(id, &e)
        {
            self.notify_error(format!("{name} was added, but its details weren't saved: {err:#}"));
        }
        self.reload_library();
        Some(id)
    }

    /// Run a script now (the Scripts tab's Test button); the result comes as a toast.
    pub fn test_script(&mut self, script: &str) {
        self.test_script_for(None, script);
    }

    /// `test_script` with a game's `LODESTAR_*` variables and install folder.
    pub fn test_script_for(&mut self, id: Option<GameId>, script: &str) {
        if script.trim().is_empty() {
            self.notify("The script is empty");
            return;
        }
        if self.no_launch {
            log::warn!("screenshot run: not running a test script");
            self.notify("Test skipped (screenshot run)");
            return;
        }
        let game = id.and_then(|id| self.library.get(id));
        let (env, cwd) = match game {
            Some(g) => (crate::launch::scripts::script_env(g), g.install_dir.clone().filter(|d| d.is_dir())),
            None => (vec![], None),
        };
        let s = crate::launch::scripts::Script { label: "Test script".into(), text: script.to_string() };
        self.launcher.run_scripts(vec![s], env, cwd);
        self.notify("Running the script…");
    }

    /// Called from `poll_background` every frame (non-blocking).
    pub(crate) fn poll_edit(&mut self) {
        while let Ok(msg) = self.edit_bg.rx.try_recv() {
            match msg {
                EditMsg::Size { id, result } => {
                    self.edit_bg.sizing.remove(&id);
                    let Some(g) = self.library.get(id) else { continue };
                    let name = g.display_name().to_string();
                    match result {
                        Ok(sz) => {
                            let mut e = g.edits.clone();
                            e.install_size = Some(sz.bytes);
                            self.set_edits(id, e);
                            let more =
                                if sz.truncated { " (at least — stopped counting, very many files)" } else { "" };
                            self.notify(format!("{name}: {}{more}", crate::theme::render::bind::format_size(sz.bytes)));
                        }
                        Err(e) => self.notify_error(format!("{name}: couldn't calculate the install size: {e}")),
                    }
                }
            }
        }
    }
}

// ----- install size -----

/// Entries a size calculation looks at before it stops (a game folder with more is unusual).
pub const SIZE_ENTRY_CAP: usize = 2_000_000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirSize {
    pub bytes: u64,
    pub files: u64,
    /// Stopped at the entry cap: `bytes` is a lower bound.
    pub truncated: bool,
}

/// Total size of the files below `root` (apparent sizes). Never follows symlinks (so no loops,
/// nothing outside the folder), counts hard-linked files once, skips what it can't read.
pub fn dir_size(root: &Path, cap: usize) -> anyhow::Result<DirSize> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(root)?;
    if !meta.is_dir() {
        anyhow::bail!("not a folder: {}", root.display());
    }
    let mut out = DirSize::default();
    let mut seen_inodes: HashSet<(u64, u64)> = HashSet::new();
    let mut stack = vec![root.to_path_buf()];
    let mut entries = 0usize;
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for ent in rd.flatten() {
            entries += 1;
            if entries > cap {
                out.truncated = true;
                return Ok(out);
            }
            // DirEntry::metadata doesn't follow symlinks.
            let Ok(m) = ent.metadata() else { continue };
            let ft = m.file_type();
            if ft.is_dir() {
                stack.push(ent.path());
            } else if ft.is_file() {
                if m.nlink() > 1 && !seen_inodes.insert((m.dev(), m.ino())) {
                    continue;
                }
                out.bytes += m.len();
                out.files += 1;
            }
        }
    }
    Ok(out)
}

// ----- desktop shortcuts -----

/// The desktop folder: `XDG_DESKTOP_DIR` from `user-dirs.dirs` (what `xdg-user-dir DESKTOP`
/// reads), else `~/Desktop`. Test runs (`LODESTAR_HOME`) use `$LODESTAR_HOME/Desktop`, so they
/// never touch the real desktop.
pub fn desktop_dir(home: &Path) -> PathBuf {
    if let Some(root) = std::env::var_os("LODESTAR_HOME") {
        return PathBuf::from(root).join("Desktop");
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    std::fs::read_to_string(config.join("user-dirs.dirs"))
        .ok()
        .and_then(|s| parse_user_dirs_desktop(&s, home))
        .unwrap_or_else(|| home.join("Desktop"))
}

/// `XDG_DESKTOP_DIR="$HOME/Desktop"` → `<home>/Desktop`.
fn parse_user_dirs_desktop(s: &str, home: &Path) -> Option<PathBuf> {
    let line = s.lines().map(str::trim).find(|l| l.starts_with("XDG_DESKTOP_DIR="))?;
    let v = line["XDG_DESKTOP_DIR=".len()..].trim().trim_matches('"');
    let p = match v.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None if v.starts_with('/') => PathBuf::from(v),
        None => return None,
    };
    // `XDG_DESKTOP_DIR="$HOME/"` means "no desktop folder": use the default.
    (p != home).then_some(p)
}

/// File name for a game's shortcut: its name without characters file managers choke on.
pub fn shortcut_file_name(name: &str, id: GameId) -> String {
    let clean: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { ' ' } else { c }).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean = clean.trim_start_matches('.').trim();
    let clean: String = clean.chars().take(100).collect();
    if clean.is_empty() { format!("lodestar-game-{id}.desktop") } else { format!("{clean}.desktop") }
}

/// Write `<dir>/<name>.desktop` starting `exe --play <id>` (executable, as KDE / GNOME want for
/// launchers on the desktop). A file of that name that isn't this game's shortcut is kept: the
/// new one gets the id in its name. Returns the file written.
pub fn write_desktop_shortcut(dir: &Path, game: &Game, exe: &Path) -> anyhow::Result<PathBuf> {
    use anyhow::Context as _;
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let marker = format!("X-Lodestar-Game-Id={}", game.id);
    let ours = |p: &Path| std::fs::read_to_string(p).is_ok_and(|s| s.lines().any(|l| l.trim() == marker));
    let mut path = dir.join(shortcut_file_name(game.display_name(), game.id));
    if path.exists() && !ours(&path) {
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        path = dir.join(format!("{stem} ({}).desktop", game.id));
    }
    let icon = game
        .art
        .icon
        .as_ref()
        .or(game.art.cover.as_ref())
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "lodestar".into());
    let name = desktop_string(game.display_name());
    let exec = [exe.to_string_lossy().into_owned(), "--play".into(), game.id.to_string()]
        .iter()
        .map(|a| exec_quote(a))
        .collect::<Vec<_>>()
        .join(" ");
    let text = format!(
        "[Desktop Entry]\nType=Application\nVersion=1.0\nName={name}\nComment={}\nExec={}\nIcon={}\nTerminal=false\nCategories=Game;\nStartupNotify=false\n{marker}\n",
        desktop_string(&format!("Play {} with Lodestar", game.display_name())),
        desktop_string(&exec).replace('%', "%%"),
        desktop_string(&icon),
    );
    crate::fsutil::write_atomic(&path, text.as_bytes()).with_context(|| format!("writing {}", path.display()))?;
    // Desktops start only executable launchers.
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(path)
}

/// A `.desktop` string value: backslashes and line breaks escaped.
fn desktop_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

/// Quote one `Exec=` argument per the Desktop Entry spec (double quotes; `"`, `` ` ``, `$`, `\`
/// escaped inside). The string-value escaping (`desktop_string`) and `%%` come on top.
fn exec_quote(a: &str) -> String {
    const RESERVED: &str = " \t\n\"'\\><~|&;$*?#()`";
    if !a.is_empty() && !a.chars().any(|c| RESERVED.contains(c)) {
        return a.to_string();
    }
    let mut out = String::from("\"");
    for c in a.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

// ----- opening links / files -----

/// Links the app opens: web pages and local files only (not `javascript:`, custom handlers …).
pub fn checked_url(url: &str) -> Result<&str, String> {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("file://") {
        if u.chars().any(char::is_control) {
            return Err("That link isn't valid".into());
        }
        Ok(u)
    } else if u.is_empty() {
        Err("The link is empty".into())
    } else {
        Err(format!("Only web (http / https) and file links can be opened: {u}"))
    }
}

/// `xdg-open <target>` without blocking; the child is reaped on a helper thread.
fn spawn_xdg_open(target: std::ffi::OsString) {
    let spawned =
        std::thread::Builder::new().name("xdg-open".into()).spawn(move || {
            match std::process::Command::new("xdg-open")
                .arg(&target)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                Ok(mut child) => {
                    let _ = child.wait();
                }
                Err(e) => log::warn!("xdg-open {}: {e}", target.to_string_lossy()),
            }
        });
    if let Err(e) = spawned {
        log::warn!("couldn't start xdg-open: {e}");
    }
}

#[cfg(test)]
#[allow(clippy::field_reassign_with_default)]
mod tests {
    use super::*;
    use crate::db::Db;
    use std::time::Duration;

    fn scanned(st: &mut AppState, g: ScannedGame) -> GameId {
        let mut db = Db::open(&st.paths.db_file()).unwrap();
        db.merge_scan(std::slice::from_ref(&g), &[g.source]).unwrap();
        st.reload_library();
        st.library.games.iter().find(|x| x.source_id == g.source_id).unwrap().id
    }

    fn sh_action(name: &str, script: &str, play: bool) -> GameAction {
        GameAction {
            name: name.into(),
            kind: ActionKind::Command,
            path: "sh".into(),
            args: format!("-c {}", crate::launch::shell_quote(script)),
            play,
            ..Default::default()
        }
    }

    fn wait_exit(st: &mut AppState) {
        assert!(st.poll_until(Duration::from_secs(15), |s| s.launcher.running().is_none()), "never exited");
    }

    #[test]
    fn completion_of_several_games_in_one_save() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let mut sg = ScannedGame::new(Source::Steam, "620", "Portal 2");
        sg.meta.developer = Some("Valve".into());
        let a = scanned(&mut st, sg);
        let b = scanned(&mut st, ScannedGame::new(Source::Steam, "400", "Portal"));
        st.set_edits(a, UserEdits { notes: "hi".into(), ..Default::default() });
        st.set_completion_many(&[(a, Some(CompletionStatus::Beaten)), (b, Some(CompletionStatus::Beaten))]);
        for id in [a, b] {
            assert_eq!(st.library.get(id).unwrap().completion(), Some(CompletionStatus::Beaten));
        }
        assert_eq!(st.library.get(a).unwrap().edits.notes, "hi", "other edits kept");
        // Saved: a reload reads the same.
        st.reload_library();
        assert_eq!(st.library.get(b).unwrap().completion(), Some(CompletionStatus::Beaten));
        // The Undo puts each one back.
        st.set_completion_many(&[(a, None), (b, Some(CompletionStatus::Playing))]);
        st.reload_library();
        assert_eq!(st.library.get(a).unwrap().completion(), None);
        assert_eq!(st.library.get(b).unwrap().completion(), Some(CompletionStatus::Playing));
    }

    #[test]
    fn edits_apply_at_once_and_clear() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let mut sg = ScannedGame::new(Source::Steam, "620", "Portal 2");
        sg.meta.developer = Some("Valve".into());
        let id = scanned(&mut st, sg.clone());
        let e = UserEdits { developer: Some("Me".into()), sort_name: Some("aaa".into()), ..Default::default() };
        st.set_edits(id, e);
        let g = st.library.get(id).unwrap();
        assert_eq!(g.meta.developer.as_deref(), Some("Me"));
        assert_eq!(g.source_meta.developer.as_deref(), Some("Valve"), "source value kept for reset");
        assert_eq!(g.sort_name, "aaa");
        st.set_completion(id, Some(CompletionStatus::Beaten));
        assert_eq!(st.library.get(id).unwrap().completion(), Some(CompletionStatus::Beaten));
        assert_eq!(st.library.get(id).unwrap().edits.developer.as_deref(), Some("Me"), "other edits kept");
        // A rescan doesn't undo them.
        scanned(&mut st, sg);
        assert_eq!(st.library.get(id).unwrap().meta.developer.as_deref(), Some("Me"));
        st.set_edits(id, UserEdits::default());
        let g = st.library.get(id).unwrap();
        assert_eq!(g.meta.developer.as_deref(), Some("Valve"));
        assert!(g.edits.is_empty());
    }

    #[test]
    fn play_stats_and_added_date() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let id = scanned(&mut st, ScannedGame::new(Source::Steam, "1", "A"));
        st.set_play_stats(id, 3600, 4, Some(1_700_000_000));
        st.set_added(id, 1_600_000_000);
        for g in [st.library.get(id).unwrap().clone(), st.db.get(id).unwrap().unwrap()] {
            assert_eq!(
                (g.playtime_secs, g.play_count, g.last_played, g.added),
                (3600, 4, Some(1_700_000_000), 1_600_000_000)
            );
        }
    }

    #[test]
    fn add_game_stores_platform_and_folder_as_edits() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        assert_eq!(st.add_game(NewGame { name: "  ".into(), ..Default::default() }), None);
        assert!(st.toasts.iter().any(|t| t.error));
        let folder = dir.path().join("game");
        let id = st
            .add_game(NewGame {
                name: " My Game ".into(),
                launch: Some(LaunchSpec::Command { program: "/bin/true".into(), args: vec![], cwd: None, env: vec![] }),
                install_dir: Some(folder.clone()),
                platform: Some("linux".into()),
                edits: UserEdits { notes: "hand-made".into(), ..Default::default() },
                ..Default::default()
            })
            .unwrap();
        let g = st.library.get(id).unwrap();
        assert_eq!((g.display_name(), g.source), ("My Game", Source::Manual));
        assert_eq!(g.platform.as_deref(), Some("linux"));
        assert_eq!(g.install_dir.as_deref(), Some(folder.as_path()));
        assert_eq!(g.edits.notes, "hand-made");
        // Added without a command: launching explains instead of running something.
        let bare = st.add_game(NewGame { name: "Bare".into(), ..Default::default() }).unwrap();
        st.launch(bare);
        assert!(st.toasts.iter().any(|t| t.error && t.text.contains("custom command")), "{:?}", st.toasts);
        assert_eq!(st.library.get(bare).unwrap().play_count, 0);
    }

    #[test]
    fn play_action_counts_as_play_other_actions_do_not() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let id = st.add_game(NewGame { name: "Acts".into(), ..Default::default() }).unwrap();
        let played = dir.path().join("played");
        let settings = dir.path().join("settings");
        let mut e = st.library.get(id).unwrap().edits.clone();
        e.actions = vec![
            sh_action("Settings", &format!("touch '{}'", settings.display()), false),
            sh_action("Play", &format!("sleep 1.1; touch '{}'", played.display()), true),
        ];
        st.set_edits(id, e);
        st.launch_action(id, 0);
        wait_exit(&mut st);
        assert!(settings.exists());
        let g = st.library.get(id).unwrap();
        assert_eq!((g.play_count, g.playtime_secs, g.completion()), (0, 0, None), "a settings tool isn't play");
        st.launch(id);
        assert_eq!(st.library.get(id).unwrap().completion(), Some(CompletionStatus::Playing), "auto Playing");
        wait_exit(&mut st);
        assert!(played.exists(), "Play ran the Play action");
        let g = st.db.get(id).unwrap().unwrap();
        assert_eq!(g.play_count, 1);
        assert!(g.playtime_secs >= 1, "{}", g.playtime_secs);
        assert_eq!(g.completion(), Some(CompletionStatus::Playing));
        // Out of range: nothing happens.
        st.launch_action(id, 9);
        assert!(st.launcher.running().is_none());
    }

    #[test]
    fn auto_playing_respects_a_set_status_and_the_setting() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let id = st.add_game(NewGame { name: "G".into(), ..Default::default() }).unwrap();
        let mut e = UserEdits::default();
        e.actions = vec![sh_action("Play", "exit 0", true)];
        e.completion = Some(CompletionStatus::Completed);
        st.set_edits(id, e.clone());
        st.launch(id);
        wait_exit(&mut st);
        assert_eq!(st.library.get(id).unwrap().completion(), Some(CompletionStatus::Completed));
        e.completion = Some(CompletionStatus::PlanToPlay);
        st.set_edits(id, e);
        st.config.auto_playing = false;
        st.launch(id);
        wait_exit(&mut st);
        assert_eq!(st.library.get(id).unwrap().completion(), Some(CompletionStatus::PlanToPlay));
    }

    #[test]
    fn hdr_scripts_and_their_environment() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let folder = dir.path().join("Game Folder");
        std::fs::create_dir_all(&folder).unwrap();
        let id = st
            .add_game(NewGame { name: "Scripted".into(), install_dir: Some(folder.clone()), ..Default::default() })
            .unwrap();
        let out = |n: &str| dir.path().join(n);
        let mut e = st.library.get(id).unwrap().edits.clone();
        e.game_options.hdr = crate::game_options::Tri::On;
        e.actions = vec![sh_action(
            "Play",
            &format!("echo \"$DXVK_HDR $PROTON_ENABLE_HDR $ENABLE_HDR_WSI\" > '{}'; sleep 0.5", out("env").display()),
            true,
        )];
        e.scripts = GameScripts {
            before_start: format!(
                "sleep 0.3; echo \"$LODESTAR_GAME_NAME|$LODESTAR_GAME_ID|$LODESTAR_INSTALL_DIR|$PWD\" > '{}'",
                out("before").display()
            ),
            after_start: format!("touch '{}'", out("after_start").display()),
            after_exit: format!("touch '{}'", out("after_exit").display()),
            use_global: true,
        };
        st.set_edits(id, e);
        st.config.scripts.before_start = format!("touch '{}'", out("global_before").display());
        st.config.scripts.after_exit = "exit 5".into();
        st.launch(id);
        // The before-start scripts run off the UI thread; the game counts as launching.
        assert_eq!(st.launcher.running(), Some(id));
        assert!(!out("env").exists());
        wait_exit(&mut st);
        assert!(st.poll_until(Duration::from_secs(5), |_| out("after_exit").exists()));
        let before = std::fs::read_to_string(out("before")).unwrap();
        assert_eq!(before.trim(), format!("Scripted|{id}|{0}|{0}", folder.display()));
        assert!(out("global_before").exists() && out("after_start").exists());
        // HDR is a game option now: a native program (`sh`) outside Gamescope gets no HDR
        // variables (spec §2.1; Proton / Wine launches are covered in game_options_tests).
        assert_eq!(std::fs::read_to_string(out("env")).unwrap().trim(), "");
        // The global after-exit script failed: reported.
        assert!(
            st.poll_until(Duration::from_secs(5), |s| s
                .toasts
                .iter()
                .any(|t| t.error && t.text.contains("Global after-exit script failed (exit code 5)"))),
            "{:?}",
            st.toasts
        );
        assert_eq!(st.db.get(id).unwrap().unwrap().play_count, 1);
    }

    #[test]
    fn failing_before_script_is_reported_and_the_game_still_starts() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let id = st.add_game(NewGame { name: "G".into(), ..Default::default() }).unwrap();
        let ran = dir.path().join("ran");
        let mut e = UserEdits::default();
        e.actions = vec![sh_action("Play", &format!("touch '{}'", ran.display()), true)];
        e.scripts.before_start = "echo nope >&2; exit 2".into();
        st.set_edits(id, e);
        st.launch(id);
        wait_exit(&mut st);
        assert!(ran.exists());
        assert!(
            st.toasts.iter().any(|t| t.error && t.text == "Before-start script failed (exit code 2): nope"),
            "{:?}",
            st.toasts
        );
    }

    #[test]
    fn stop_during_before_script_cancels_the_launch() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let id = st.add_game(NewGame { name: "G".into(), ..Default::default() }).unwrap();
        let ran = dir.path().join("ran");
        let mut e = UserEdits::default();
        e.actions = vec![sh_action("Play", &format!("touch '{}'", ran.display()), true)];
        e.scripts.before_start = "sleep 0.6".into();
        st.set_edits(id, e);
        st.launch(id);
        st.stop_running_game();
        wait_exit(&mut st);
        std::thread::sleep(Duration::from_millis(300));
        assert!(!ran.exists(), "the game started after Stop");
    }

    #[test]
    fn test_script_reports_its_result() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        st.test_script("echo hello from the script");
        assert!(st.poll_until(Duration::from_secs(5), |s| {
            s.toasts.iter().any(|t| !t.error && t.text == "Test script finished (exit code 0): hello from the script")
        }));
        st.test_script("exit 7");
        assert!(st.poll_until(Duration::from_secs(5), |s| {
            s.toasts.iter().any(|t| t.error && t.text.contains("exit code 7"))
        }));
    }

    #[test]
    fn install_size_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let folder = dir.path().join("g");
        std::fs::create_dir_all(folder.join("data")).unwrap();
        std::fs::write(folder.join("a.bin"), vec![0u8; 1000]).unwrap();
        std::fs::write(folder.join("data/b.bin"), vec![0u8; 24]).unwrap();
        let id = st.add_game(NewGame { name: "G".into(), install_dir: Some(folder), ..Default::default() }).unwrap();
        st.calc_install_size(id);
        assert!(st.size_calculating(id));
        assert!(st.poll_until(Duration::from_secs(5), |s| !s.size_calculating(id)));
        let g = st.library.get(id).unwrap();
        assert_eq!((g.edits.install_size, g.size_bytes), (Some(1024), Some(1024)));
        assert!(st.toasts.iter().any(|t| t.text == "G: 1.0 KB"), "{:?}", st.toasts);
        // No folder: says so.
        let none = st.add_game(NewGame { name: "H".into(), ..Default::default() }).unwrap();
        st.calc_install_size(none);
        assert!(!st.size_calculating(none));
        assert!(st.toasts.iter().any(|t| t.error && t.text.contains("no install folder")));
    }

    #[test]
    fn dir_size_skips_symlinks_and_counts_hardlinks_once() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a"), vec![1u8; 100]).unwrap();
        std::fs::write(d.join("sub/b"), vec![1u8; 10]).unwrap();
        std::fs::hard_link(d.join("a"), d.join("sub/a-again")).unwrap();
        std::os::unix::fs::symlink(d, d.join("sub/loop")).unwrap();
        std::os::unix::fs::symlink("/usr", d.join("usr")).unwrap();
        let s = dir_size(d, SIZE_ENTRY_CAP).unwrap();
        assert_eq!((s.bytes, s.files, s.truncated), (110, 2, false));
        let capped = dir_size(d, 2).unwrap();
        assert!(capped.truncated);
        assert!(dir_size(&d.join("a"), 10).is_err());
    }

    #[test]
    fn desktop_shortcut_file() {
        let dir = tempfile::tempdir().unwrap();
        let desk = dir.path().join("Desk top");
        let mut st = AppState::for_test(dir.path());
        let id = st.add_game(NewGame { name: "Half-Life: 100% \"Source\"/Ep 1".into(), ..Default::default() }).unwrap();
        let g = st.library.get(id).unwrap().clone();
        let exe = Path::new("/opt/My Apps/lodestar");
        let p = write_desktop_shortcut(&desk, &g, exe).unwrap();
        assert_eq!(p.file_name().unwrap().to_str().unwrap(), "Half-Life: 100% \"Source\" Ep 1.desktop");
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.starts_with("[Desktop Entry]\nType=Application\n"), "{text}");
        assert!(text.contains(&format!("\nExec=\"/opt/My Apps/lodestar\" --play {id}\n")), "{text}");
        assert!(text.contains("\nName=Half-Life: 100% \"Source\"/Ep 1\n"), "{text}");
        assert!(text.contains("\nIcon=lodestar\n"));
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o755);
        // Again: replaces our own file; someone else's file of that name is kept.
        assert_eq!(write_desktop_shortcut(&desk, &g, exe).unwrap(), p);
        std::fs::write(&p, "[Desktop Entry]\nName=Other\n").unwrap();
        let p2 = write_desktop_shortcut(&desk, &g, exe).unwrap();
        assert_ne!(p2, p);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[Desktop Entry]\nName=Other\n");
        assert_eq!(std::fs::read_dir(&desk).unwrap().count(), 2, "no temp files left");
    }

    #[test]
    fn exec_quoting_and_file_names() {
        assert_eq!(exec_quote("/usr/bin/lodestar"), "/usr/bin/lodestar");
        assert_eq!(exec_quote("/a b/$x\\\"y`"), "\"/a b/\\$x\\\\\\\"y\\`\"");
        assert_eq!(exec_quote(""), "\"\"");
        assert_eq!(shortcut_file_name("..//", 5), "lodestar-game-5.desktop");
        assert_eq!(shortcut_file_name(" A\n B ", 5), "A B.desktop");
    }

    #[test]
    fn user_dirs_desktop() {
        let home = Path::new("/home/u");
        let f = "# comment\nXDG_DOWNLOAD_DIR=\"$HOME/Downloads\"\nXDG_DESKTOP_DIR=\"$HOME/Schreibtisch\"\n";
        assert_eq!(parse_user_dirs_desktop(f, home), Some(PathBuf::from("/home/u/Schreibtisch")));
        assert_eq!(parse_user_dirs_desktop("XDG_DESKTOP_DIR=\"/data/desk\"", home), Some(PathBuf::from("/data/desk")));
        assert_eq!(parse_user_dirs_desktop("XDG_DESKTOP_DIR=\"$HOME/\"", home), None);
        assert_eq!(parse_user_dirs_desktop("", home), None);
    }

    #[test]
    fn only_web_and_file_links_open() {
        assert_eq!(checked_url(" https://www.protondb.com/app/1 "), Ok("https://www.protondb.com/app/1"));
        assert!(checked_url("HTTP://x").is_ok());
        assert!(checked_url("file:///home/u/manual.pdf").is_ok());
        assert!(checked_url("javascript:alert(1)").is_err());
        assert!(checked_url("steam://rungameid/570").is_err());
        assert!(checked_url("").is_err());
        assert!(checked_url("https://x\n--flag").is_err());
    }
}
