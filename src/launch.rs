//! Starting games and tracking their sessions.
//!
//! * [`resolve`] is pure: it applies `Game::launch_override` (see `LaunchOverride`) and the game
//!   options over `Game::launch`, and returns what would run: both the editor's "Will run: …" line
//!   and what [`Launcher::launch`] spawns. Steam games without a custom command take only plain
//!   extra arguments (`steam -applaunch <appid> args`); environment variables or wrappers there are
//!   an `Err` explaining that a custom command is needed. [`preview`] resolves an unsaved override
//!   for the editor, [`command_template`] gives the source's command as editable text (to prefill
//!   the custom-command field), and `Resolved::notes` carries non-fatal hints for the UI ("Steam
//!   also applies its own launch options").
//! * [`Launcher`] owns the sessions: `launch` returns at once (it never blocks the UI), `poll`
//!   hands over [`LaunchEvent`]s without blocking, `running` and `stop` ask about or end the
//!   current game (best effort). Its background threads call `ctx.request_repaint()` when they
//!   post an event.
//!
//! How launch options are applied (all in Lodestar, never written back to the source):
//!   * `command` (custom command) replaces the source's command entirely, so whatever the source
//!     launcher would add is bypassed. It may start with `VAR=value` words. `options` then
//!     wrap/extend that command.
//!     A leading `env VAR=value… program` (as in the Faugus/Steam direct-launch lines) is folded
//!     into the environment too, so the options' `VAR=value` override those variables.
//!   * `options` is Steam-style: `VAR=1 OTHER=2 wrapper --flag %command% -game-arg`.
//!     Leading `VAR=value` words become environment variables, words before `%command%` are a
//!     wrapper prefix, words after it are appended. Without `%command%` every non-env word is
//!     appended; `%command%` may appear only once. `$VAR` / `${VAR}` expand from Lodestar's
//!     environment, `~/` to the home dir (also in the working directory).
//!   * Launched processes get their own session and inherit no file descriptors but stdio.
//!   * Steam games are started by the Steam client, so only appended arguments can be passed
//!     (`steam -applaunch <appid> args…`); Steam still applies its own launch options too.
//!   * Games started through another launcher (`faugus-launcher --game x`, lutris …) inherit
//!     env vars and wrappers, but appended arguments reach that launcher, not the game (a note
//!     says so). URL launches (xdg-open) can't take arguments at all (Err).

use crate::game_options::compose::{self, ComposeInput, LaunchKind};
use crate::game_options::{Effective, LaunchEnv, OptionId, OptionState};
use crate::model::*;
use anyhow::{Context as _, anyhow, bail};
use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

pub(crate) mod track;

pub mod scripts;

pub mod play;

pub mod hygiene;

pub mod steam;

pub mod winhide;

pub mod winhide_wayland;

pub mod switch;

// Parts of this module, in sibling files: command-line words, what a spec is underneath, the
// game's own actions, adopting games that already run, spawning, and the Launcher. Their items
// are re-exported here with their own visibility (pub, pub(crate), or pub(super): private to
// launch).
mod action;
mod adopt;
mod base;
mod cmdline;
mod launcher;
mod spawn;

pub use action::*;
pub use adopt::*;
use base::*;
pub use cmdline::*;
pub use launcher::*;
use spawn::*;

use scripts::{ScriptNotice, ScriptRunner, SessionScripts};
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Debug, PartialEq)]
pub enum LaunchEvent {
    /// Process (or Steam's game process) detected running.
    Started {
        id: GameId,
    },
    /// Session ended; `secs` is how long it ran (0 if unknown, e.g. Steam handled it).
    Exited {
        id: GameId,
        secs: u64,
    },
    Failed {
        id: GameId,
        error: String,
    },
}

// ---------------------------------------------------------------------------------------------
// Resolving
// ---------------------------------------------------------------------------------------------

/// How a resolved launch is tracked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolvedKind {
    /// A process we spawn; the game runs while anything in its session/process group lives.
    Process,
    /// Started through the Steam client; tracked via Steam's reaper (`AppId=<appid>`).
    Steam { appid: u32 },
    /// A non-Steam shortcut inside Steam; `appid` is the 32-bit shortcut appid.
    SteamShortcut { appid: u32 },
    /// Handed to the desktop (xdg-open); not tracked.
    OpenUrl,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub program: String,
    pub args: Vec<String>,
    /// Extra environment for the game (on top of Lodestar's environment, minus internal vars).
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
    pub kind: ResolvedKind,
    /// Shell-quoted preview, e.g. `MANGOHUD=1 gamemoderun /opt/game/run.sh -fullscreen`.
    pub display: String,
    /// Non-fatal hints for the UI ("Steam also applies its own launch options: …").
    pub notes: Vec<String>,
    /// What happened to each game option (spec §2.8; layers carry their `layer.name`).
    pub options: Vec<(OptionId, Option<String>, OptionState)>,
    /// For Steam games: what to paste into the game's launch options in Steam (spec §3.4).
    pub steam_string: Option<String>,
    /// Folders to create before spawning (the Proton log folder).
    pub make_dirs: Vec<PathBuf>,
}

pub const STEAM_WRAPPER_ERR: &str = "Steam games can't take wrappers or environment variables from here — use Steam's own launch options, or set a custom command";
pub const URL_ARGS_ERR: &str =
    "This game is opened through a URL, which can't take extra arguments — set a custom command instead";
pub const SHORTCUT_ARGS_ERR: &str =
    "Non-Steam shortcuts in Steam can't take launch options from here — set them in Steam, or set a custom command";

/// How to reach the Steam client on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SteamCmd {
    /// `steam` on PATH.
    Native,
    /// `flatpak run com.valvesoftware.Steam`.
    Flatpak,
    /// Neither found: hand `steam://` URLs to xdg-open.
    XdgOpen,
}

/// Facts about the machine that resolving depends on. Detected once (no fs work per call).
#[derive(Clone, Debug)]
pub struct Host {
    pub steam: SteamCmd,
    pub home: Option<PathBuf>,
    /// Snapshot of the environment for `$VAR` expansion.
    pub vars: HashMap<String, String>,
}

impl Host {
    pub fn detect() -> Host {
        let home = dirs::home_dir();
        let steam = if find_in_path("steam").is_some() {
            SteamCmd::Native
        } else {
            let mut flatpak_dirs = vec![PathBuf::from("/var/lib/flatpak/app/com.valvesoftware.Steam")];
            if let Some(h) = &home {
                flatpak_dirs.push(h.join(".local/share/flatpak/app/com.valvesoftware.Steam"));
            }
            if flatpak_dirs.iter().any(|d| d.is_dir()) && find_in_path("flatpak").is_some() {
                SteamCmd::Flatpak
            } else {
                SteamCmd::XdgOpen
            }
        };
        Host { steam, home, vars: std::env::vars().collect() }
    }
}

fn host() -> &'static Host {
    static HOST: OnceLock<Host> = OnceLock::new();
    HOST.get_or_init(Host::detect)
}

/// Resolve what launching `game` would run (its Play action when the user set one, else the
/// source's launch, with the launch override and the game options applied). Cheap (string work
/// only), safe to call every frame.
pub fn resolve(game: &Game) -> anyhow::Result<Resolved> {
    resolve_full(game, host(), &crate::game_options::launch_env())
}

/// Resolve `game` as if its launch override were `ov` (for previewing unsaved edits).
pub fn preview(game: &Game, ov: &LaunchOverride) -> anyhow::Result<Resolved> {
    resolve_game(game, Some(ov), game.play_action(), host(), &crate::game_options::launch_env())
}

/// `resolve` with an explicit host and no global game options (`LaunchEnv::none()`: only the
/// game's own options apply, with nothing detected except the cached Proton dir lookup).
#[cfg(test)]
pub fn resolve_with(game: &Game, host: &Host) -> anyhow::Result<Resolved> {
    resolve_full(game, host, &LaunchEnv::none())
}

/// `resolve` with an explicit host and launch environment (defaults, caps, monitor).
pub fn resolve_full(game: &Game, host: &Host, env: &LaunchEnv) -> anyhow::Result<Resolved> {
    resolve_game(game, game.launch_override.as_ref(), game.play_action(), host, env)
}

/// Resolve the game's action `index` (`edits.actions[index]`).
pub fn resolve_action(game: &Game, index: usize) -> anyhow::Result<Resolved> {
    resolve_action_full(game, index, host(), &crate::game_options::launch_env())
}

#[cfg(test)]
pub fn resolve_action_with(game: &Game, index: usize, host: &Host) -> anyhow::Result<Resolved> {
    resolve_action_full(game, index, host, &LaunchEnv::none())
}

/// `resolve_action` with an explicit host and launch environment.
pub fn resolve_action_full(game: &Game, index: usize, host: &Host, env: &LaunchEnv) -> anyhow::Result<Resolved> {
    let a = game.edits.actions.get(index).ok_or_else(|| anyhow!("this game has no action {}", index + 1))?;
    resolve_game(game, game.launch_override.as_ref(), Some(a), host, env)
}

/// Resolve an unsaved action (the editor's "Will run:" line).
pub fn preview_action(game: &Game, action: &GameAction) -> anyhow::Result<Resolved> {
    resolve_game(game, game.launch_override.as_ref(), Some(action), host(), &crate::game_options::launch_env())
}

/// The game options for one resolve: merged values, the environment they apply in, and the
/// launch they apply to (spec §5 Plumbing).
pub(crate) struct Toggles<'a> {
    pub opts: &'a Effective,
    pub env: &'a LaunchEnv,
    /// The Play launch (wrapper toggles apply) or another action (env toggles only).
    pub play: bool,
    pub name: &'a str,
    pub game_id: GameId,
}

fn resolve_game(
    game: &Game,
    ov: Option<&LaunchOverride>,
    action: Option<&GameAction>,
    host: &Host,
    env: &LaunchEnv,
) -> anyhow::Result<Resolved> {
    let defaults = crate::game_options::defaults_for(env, game.source.key());
    let opts = crate::game_options::effective(&defaults, &game.edits.game_options);
    let t =
        Toggles { opts: &opts, env, play: action.is_none_or(|a| a.play), name: game.display_name(), game_id: game.id };
    // Emulated games: the command is rebuilt from the published emulation registry (the
    // emulator chosen now, per game / console), else the one the scan stored. Cloud games:
    // from the service app / browser installed now (`cloud::launch_for`).
    let emu_spec = emu_launch(game).or_else(|| crate::cloud::launch_for(game));
    // A game whose files no emulator can open (a Wii U folder without title.tmd …), or whose
    // chosen emulator isn't allowed to run yet: never fall back to an older stored command (that
    // would quietly use another emulator), say why instead.
    let blocked = action.is_none() && emu_spec.is_none() && crate::emulation::registry::no_fallback(game);
    let spec = if blocked { None } else { emu_spec.as_ref().or(game.launch.as_ref()) };
    match action {
        None => {
            let custom = ov.and_then(|o| o.command.as_deref()).is_some_and(|c| !c.trim().is_empty());
            if spec.is_none()
                && !custom
                && game.emu.is_some()
                && let Some(why) = crate::emulation::registry::missing_reason(game)
            {
                bail!(why);
            }
            resolve_parts(spec, ov, game.source_launch_options.as_deref(), host, Some(&t))
        }
        Some(a) => resolve_user_action(game, a, ov, host, &t),
    }
}

/// An emulated game's launch rebuilt from the published emulation registry (None: not an
/// emulated game, or nothing published — use `game.launch`). Cheap: no filesystem access.
fn emu_launch(game: &Game) -> Option<LaunchSpec> {
    game.emu.as_ref().and_then(|e| crate::emulation::registry::spec_for(game, e))
}

/// This Lodestar binary's path (what a `@lodestar` program resolves to), looked up once.
fn self_exe() -> Option<&'static str> {
    static EXE: OnceLock<Option<String>> = OnceLock::new();
    EXE.get_or_init(|| std::env::current_exe().ok().map(|p| p.display().to_string())).as_deref()
}

/// The parts of a launch before the user's options are folded in, for [`compose_launch`].
struct Parts {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    custom: bool,
    kind: ResolvedKind,
}

/// Fold the user's launch options and the game options into a command (spec §3).
fn compose_launch(p: Parts, opts: Options, cwd: Option<PathBuf>, notes: Vec<String>, t: &Toggles) -> Resolved {
    // The URL of a URL launch (after `run com.valvesoftware.Steam` for Flatpak Steam).
    let url = (p.kind == ResolvedKind::OpenUrl).then(|| p.args.iter().find(|a| a.contains("://")).cloned()).flatten();
    let steam_url = url.as_deref().is_some_and(|u| u.to_ascii_lowercase().starts_with("steam://"));
    let lk = match p.kind {
        ResolvedKind::Process => LaunchKind::Process,
        ResolvedKind::Steam { .. } | ResolvedKind::SteamShortcut { .. } => LaunchKind::Steam,
        // `steam://…` URLs are started by the Steam client: its note and launch-options string.
        ResolvedKind::OpenUrl if steam_url => LaunchKind::Steam,
        ResolvedKind::OpenUrl => LaunchKind::Url,
    };
    let out = compose::apply(ComposeInput {
        program: p.program,
        args: p.args,
        base_env: p.env,
        custom_command: p.custom,
        user_env: opts.env,
        user_prefix: opts.prefix,
        user_suffix: opts.suffix,
        kind: lk,
        play: t.play,
        name: t.name,
        game_id: t.game_id,
        url: url.as_deref(),
        self_exe: self_exe(),
        probe_runner: Some(crate::game_options::caps::runner_caps),
        probe_wine: Some(probe_wine),
        opts: t.opts,
        env: t.env,
    });
    let mut notes = notes;
    notes.extend(out.notes);
    let mut r = finish(out.program, out.args, out.env, cwd, p.kind, notes);
    r.options = out.report;
    r.steam_string = out.steam_string;
    r.make_dirs = out.make_dirs;
    r
}

/// What the plain Wine a launch runs supports (`wine`, `/usr/bin/wine`, a Lutris runner's wine),
/// for compose's `probe_wine`: a bare name is looked up in the command's PATH, then
/// `caps::wine_caps` checks its files. Resolving runs every frame, so the answer is kept for a
/// few seconds per (wine, PATH); a wine that isn't found is an unchecked (Unknown) wine.
fn probe_wine(wine: &Path, env: &[(String, String)]) -> std::sync::Arc<crate::game_options::RunnerCaps> {
    use crate::game_options::RunnerCaps;
    use std::sync::{Arc, Mutex};
    use std::time::Instant;
    type Cache = Mutex<HashMap<(PathBuf, Option<String>), (Instant, Arc<RunnerCaps>)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    const KEEP: Duration = Duration::from_secs(5);
    let path_var = env.iter().rev().find(|(k, _)| k == "PATH").map(|(_, v)| v.clone());
    let key = (wine.to_path_buf(), path_var);
    let cache = CACHE.get_or_init(Default::default);
    if let Ok(c) = cache.lock()
        && let Some((at, rc)) = c.get(&key)
        && at.elapsed() < KEEP
    {
        return rc.clone();
    }
    let rc = match find_in_command_path(&wine.to_string_lossy(), env) {
        Some(p) => crate::game_options::caps::wine_caps(&p),
        None => Arc::new(RunnerCaps::unknown()),
    };
    if let Ok(mut c) = cache.lock() {
        c.insert(key, (Instant::now(), rc.clone()));
    }
    rc
}

/// The game options' notes / report for a launch that is already built (Steam, Steam URLs):
/// nothing is added to its command.
fn with_option_notes(r: Resolved, t: Option<&Toggles>) -> Resolved {
    let Some(t) = t else { return r };
    let parts = Parts { program: r.program, args: r.args, env: r.env, custom: false, kind: r.kind };
    compose_launch(parts, Options::default(), r.cwd, r.notes, t)
}

/// Resolve a launch spec with the launch override and (`toggles`) the game options. The game
/// options never make this fail: the `bail!`s are about the user's typed options only.
fn resolve_parts(
    spec: Option<&LaunchSpec>,
    ov: Option<&LaunchOverride>,
    source_opts: Option<&str>,
    host: &Host,
    toggles: Option<&Toggles>,
) -> anyhow::Result<Resolved> {
    let empty = LaunchOverride::default();
    let ov = ov.unwrap_or(&empty);
    let opts = parse_options(&ov.options, host).context("launch options")?;
    let custom = ov.command.as_deref().filter(|c| !c.trim().is_empty());
    let mut notes = vec![];

    // 1. The command to run before options are applied.
    let (program, args, mut env, cwd, kind) = if let Some(c) = custom {
        // Leading `VAR=value` words are environment, like in a shell.
        let words = split_words(c, host, true).context("custom command")?;
        let mut cenv = vec![];
        let mut it = words.into_iter().peekable();
        while let Some(w) = it.peek() {
            match (!w.quoted_eq).then(|| is_env_word(&w.text)).flatten() {
                Some((k, v)) => {
                    set_env(&mut cenv, k, v);
                    it.next();
                }
                None => break,
            }
        }
        let mut words: Vec<String> = it.map(|w| w.text).collect();
        if words.is_empty() {
            bail!("custom command has no program");
        }
        let program = words.remove(0);
        let (program, words) = fold_env_command(program, words, &mut cenv);
        let kind = steam_kind_of(&program, &words).unwrap_or(ResolvedKind::Process);
        (program, words, cenv, ov_cwd(ov, host), kind)
    } else {
        let spec = spec.ok_or_else(|| anyhow!("no launch command known for this game — set a custom command"))?;
        match base_of(spec) {
            Base::Cmd { program, args, mut env, cwd } => {
                if program.trim().is_empty() {
                    bail!("launch command is empty — set a custom command");
                }
                let (program, args) = fold_env_command(program, args, &mut env);
                let kind = steam_kind_of(&program, &args).unwrap_or(ResolvedKind::Process);
                (program, args, env, ov_cwd(ov, host).or(cwd), kind)
            }
            Base::Url(url) => (str_s("xdg-open"), vec![url], vec![], ov_cwd(ov, host), ResolvedKind::OpenUrl),
            Base::SteamUrl(url) => {
                if !opts.env.is_empty() || !opts.prefix.is_empty() || !opts.suffix.is_empty() {
                    bail!(STEAM_WRAPPER_ERR);
                }
                let (program, args) = steam_invocation(host, vec![url]);
                let r = finish(program, args, vec![], None, ResolvedKind::OpenUrl, notes);
                return Ok(with_option_notes(r, toggles));
            }
            Base::Steam { appid } => {
                if !opts.env.is_empty() || !opts.prefix.is_empty() {
                    bail!(STEAM_WRAPPER_ERR);
                }
                if ov.cwd.is_some() {
                    notes.push("The working directory is ignored for Steam games (Steam sets it).".into());
                }
                let extra = opts.suffix;
                let (program, args) = if extra.is_empty() {
                    steam_invocation(host, vec![format!("steam://rungameid/{appid}")])
                } else {
                    if let Some(s) = source_opts.map(str::trim).filter(|s| !s.is_empty()) {
                        notes.push(format!(
                            "Steam also applies its own launch options ({s}); these arguments are added after them."
                        ));
                    }
                    if host.steam == SteamCmd::XdgOpen {
                        let joined = extra.join(" ");
                        (str_s("xdg-open"), vec![format!("steam://run/{appid}//{}/", url_encode(&joined))])
                    } else {
                        let mut a = vec![str_s("-applaunch"), appid.to_string()];
                        a.extend(extra);
                        steam_invocation(host, a)
                    }
                };
                let r = finish(program, args, vec![], None, ResolvedKind::Steam { appid }, notes);
                return Ok(with_option_notes(r, toggles));
            }
            Base::Shortcut { gameid, appid } => {
                if !opts.env.is_empty() || !opts.prefix.is_empty() || !opts.suffix.is_empty() {
                    bail!(SHORTCUT_ARGS_ERR);
                }
                let (program, args) = steam_invocation(host, vec![format!("steam://rungameid/{gameid}")]);
                let r = finish(program, args, vec![], None, ResolvedKind::SteamShortcut { appid }, notes);
                return Ok(with_option_notes(r, toggles));
            }
        }
    };

    // 2. Apply options around it.
    let via_steam = matches!(kind, ResolvedKind::Steam { .. } | ResolvedKind::SteamShortcut { .. });
    let wraps = !opts.env.is_empty() || !opts.prefix.is_empty();
    // `steam -applaunch …` only hands the request to the running client: environment and
    // wrappers would apply to that hand-off (or, when Steam isn't running yet, to the whole
    // Steam client), never to the game.
    if via_steam && wraps && custom.is_none() {
        bail!(STEAM_WRAPPER_ERR);
    }
    if via_steam && custom.is_some() && (wraps || !env.is_empty()) {
        notes.push(
            "This command starts the game through Steam: environment variables and wrappers reach the Steam client, not the game."
                .into(),
        );
    }
    if !opts.suffix.is_empty() && custom.is_none() {
        if kind == ResolvedKind::OpenUrl {
            bail!(URL_ARGS_ERR);
        }
        if let Some(l) = launcher_wrapper(&program, &args) {
            notes.push(format!(
                "Extra arguments go to {l}, not to the game. To pass arguments to the game itself (or replace {l}'s own launch options), set a custom command."
            ));
        }
    }
    if let Some(t) = toggles {
        if kind == ResolvedKind::OpenUrl && !(env.is_empty() && opts.env.is_empty()) {
            notes.push("Environment variables only reach the URL handler, which may already be running.".into());
        }
        let parts = Parts { program, args, env, custom: custom.is_some(), kind };
        return Ok(compose_launch(parts, opts, cwd, notes, t));
    }
    for (k, v) in opts.env {
        set_env(&mut env, k, v);
    }
    let (program, args) = if opts.prefix.is_empty() {
        let mut a = args;
        a.extend(opts.suffix);
        (program, a)
    } else {
        let mut words = opts.prefix;
        let wrapper = words.remove(0);
        words.push(program);
        words.extend(args);
        words.extend(opts.suffix);
        (wrapper, words)
    };
    if kind == ResolvedKind::OpenUrl && !env.is_empty() {
        notes.push("Environment variables only reach the URL handler, which may already be running.".into());
    }
    Ok(finish(program, args, env, cwd, kind, notes))
}

/// Programs that are launchers rather than games: arguments appended to them don't reach the game.
/// `flatpak run <app> …` passes them on to the app, so only launcher apps (Lutris, Heroic,
/// Bottles … as Flatpaks) count; emulators and Flatpak games get the arguments.
fn launcher_wrapper(program: &str, args: &[String]) -> Option<&'static str> {
    let base = Path::new(program).file_name().and_then(|s| s.to_str()).unwrap_or(program);
    Some(match base {
        "faugus-launcher" => "Faugus",
        "lutris" => "Lutris",
        "heroic" => "Heroic",
        "bottles-cli" => "Bottles",
        "legendary" => "Legendary",
        "itch" => "itch",
        "flatpak" => {
            let mut w = args.iter().skip_while(|a| a.starts_with('-'));
            if w.next().map(String::as_str) != Some("run") {
                return None;
            }
            let opts: Vec<&String> = w.clone().take_while(|a| a.starts_with('-')).collect();
            let app = w.find(|a| !a.starts_with('-')).map(String::as_str).unwrap_or("");
            let bottles = opts.iter().any(|o| o.strip_prefix("--command=").is_some_and(|c| c.ends_with("bottles-cli")));
            return crate::game_options::compose::flatpak_launcher(app).or(bottles.then_some("Bottles"));
        }
        _ => return None,
    })
}

/// `env [VAR=value…] program args…` (env(1) with assignments only) -> `program args…`, with the
/// assignments moved into `env`. That way `VAR=value` launch options override them (env(1) would
/// otherwise set them after Lodestar did, so e.g. a direct-launch line's `WINEPREFIX` would always
/// win), and wrapper/launcher detection sees the real program. Anything else is left alone
/// (`env -i …`, `env -u X …`, `env` without a program).
fn fold_env_command(program: String, args: Vec<String>, env: &mut Vec<(String, String)>) -> (String, Vec<String>) {
    let base = Path::new(&program).file_name().and_then(|s| s.to_str()).unwrap_or(&program);
    if base != "env" {
        return (program, args);
    }
    let n = args.iter().take_while(|a| is_env_word(a).is_some()).count();
    match args.get(n) {
        Some(p) if !p.starts_with('-') && !p.is_empty() => {}
        _ => return (program, args),
    }
    let mut it = args.into_iter();
    for a in it.by_ref().take(n) {
        if let Some((k, v)) = is_env_word(&a) {
            set_env(env, k, v);
        }
    }
    let program = it.next().unwrap_or_default();
    // `env A=1 env B=2 game` folds all the way down.
    fold_env_command(program, it.collect(), env)
}

/// The override's working directory, `~/…` expanded.
fn ov_cwd(ov: &LaunchOverride, host: &Host) -> Option<PathBuf> {
    let c = ov.cwd.as_ref()?;
    let s = c.to_str()?;
    match (&host.home, s.strip_prefix('~')) {
        (Some(h), Some(rest)) if rest.is_empty() || rest.starts_with('/') => Some(h.join(rest.trim_start_matches('/'))),
        _ => Some(c.clone()),
    }
}

/// The source's command as an editable command line, for prefilling the "custom command" field
/// so the user can start from it and change anything (None for Steam/URL launches, which have no
/// local command line). The source's environment comes first as `VAR=value` words, which the
/// custom command field accepts. Shell-quoted; cheap.
pub fn command_template(game: &Game) -> Option<String> {
    let emu_spec = emu_launch(game).or_else(|| crate::cloud::launch_for(game));
    let spec = emu_spec.as_ref().or(game.launch.as_ref())?;
    let LaunchSpec::Command { program, args, env, .. } = spec else {
        return None;
    };
    if let Base::Steam { .. } | Base::Shortcut { .. } = base_of(spec) {
        return None;
    }
    let mut words: Vec<String> = env.iter().map(|(k, v)| format!("{k}={}", shell_quote(v))).collect();
    words.push(quote_program(program));
    words.extend(args.iter().map(|a| shell_quote(a)));
    Some(words.join(" "))
}

fn finish(
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    cwd: Option<PathBuf>,
    kind: ResolvedKind,
    notes: Vec<String>,
) -> Resolved {
    let mut parts: Vec<String> = env.iter().map(|(k, v)| format!("{k}={}", shell_quote(v))).collect();
    parts.push(quote_program(&program));
    parts.extend(args.iter().map(|a| shell_quote(a)));
    Resolved {
        display: parts.join(" "),
        program,
        args,
        env,
        cwd,
        kind,
        notes,
        options: vec![],
        steam_string: None,
        make_dirs: vec![],
    }
}

fn str_s(s: &str) -> String {
    s.to_string()
}

fn set_env(env: &mut Vec<(String, String)>, k: String, v: String) {
    if let Some(e) = env.iter_mut().find(|(ek, _)| *ek == k) {
        e.1 = v;
    } else {
        env.push((k, v));
    }
}

// ---------------------------------------------------------------------------------------------
// Launching
// ---------------------------------------------------------------------------------------------

/// `program` is this Lodestar binary (a `@lodestar` helper spec, resolved in `base`).
fn is_self_program(program: &str) -> bool {
    std::env::current_exe().is_ok_and(|me| Path::new(program) == me)
}

/// Environment variables that belong to Lodestar and must not leak into games.
pub(crate) fn strip_env_var(k: &str) -> bool {
    k.starts_with("LODESTAR_")
        || matches!(
            k,
            "RUST_LOG" | "RUST_BACKTRACE" | "RUST_LIB_BACKTRACE" | "XDG_ACTIVATION_TOKEN" | "DESKTOP_STARTUP_ID"
        )
}

/// Close-on-exec every fd above stderr, so the game doesn't inherit Lodestar's descriptors.
/// Libraries open some without O_CLOEXEC — gilrs' evdev handles (O_RDWR on the controllers), its
/// epoll and eventfd — and a game (or a Steam client started by `steam -applaunch`) would keep
/// those open for as long as it runs. Marking instead of closing keeps std's own exec-error pipe
/// working (it is CLOEXEC already, so exec closes it as usual).
///
/// # Safety
/// Only call between fork and exec (async-signal-safe: plain syscalls, no allocation).
pub(crate) unsafe fn mark_inherited_fds_cloexec() {
    // SAFETY: plain syscalls on fd numbers.
    unsafe {
        let r = libc::syscall(libc::SYS_close_range, 3 as libc::c_uint, libc::c_uint::MAX, libc::CLOSE_RANGE_CLOEXEC);
        if r == 0 {
            return;
        }
        // Kernel < 5.11: walk the table.
        let mut lim = libc::rlimit { rlim_cur: 0, rlim_max: 0 };
        let max = if libc::getrlimit(libc::RLIMIT_NOFILE, &mut lim) == 0 {
            lim.rlim_cur.min(65536) as libc::c_int
        } else {
            4096
        };
        for fd in 3..max {
            let fl = libc::fcntl(fd, libc::F_GETFD);
            if fl >= 0 && fl & libc::FD_CLOEXEC == 0 {
                libc::fcntl(fd, libc::F_SETFD, fl | libc::FD_CLOEXEC);
            }
        }
    }
}

/// Starts games and follows their sessions (its methods: launch/launcher.rs).
pub struct Launcher {
    ctx: egui::Context,
    log_dir: PathBuf,
    monitor: Option<Sender<track::Msg>>,
    ev_tx: Sender<LaunchEvent>,
    ev_rx: Receiver<LaunchEvent>,
    pending: Vec<LaunchEvent>,
    /// Launching or running, oldest first.
    active: Vec<GameId>,
    /// Seen running (Started emitted), subset of `active`.
    started: Vec<GameId>,
    logs: HashMap<GameId, PathBuf>,
    /// How each game was last started this session (decides who records its playtime).
    kinds: HashMap<GameId, ResolvedKind>,
    poll_every: Duration,
    steam_timeout: Duration,
    /// Runs game scripts in the background; results arrive on `notices`.
    scripts: ScriptRunner,
    notices: Receiver<ScriptNotice>,
    /// After-start / after-exit scripts of the current session per game.
    session_scripts: HashMap<GameId, SessionScripts>,
    /// Games whose after-start scripts ran this session (after-exit runs only for those).
    script_started: std::collections::HashSet<GameId>,
    /// Launches waiting for their before-start scripts (PRE_*).
    prelaunch: HashMap<GameId, Arc<AtomicU8>>,
    /// Launcher hygiene (minimise / close store clients); off until the app turns it on.
    hygiene_cfg: hygiene::HygieneConfig,
    /// The hygiene thread, started with the first launch that needs it.
    hygiene: Option<Sender<hygiene::Msg>>,
    /// Launches registered with the hygiene thread (their Started / Exited go there too).
    hygiene_ids: std::collections::HashSet<GameId>,
    /// What each game was last started through (for "Starting X via Y…").
    via: HashMap<GameId, Via>,
    /// Adopted games another running Lodestar follows (it records their sessions).
    foreign: std::collections::HashSet<GameId>,
    /// Launch tags of adopted games that are followed now…
    adopted_tags: HashMap<GameId, String>,
    /// …and of those whose session ended (stray processes of theirs aren't adopted again).
    ended_tags: std::collections::HashSet<String>,
    /// When adopted games really started (unix seconds).
    adopted_at: HashMap<GameId, i64>,
    /// A scan for running games in progress (`start_running_scan`).
    running_scan: Option<Receiver<RunningScan>>,
    /// When the last scan for running games finished, and whether it saw something that looks
    /// like a game this run may not follow (`track::other_game`): the background
    /// controller's safety check.
    others_look: Option<(std::time::Instant, bool)>,
}

/// What a launch went through, for status lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Via {
    /// A Windows store client in a Wine prefix (Battle.net, EA app, Ubisoft Connect …).
    StoreClient,
    /// A `heroic://` link handled by Heroic.
    Heroic,
    /// The Steam client.
    Steam,
    /// Lodestar started the game itself (or handed a URL to the desktop).
    Direct,
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod adopt_tests;
