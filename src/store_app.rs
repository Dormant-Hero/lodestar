//! Store actions for library games: Install / Uninstall / Store page, handed to the owning client.
//! Lodestar never installs or deletes game files itself; it opens the store's own client (or its
//! CLI in a terminal, or asks the add-on) and a later scan picks up the change.
//!
//! CONTRACT (desktop + controller UIs):
//!   st.can_install(id) -> bool        not installed and its store / add-on can install it
//!   st.install_game(id)               Steam: steam://install/<id>; Epic/GOG/Amazon: Heroic (or the CLI);
//!                                     Battle.net/EA/Ubisoft…: open the launcher in its prefix; add-on: `install`
//!   st.can_uninstall(id) -> bool
//!   st.uninstall_game(id)             same, never deletes files itself
//!   st.store_page(id) -> Option<String>   URL of the game's store page (for the Links chips / More menu)
//!   st.poll_store()                   call once per frame: delivers add-on install/uninstall results
//!                                     (toast + rescan). Cheap (one uncontended lock).
//! All of these respect AppState::no_launch (screenshot runs never open anything). Pure helpers:
//!   install_plan(&Game) / uninstall_plan(&Game) -> Option<Plan>   what the buttons would do
//!   store_page_for(&Game, &StoreUrls)  store-page lookup
//!   invalidate_store_urls()           scanners call it; the next store_page() re-reads the caches
//!
//! Routes per source:
//!   Steam            steam://install/<appid>, steam://uninstall/<appid> (through the Steam client)
//!   Epic/GOG/Amazon  from Heroic (launch is heroic://…): install = heroic://launch?appName=…&runner=…,
//!                    which makes Heroic ask "not installed, install it?"; uninstall = open Heroic.
//!                    From the CLIs (launch is `legendary launch …` / `nile launch …`): the CLI's own
//!                    install / uninstall in a terminal window (it asks before doing anything).
//!   Heroic sideload  uninstall = open Heroic
//!   itch             open the itch app's library
//!   Battle.net, EA, Ubisoft, Rockstar, Riot, HoYoPlay
//!                    open the launcher in the game's prefix (the game's launch command cut after the
//!                    launcher's .exe, same runner and environment)
//!   Add-on           the add-on's `install` / `uninstall` command, on a background thread
//!   Minecraft, Moonlight, Lutris, Bottles, Faugus, desktop, manual: none

use crate::app::AppState;
use crate::model::*;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Known store pages by game: `(source, source_id)` → URL.
pub type StoreUrls = HashMap<(Source, String), String>;

/// What Install / Uninstall does for one game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreAction {
    /// Open a URI with the desktop's handler (xdg-open).
    OpenUri(String),
    /// Start a program, detached and untracked (a store client, a launcher in its Wine prefix).
    Run { program: String, args: Vec<String>, env: Vec<(String, String)>, cwd: Option<PathBuf> },
    /// Run a CLI in a terminal window, so the user sees its questions and progress.
    Terminal(Vec<String>),
    /// Open the Heroic client (its binary, its Flatpak, or `heroic://ping`).
    OpenHeroic,
    /// Steam URI through the Steam client (`steam <uri>`, Flatpak, or xdg-open).
    Steam(String),
    /// Open the store client in the game's Wine prefix: `wine_launchers::launcher_command` (looked
    /// up off the UI thread), else `fallback` (the game's own command cut after the launcher .exe).
    PrefixLauncher { source: Source, source_id: String, fallback: Option<Box<StoreAction>> },
    /// Ask an external add-on (`install` / `uninstall` with `{"id"}`).
    Addon { addon: String, game: String, command: &'static str },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub action: StoreAction,
    /// Toast shown when it starts ("Steam will install Portal").
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verb {
    Install,
    Uninstall,
}

/// How an Epic / GOG / Amazon game reached the library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StoreClient {
    Heroic,
    Legendary,
    Nile,
}

fn store_client(g: &Game) -> Option<StoreClient> {
    match &g.launch {
        Some(LaunchSpec::Url(u)) if u.to_ascii_lowercase().starts_with("heroic://") => Some(StoreClient::Heroic),
        Some(LaunchSpec::Command { program, .. }) => match Path::new(program).file_name()?.to_str()? {
            "legendary" => Some(StoreClient::Legendary),
            "nile" => Some(StoreClient::Nile),
            "heroic" => Some(StoreClient::Heroic),
            _ => None,
        },
        // A game whose launch was never set by its scanner: Heroic is how these stores arrive.
        None => Some(StoreClient::Heroic),
        _ => None,
    }
}

/// What Install does for `g`, if anything (pure; no file system access).
pub fn install_plan(g: &Game) -> Option<Plan> {
    if g.installed {
        return None;
    }
    plan(g, Verb::Install)
}

/// What Uninstall does for `g`, if anything (pure; no file system access).
pub fn uninstall_plan(g: &Game) -> Option<Plan> {
    if !g.installed {
        return None;
    }
    plan(g, Verb::Uninstall)
}

fn plan(g: &Game, verb: Verb) -> Option<Plan> {
    let name = g.display_name();
    let (install, word) = match verb {
        Verb::Install => (true, "install"),
        Verb::Uninstall => (false, "uninstall"),
    };
    let p = |action, message: String| Some(Plan { action, message });
    match g.source {
        Source::Steam => {
            let appid = g.steam_appid()?;
            let uri = format!("steam://{word}/{appid}");
            let msg = if install {
                format!("Steam will install {name}")
            } else {
                format!("Steam will ask to uninstall {name}")
            };
            p(StoreAction::Steam(uri), msg)
        }
        Source::Epic | Source::Gog | Source::Amazon => match store_client(g)? {
            StoreClient::Heroic if install => {
                let runner = crate::scan::heroic::runner_for(g.source)?;
                p(
                    StoreAction::OpenUri(crate::scan::heroic::launch_url(&g.source_id, runner)),
                    format!("Heroic will ask to install {name}"),
                )
            }
            StoreClient::Heroic => {
                p(StoreAction::OpenHeroic, format!("Opening Heroic: uninstall {name} from its library"))
            }
            StoreClient::Legendary if g.source == Source::Epic => p(
                StoreAction::Terminal(vec!["legendary".into(), word.into(), g.source_id.clone()]),
                format!("Running legendary {word} for {name} in a terminal"),
            ),
            StoreClient::Nile if g.source == Source::Amazon => p(
                StoreAction::Terminal(vec!["nile".into(), word.into(), g.source_id.clone()]),
                format!("Running nile {word} for {name} in a terminal"),
            ),
            _ => None,
        },
        Source::Heroic if !install && store_client(g) == Some(StoreClient::Heroic) => {
            p(StoreAction::OpenHeroic, format!("Opening Heroic: remove {name} from its library"))
        }
        Source::Itch => p(StoreAction::OpenUri("itch://library".into()), format!("Opening itch: {word} {name} there")),
        Source::BattleNet | Source::Ea | Source::Ubisoft | Source::Rockstar | Source::Riot | Source::HoYoPlay => {
            // Only games that run in a prefix. A game that is gone can't be found by the prefix
            // scan any more: Install needs its own command to name the launcher.
            if !matches!(g.launch, Some(LaunchSpec::Command { .. })) {
                return None;
            }
            let fallback = prefix_launcher(g).map(Box::new);
            if install && fallback.is_none() {
                return None;
            }
            p(
                StoreAction::PrefixLauncher { source: g.source, source_id: g.source_id.clone(), fallback },
                format!("Opening {}: {word} {name} there", g.source.display()),
            )
        }
        Source::Addon => {
            let (addon, game) = crate::addons::split_source_id(&g.source_id)?;
            p(
                StoreAction::Addon {
                    addon: addon.into(),
                    game: game.into(),
                    command: if install { "install" } else { "uninstall" },
                },
                format!("Asking the {addon} add-on to {word} {name}…"),
            )
        }
        _ => None,
    }
}

/// Launcher executables (lowercase file names) of the prefix stores. `launcher.exe` is too generic
/// alone: it counts only inside a Rockstar / HoYoPlay folder.
fn is_launcher_exe(arg: &str) -> bool {
    let lower = arg.replace('\\', "/").to_ascii_lowercase();
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    match base {
        "battle.net.exe"
        | "battle.net launcher.exe"
        | "eadesktop.exe"
        | "ealauncher.exe"
        | "origin.exe"
        | "upc.exe"
        | "ubisoftconnect.exe"
        | "uplay.exe"
        | "riotclientservices.exe"
        | "hyp.exe"
        | "hoyoplay.exe" => true,
        "launcher.exe" => lower.contains("rockstar") || lower.contains("hoyoplay"),
        _ => false,
    }
}

/// The game's launch command cut right after the launcher's .exe: the same runner (umu-run / proton /
/// wine), prefix and environment, without the "start this game" arguments — opens the launcher.
fn prefix_launcher(g: &Game) -> Option<StoreAction> {
    let Some(LaunchSpec::Command { program, args, env, cwd }) = &g.launch else {
        return None;
    };
    if is_launcher_exe(program) {
        return Some(StoreAction::Run { program: program.clone(), args: vec![], env: env.clone(), cwd: cwd.clone() });
    }
    let i = args.iter().position(|a| is_launcher_exe(a))?;
    Some(StoreAction::Run { program: program.clone(), args: args[..=i].to_vec(), env: env.clone(), cwd: cwd.clone() })
}

/// The game's store page (pure). Epic / GOG / itch pages come from the store caches (`urls`).
/// GOG games without a known slug (Heroic 2.22 no longer caches gog.com slugs; its `game.slug` is
/// GOG Galaxy's database slug, not the store's) get a GOG store search for their name.
pub fn store_page_for(g: &Game, urls: &StoreUrls) -> Option<String> {
    match g.source {
        Source::Steam => g.steam_appid().map(|id| format!("https://store.steampowered.com/app/{id}")),
        Source::Gog => urls.get(&(g.source, g.source_id.clone())).cloned().or_else(|| {
            let q: String = g.name.chars().filter(|c| !matches!(c, '®' | '™' | '©')).collect();
            let q = q.trim();
            (!q.is_empty()).then(|| {
                format!("https://www.gog.com/en/games?query={}", crate::scan::desktop::common::percent_encode(q))
            })
        }),
        Source::Epic | Source::Itch => urls.get(&(g.source, g.source_id.clone())).cloned(),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// Store-page index: built off the UI thread from Heroic's / Legendary's caches and itch's DB.

#[derive(Default)]
struct UrlIndex {
    map: Option<Arc<StoreUrls>>,
    loading: bool,
    stale: bool,
}

static URLS: Mutex<UrlIndex> = Mutex::new(UrlIndex { map: None, loading: false, stale: false });

/// The store caches changed (a scan ran): the next `store_page` re-reads them in the background.
pub fn invalidate_store_urls() {
    URLS.lock().stale = true;
}

/// Every store page the local caches know.
pub fn collect_store_urls(ctx: &crate::scan::ScanContext) -> StoreUrls {
    let mut map: StoreUrls = crate::scan::heroic::store_urls(ctx).into_iter().collect();
    for db in
        [ctx.config_home().join("itch/db/butler.db"), ctx.flatpak_app("io.itch.itch").join("config/itch/db/butler.db")]
    {
        if db.is_file() {
            for (cave, url) in itch_urls(&db) {
                map.entry((Source::Itch, cave)).or_insert(url);
            }
        }
    }
    map
}

/// itch game pages by cave id (the itch scanner's `source_id`).
fn itch_urls(db: &Path) -> Vec<(String, String)> {
    let Ok(conn) = crate::scan::desktop::common::open_sqlite_ro(db) else {
        return vec![];
    };
    let Ok(mut st) =
        conn.prepare("SELECT c.id, g.url FROM caves c JOIN games g ON g.id = c.game_id WHERE g.url IS NOT NULL")
    else {
        return vec![];
    };
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)));
    match rows {
        Ok(rows) => rows.flatten().filter(|(_, u)| u.starts_with("https://") && u.contains(".itch.io/")).collect(),
        Err(_) => vec![],
    }
}

// ---------------------------------------------------------------------------------------------
// Add-on results

enum StoreMsg {
    AddonDone { name: String, verb: &'static str, result: Result<(), String> },
    Error(String),
}

static MSGS: Mutex<Vec<StoreMsg>> = Mutex::new(Vec::new());

// ---------------------------------------------------------------------------------------------
// AppState API

impl AppState {
    pub fn can_install(&self, id: GameId) -> bool {
        self.library.get(id).is_some_and(|g| {
            // RomM: Download into a ROM folder (romm_app.rs).
            if g.source == Source::Romm {
                return self.romm_can_download(g);
            }
            install_plan(g).is_some() && self.addon_ready(g)
        })
    }

    /// An add-on game's add-on is installed, valid and switched on (others: always true). Keeps
    /// Install / Uninstall off for games of a switched-off add-on instead of failing on click.
    fn addon_ready(&self, g: &crate::model::Game) -> bool {
        if g.source != Source::Addon {
            return true;
        }
        crate::addons::split_source_id(&g.source_id).is_some_and(|(addon, _)| {
            crate::addons::is_enabled(addon, &self.config.addons)
                && self.addons.get(addon).is_some_and(|a| a.is_usable())
        })
    }

    pub fn install_game(&mut self, id: GameId) {
        let Some(g) = self.library.get(id) else {
            return;
        };
        if g.source == Source::Romm {
            self.romm_download(id);
            return;
        }
        match install_plan(g) {
            Some(p) => self.run_store_plan(p),
            None => {
                let msg = format!("{} can't be installed from Lodestar", g.display_name());
                self.notify(msg);
            }
        }
    }

    pub fn can_uninstall(&self, id: GameId) -> bool {
        self.library.get(id).is_some_and(|g| uninstall_plan(g).is_some() && self.addon_ready(g))
    }

    pub fn uninstall_game(&mut self, id: GameId) {
        let Some(g) = self.library.get(id) else {
            return;
        };
        match uninstall_plan(g) {
            Some(p) => self.run_store_plan(p),
            None => {
                let msg = format!("{} can't be uninstalled from Lodestar", g.display_name());
                self.notify(msg);
            }
        }
    }

    /// Cheap (a hash lookup); safe to call every frame. Epic / GOG / itch pages appear once the
    /// background read of the store caches is done (it starts on the first call).
    pub fn store_page(&self, id: GameId) -> Option<String> {
        let g = self.library.get(id)?;
        if g.source == Source::Steam {
            return store_page_for(g, &StoreUrls::new());
        }
        if !matches!(g.source, Source::Epic | Source::Gog | Source::Itch) {
            return None;
        }
        // Before the caches are read: no Epic / itch page yet, GOG's store search.
        store_page_for(g, &self.store_page_index().unwrap_or_default())
    }

    /// The store-page index; starts the background read on first use / after a scan.
    fn store_page_index(&self) -> Option<Arc<StoreUrls>> {
        {
            let mut idx = URLS.lock();
            if (idx.map.is_none() || idx.stale) && !idx.loading {
                idx.loading = true;
                idx.stale = false;
                let mut sctx = crate::scan::ScanContext::from_config(&self.config);
                sctx.home = self.paths.home.clone();
                let ctx = self.ctx().clone();
                let spawned = std::thread::Builder::new().name("store-urls".into()).spawn(move || {
                    let map = collect_store_urls(&sctx);
                    let mut idx = URLS.lock();
                    idx.map = Some(Arc::new(map));
                    idx.loading = false;
                    drop(idx);
                    ctx.request_repaint();
                });
                if spawned.is_err() {
                    idx.loading = false;
                }
            }
            idx.map.clone()
        }
    }

    /// Deliver finished add-on store calls (toast; rescan after a success). Call once per frame.
    pub fn poll_store(&mut self) {
        let msgs = std::mem::take(&mut *MSGS.lock());
        let mut rescan = false;
        for m in msgs {
            match m {
                StoreMsg::AddonDone { name, verb, result: Ok(()) } => {
                    self.notify(format!("{name}: {verb} done"));
                    rescan = true;
                }
                StoreMsg::AddonDone { name, verb, result: Err(e) } => {
                    self.notify_error(format!("Couldn't {verb} {name}: {e}"));
                }
                StoreMsg::Error(e) => self.notify_error(e),
            }
        }
        if rescan {
            self.rescan();
        }
    }

    fn run_store_plan(&mut self, plan: Plan) {
        if self.no_launch {
            log::warn!("screenshot run: not running store action {:?}", plan.action);
            self.notify(format!("Skipped (screenshot run): {}", plan.message));
            return;
        }
        match plan.action {
            StoreAction::OpenUri(u) => spawn_detached("xdg-open".into(), vec![u], vec![], None),
            StoreAction::Run { program, args, env, cwd } => spawn_detached(program, args, env, cwd),
            StoreAction::PrefixLauncher { source, source_id, fallback } => {
                let mut sctx = crate::scan::ScanContext::from_config(&self.config);
                sctx.home = self.paths.home.clone();
                let ctx = self.ctx().clone();
                let spawned = std::thread::Builder::new().name("store-prefix".into()).spawn(move || {
                    let found = crate::scan::wine_launchers::launcher_command(&sctx, source, &source_id);
                    let run = match (found, fallback.map(|b| *b)) {
                        (Some(LaunchSpec::Command { program, args, cwd, env }), _)
                        | (None, Some(StoreAction::Run { program, args, env, cwd })) => Some((program, args, env, cwd)),
                        _ => None,
                    };
                    match run {
                        Some((program, args, env, cwd)) => spawn_detached(program, args, env, cwd),
                        None => {
                            MSGS.lock().push(StoreMsg::Error(format!(
                                "Couldn't find the {} client in the game's Wine prefix",
                                source.display()
                            )));
                            ctx.request_repaint();
                        }
                    }
                });
                if let Err(e) = spawned {
                    self.notify_error(format!("Couldn't open {}: {e}", source.display()));
                    return;
                }
            }
            StoreAction::Steam(uri) => {
                let (program, args) = steam_command(&uri);
                spawn_detached(program, args, vec![], None);
            }
            StoreAction::OpenHeroic => {
                let (program, args) = heroic_command();
                spawn_detached(program, args, vec![], None);
            }
            StoreAction::Terminal(argv) => match terminal_command(&argv) {
                Some((program, args)) => spawn_detached(program, args, vec![], None),
                None => {
                    self.notify_error(format!(
                        "No terminal program found; run this yourself: {}",
                        argv.iter().map(|a| crate::launch::shell_quote(a)).collect::<Vec<_>>().join(" ")
                    ));
                    return;
                }
            },
            StoreAction::Addon { addon, game, command } => {
                let name = self
                    .library
                    .games
                    .iter()
                    .find(|g| g.source == Source::Addon && g.source_id == format!("{addon}:{game}"));
                let name = name.map(|g| g.display_name().to_string()).unwrap_or_else(|| game.clone());
                let manager = self.addons.clone();
                let cfg = self.config.addons.clone();
                let ctx = self.ctx().clone();
                let spawned = std::thread::Builder::new().name("addon-store".into()).spawn(move || {
                    let result = manager.game_command(&addon, command, &game, &cfg).map_err(|e| format!("{e:#}"));
                    MSGS.lock().push(StoreMsg::AddonDone { name, verb: command, result });
                    ctx.request_repaint();
                });
                if let Err(e) = spawned {
                    self.notify_error(format!("Couldn't ask the add-on: {e}"));
                    return;
                }
            }
        }
        self.notify(plan.message);
    }
}

// ---------------------------------------------------------------------------------------------
// Spawning (only at click time; never in screenshot runs)

/// `steam <uri>` / `flatpak run com.valvesoftware.Steam <uri>` / `xdg-open <uri>`.
fn steam_command(uri: &str) -> (String, Vec<String>) {
    use crate::launch::SteamCmd;
    match crate::launch::Host::detect().steam {
        SteamCmd::Native => ("steam".into(), vec![uri.into()]),
        SteamCmd::Flatpak => ("flatpak".into(), vec!["run".into(), "com.valvesoftware.Steam".into(), uri.into()]),
        SteamCmd::XdgOpen => ("xdg-open".into(), vec![uri.into()]),
    }
}

/// Heroic's own binary, a running or installed AppImage, its Flatpak, or its URL handler.
fn heroic_command() -> (String, Vec<String>) {
    let home = dirs::home_dir().unwrap_or_default();
    crate::app_locate::command(&crate::app_locate::HEROIC, &home)
        .unwrap_or_else(|| ("xdg-open".into(), vec!["heroic://ping".into()]))
}

/// `argv` in a terminal window that stays open until Enter is pressed.
fn terminal_command(argv: &[String]) -> Option<(String, Vec<String>)> {
    let found = |p: &str| crate::launch::find_in_path(p).is_some();
    let env_term = std::env::var("TERMINAL").ok().filter(|t| !t.trim().is_empty() && found(t.trim()));
    let (term, flags) = match env_term {
        Some(t) => (t.trim().to_string(), vec!["-e"]),
        None => terminal_candidates().into_iter().find(|(t, _)| found(t)).map(|(t, f)| (t.to_string(), f))?,
    };
    Some((term, terminal_args(&flags, argv)))
}

fn terminal_candidates() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        ("xdg-terminal-exec", vec![]),
        ("konsole", vec!["-e"]),
        ("gnome-terminal", vec!["--"]),
        ("kgx", vec!["--"]),
        ("ptyxis", vec!["--"]),
        ("xfce4-terminal", vec!["-x"]),
        ("alacritty", vec!["-e"]),
        ("kitty", vec![]),
        ("foot", vec![]),
        ("wezterm", vec!["start", "--"]),
        ("xterm", vec!["-e"]),
    ]
}

/// `<flags> sh -c '<run "$@", then wait for Enter>' sh <argv…>` (argv passed as positional
/// parameters, never pasted into the script).
fn terminal_args(flags: &[&str], argv: &[String]) -> Vec<String> {
    const SCRIPT: &str = "\"$@\"; s=$?; echo; if [ $s -eq 0 ]; then echo 'Done.'; else echo \"Failed (exit $s).\"; fi; \
                          printf 'Press Enter to close this window. '; read -r _";
    let mut v: Vec<String> = flags.iter().map(|f| f.to_string()).collect();
    v.extend(["sh".into(), "-c".into(), SCRIPT.into(), "sh".into()]);
    v.extend(argv.iter().cloned());
    v
}

/// Start a program in its own process group with no stdio; reaped on a helper thread.
fn spawn_detached(program: String, args: Vec<String>, env: Vec<(String, String)>, cwd: Option<PathBuf>) {
    let spawned = std::thread::Builder::new().name("store-open".into()).spawn(move || {
        use std::os::unix::process::CommandExt as _;
        let mut cmd = std::process::Command::new(&program);
        cmd.args(&args)
            .envs(env)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0);
        if let Some(d) = cwd.filter(|d| d.is_dir()) {
            cmd.current_dir(d);
        }
        match cmd.spawn() {
            Ok(mut child) => {
                let _ = child.wait();
            }
            Err(e) => log::warn!("store action {program}: {e}"),
        }
    });
    if let Err(e) = spawned {
        log::warn!("couldn't start a store action: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(source: Source, id: &str, installed: bool, launch: Option<LaunchSpec>) -> Game {
        Game { id: 1, source, source_id: id.into(), installed, launch, ..Game::blank("Some Game") }
    }

    fn cmd(program: &str, args: &[&str], env: &[(&str, &str)]) -> LaunchSpec {
        LaunchSpec::Command {
            program: program.into(),
            args: args.iter().map(|a| a.to_string()).collect(),
            cwd: None,
            env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    fn action(p: Option<Plan>) -> Option<StoreAction> {
        p.map(|p| p.action)
    }

    #[test]
    fn steam_routes() {
        let g = game(Source::Steam, "620", false, Some(LaunchSpec::Steam { appid: 620 }));
        assert_eq!(action(install_plan(&g)), Some(StoreAction::Steam("steam://install/620".into())));
        assert_eq!(uninstall_plan(&g), None, "not installed");
        let g = game(Source::Steam, "620", true, Some(LaunchSpec::Steam { appid: 620 }));
        assert_eq!(install_plan(&g), None, "already installed");
        assert_eq!(action(uninstall_plan(&g)), Some(StoreAction::Steam("steam://uninstall/620".into())));
        assert_eq!(store_page_for(&g, &StoreUrls::new()).as_deref(), Some("https://store.steampowered.com/app/620"));
        let shortcut = game(Source::Steam, "shortcut:123", true, None);
        assert_eq!(uninstall_plan(&shortcut), None, "non-Steam shortcuts aren't Steam's to uninstall");
        assert_eq!(store_page_for(&shortcut, &StoreUrls::new()), None);
    }

    #[test]
    fn heroic_and_cli_routes() {
        let heroic = |src, id: &str, runner: &str, installed| {
            game(src, id, installed, Some(LaunchSpec::Url(crate::scan::heroic::launch_url(id, runner))))
        };
        let g = heroic(Source::Epic, "Fortnite", "legendary", false);
        assert_eq!(
            action(install_plan(&g)),
            Some(StoreAction::OpenUri("heroic://launch?appName=Fortnite&runner=legendary".into()))
        );
        let g = heroic(Source::Gog, "1423049311", "gog", false);
        assert_eq!(
            action(install_plan(&g)),
            Some(StoreAction::OpenUri("heroic://launch?appName=1423049311&runner=gog".into()))
        );
        let g = heroic(Source::Amazon, "amzn1.adg.product.x", "nile", true);
        assert_eq!(action(uninstall_plan(&g)), Some(StoreAction::OpenHeroic));
        let g = heroic(Source::Heroic, "sl_x", "sideload", true);
        assert_eq!(action(uninstall_plan(&g)), Some(StoreAction::OpenHeroic));
        assert_eq!(
            install_plan(&game(Source::Heroic, "sl_x", false, None)),
            None,
            "sideloaded apps aren't installable"
        );

        let g = game(Source::Epic, "Sugar", false, Some(cmd("legendary", &["launch", "Sugar"], &[])));
        assert_eq!(
            action(install_plan(&g)),
            Some(StoreAction::Terminal(vec!["legendary".into(), "install".into(), "Sugar".into()]))
        );
        let g = game(Source::Amazon, "amzn1.x", true, Some(cmd("nile", &["launch", "amzn1.x"], &[])));
        assert_eq!(
            action(uninstall_plan(&g)),
            Some(StoreAction::Terminal(vec!["nile".into(), "uninstall".into(), "amzn1.x".into()]))
        );
        // A user's own command doesn't change the route (that's launch_override), but a foreign
        // source launch has no install route.
        let g = game(Source::Epic, "x", false, Some(cmd("/opt/game/run.sh", &[], &[])));
        assert_eq!(install_plan(&g), None);
    }

    #[test]
    fn prefix_launchers_open_without_game_args() {
        let env = [("WINEPREFIX", "/p/bnet"), ("GAMEID", "umu-0")];
        let exe = "/p/bnet/drive_c/Program Files (x86)/Battle.net/Battle.net.exe";
        let g = game(Source::BattleNet, "wow", true, Some(cmd("umu-run", &[exe, "--exec=launch WoW"], &env)));
        let run = StoreAction::Run {
            program: "umu-run".into(),
            args: vec![exe.into()],
            env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            cwd: None,
        };
        assert_eq!(
            action(uninstall_plan(&g)),
            Some(StoreAction::PrefixLauncher {
                source: Source::BattleNet,
                source_id: "wow".into(),
                fallback: Some(Box::new(run))
            })
        );
        let rockstar = "C:\\Program Files\\Rockstar Games\\Launcher\\Launcher.exe";
        let g = game(Source::Rockstar, "gta5", false, Some(cmd("wine", &[rockstar, "-launchTitleInFolder", "x"], &[])));
        assert!(matches!(action(install_plan(&g)), Some(StoreAction::PrefixLauncher { fallback: Some(f), .. })
            if matches!(&*f, StoreAction::Run { args, .. } if *args == vec![rockstar.to_string()])));
        // A game started by its own .exe (HoYoPlay games): Uninstall asks the prefix scan for the
        // launcher; Install of a vanished one has nothing to go on.
        let g =
            game(Source::HoYoPlay, "hk4e", true, Some(cmd("umu-run", &["/p/drive_c/Genshin/GenshinImpact.exe"], &[])));
        assert!(matches!(action(uninstall_plan(&g)), Some(StoreAction::PrefixLauncher { fallback: None, .. })));
        let g =
            game(Source::HoYoPlay, "hk4e", false, Some(cmd("umu-run", &["/p/drive_c/Genshin/GenshinImpact.exe"], &[])));
        assert_eq!(install_plan(&g), None);
        assert!(
            !is_launcher_exe("C:\\Games\\launcher.exe"),
            "a bare launcher.exe outside a known launcher folder isn't one"
        );
        let g = game(Source::Ea, "x", true, Some(LaunchSpec::Url("link2ea://launchgame/x".into())));
        assert_eq!(uninstall_plan(&g), None);
    }

    #[test]
    fn addons_minecraft_moonlight_and_store_pages() {
        let g = game(Source::Addon, "romm:42", false, None);
        assert_eq!(
            action(install_plan(&g)),
            Some(StoreAction::Addon { addon: "romm".into(), game: "42".into(), command: "install" })
        );
        assert_eq!(install_plan(&game(Source::Addon, "broken", false, None)), None);
        for s in [Source::Minecraft, Source::Moonlight, Source::Desktop, Source::Manual, Source::Faugus] {
            assert_eq!(install_plan(&game(s, "x", false, None)), None, "{s:?}");
            assert_eq!(uninstall_plan(&game(s, "x", true, None)), None, "{s:?}");
        }
        let mut urls = StoreUrls::new();
        urls.insert((Source::Gog, "1423049311".into()), "https://www.gog.com/game/cyberpunk_2077".into());
        urls.insert((Source::Itch, "cave-1".into()), "https://someone.itch.io/game".into());
        assert_eq!(
            store_page_for(&game(Source::Gog, "1423049311", true, None), &urls).as_deref(),
            Some("https://www.gog.com/game/cyberpunk_2077")
        );
        assert_eq!(
            store_page_for(&game(Source::Itch, "cave-1", true, None), &urls).as_deref(),
            Some("https://someone.itch.io/game")
        );
        assert_eq!(store_page_for(&game(Source::Epic, "unknown", true, None), &urls), None, "no guessed pages");
        assert_eq!(
            store_page_for(&game(Source::Gog, "1207658919", true, None), &urls).as_deref(),
            Some("https://www.gog.com/en/games?query=Some%20Game"),
            "GOG without a known slug: store search"
        );
        assert_eq!(store_page_for(&game(Source::Amazon, "x", true, None), &urls), None);
    }

    #[test]
    fn terminal_args_pass_argv_as_parameters() {
        let v = terminal_args(&["-e"], &["legendary".into(), "install".into(), "a'b; rm -rf ~".into()]);
        assert_eq!(&v[..4], &["-e", "sh", "-c", v[3].as_str()]);
        assert_eq!(&v[4..], &["sh", "legendary", "install", "a'b; rm -rf ~"]);
        assert!(!v[3].contains("legendary"), "argv never pasted into the script");
    }

    /// Read-only look at this machine's store caches (`cargo test real_store_pages -- --ignored --nocapture`).
    #[test]
    #[ignore]
    fn real_store_pages() {
        let cfg = crate::config::Config::default();
        let mut ctx = crate::scan::ScanContext::from_config(&cfg);
        ctx.config.include_uninstalled = true;
        let urls = collect_store_urls(&ctx);
        let mut games = crate::scan::heroic::scan(&ctx).unwrap();
        games.extend(crate::scan::minecraft::scan(&ctx).unwrap());
        games.extend(crate::scan::moonlight::scan(&ctx).unwrap());
        for sg in games {
            let mut g = game(sg.source, &sg.source_id, sg.installed, sg.launch.clone());
            g.name = sg.name.clone();
            let plan = if g.installed { uninstall_plan(&g) } else { install_plan(&g) };
            println!(
                "{:?} {} installed={} page={:?} plan={:?}",
                g.source,
                g.name,
                g.installed,
                store_page_for(&g, &urls),
                plan.map(|p| p.action)
            );
        }
    }

    /// Screenshot runs never start anything; real runs do (a harmless `sh` script stands in for
    /// the launcher .exe).
    #[test]
    fn respects_no_launch() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("opened");
        let exe = dir.path().join("Battle.net.exe");
        std::fs::write(&exe, format!("touch '{}'\n", marker.display())).unwrap();
        let mut st = AppState::for_test(dir.path());
        let mut g = ScannedGame::new(Source::BattleNet, "wow", "World of Warcraft");
        g.launch = Some(cmd("sh", &[&exe.to_string_lossy(), "--exec=launch WoW"], &[]));
        let mut db = crate::db::Db::open(&st.paths.db_file()).unwrap();
        db.merge_scan(&[g], &[Source::BattleNet]).unwrap();
        st.reload_library();
        let id = st.library.games.iter().find(|g| g.name == "World of Warcraft").unwrap().id;
        assert!(st.can_uninstall(id) && !st.can_install(id));

        st.no_launch = true;
        st.uninstall_game(id);
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(!marker.exists(), "a screenshot run opened a launcher");
        assert!(st.toasts.iter().any(|t| t.text.contains("Skipped (screenshot run)")));

        st.no_launch = false;
        st.uninstall_game(id);
        let t = std::time::Instant::now();
        while !marker.exists() && t.elapsed() < std::time::Duration::from_secs(5) {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(marker.exists(), "the launcher command ran");
    }
}
