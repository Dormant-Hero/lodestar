//! Game scripts: shell snippets run before a game starts, after it started and after it exited
//! (per game and the global ones from Settings), and the editor's Test button.
//!
//! Scripts run with `sh -c` as the user (never elevated; refused when Lodestar itself runs as
//! root), in their own session with stdin closed, never on the UI thread. A script still running
//! when its timeout is up is ended (SIGTERM, then SIGKILL); processes it left in the background
//! after it exited are left alone. The first line of output is kept for the toast.

use crossbeam_channel::Sender;
use parking_lot::Mutex;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

/// How long a script may run before it is ended (before-start scripts hold up the launch).
pub const SCRIPT_TIMEOUT: Duration = Duration::from_secs(30);

/// Output kept per stream (only the first line is shown).
const OUTPUT_CAP: usize = 16 * 1024;

/// One script to run, with what to call it in messages ("Before-start script").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Script {
    pub label: String,
    pub text: String,
}

/// Scripts around one game session plus what they run with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SessionScripts {
    pub before_start: Vec<Script>,
    pub after_start: Vec<Script>,
    pub after_exit: Vec<Script>,
    /// `LODESTAR_GAME_NAME`, `LODESTAR_GAME_ID`, `LODESTAR_INSTALL_DIR`.
    pub env: Vec<(String, String)>,
    /// Working directory (the install folder when it exists).
    pub cwd: Option<PathBuf>,
}

impl SessionScripts {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.before_start.is_empty() && self.after_start.is_empty() && self.after_exit.is_empty()
    }

    /// The scripts for a session of `game`: global ones (unless the game opted out) and the
    /// game's own. Before start: global first, then the game's; after start / exit: the game's
    /// first, then the global one (Playnite's order).
    pub fn for_game(game: &crate::model::Game, global: &crate::model::GameScripts) -> SessionScripts {
        let own = &game.edits.scripts;
        let use_global = own.use_global;
        let mk = |label: &str, text: &str| {
            (!text.trim().is_empty()).then(|| Script { label: label.into(), text: text.to_string() })
        };
        let g = |label: &str, text: &str| if use_global { mk(label, text) } else { None };
        let before_start =
            [g("Global before-start script", &global.before_start), mk("Before-start script", &own.before_start)];
        let after_start =
            [mk("After-start script", &own.after_start), g("Global after-start script", &global.after_start)];
        let after_exit = [mk("After-exit script", &own.after_exit), g("Global after-exit script", &global.after_exit)];
        SessionScripts {
            before_start: before_start.into_iter().flatten().collect(),
            after_start: after_start.into_iter().flatten().collect(),
            after_exit: after_exit.into_iter().flatten().collect(),
            env: script_env(game),
            cwd: game.install_dir.clone().filter(|d| d.is_dir()),
        }
    }
}

/// Environment a game's scripts get.
pub fn script_env(game: &crate::model::Game) -> Vec<(String, String)> {
    vec![
        ("LODESTAR_GAME_NAME".into(), game.display_name().to_string()),
        ("LODESTAR_GAME_ID".into(), game.id.to_string()),
        (
            "LODESTAR_INSTALL_DIR".into(),
            game.install_dir.as_ref().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
        ),
    ]
}

/// How a script run went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptOutcome {
    /// Exit code (None: killed by a signal, timed out or never started).
    pub code: Option<i32>,
    pub timed_out: bool,
    /// First non-empty line of output (stdout, else stderr).
    pub first_line: Option<String>,
    /// Couldn't start it at all.
    pub error: Option<String>,
    pub elapsed: Duration,
}

impl ScriptOutcome {
    pub fn ok(&self) -> bool {
        self.error.is_none() && !self.timed_out && self.code == Some(0)
    }

    /// "Before-start script failed (exit code 3): no such file" / "… finished: hello".
    pub fn summary(&self, label: &str) -> String {
        let line = self.first_line.as_deref().map(|l| format!(": {l}")).unwrap_or_default();
        if let Some(e) = &self.error {
            return format!("{label} couldn't run: {e}");
        }
        if self.timed_out {
            return format!("{label} was stopped after {}s{line}", self.elapsed.as_secs());
        }
        match self.code {
            Some(0) => format!("{label} finished (exit code 0){line}"),
            Some(c) => format!("{label} failed (exit code {c}){line}"),
            None => format!("{label} was killed{line}"),
        }
    }
}

/// A finished script, for the UI (toast) or the terminal (`--play`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptNotice {
    pub label: String,
    pub outcome: ScriptOutcome,
    /// Tell the user even when it went fine (the Test button).
    pub always_report: bool,
}

impl ScriptNotice {
    pub fn text(&self) -> String {
        self.outcome.summary(&self.label)
    }
    /// Worth showing at all.
    pub fn should_report(&self) -> bool {
        self.always_report || !self.outcome.ok()
    }
}

/// Run one script and wait for it (call from a background thread only).
pub fn run_blocking(script: &str, env: &[(String, String)], cwd: Option<&Path>, timeout: Duration) -> ScriptOutcome {
    use std::os::unix::process::CommandExt;
    let t0 = Instant::now();
    let fail = |e: String| ScriptOutcome {
        code: None,
        timed_out: false,
        first_line: None,
        error: Some(e),
        elapsed: t0.elapsed(),
    };
    // SAFETY: plain syscall.
    if unsafe { libc::geteuid() } == 0 {
        return fail("scripts are never run as root".into());
    }
    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c")
        .arg(script)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for (k, _) in std::env::vars_os() {
        if k.to_str().is_some_and(super::strip_env_var) {
            cmd.env_remove(&k);
        }
    }
    cmd.envs(env.iter().map(|(k, v)| (k, v)));
    if let Some(d) = cwd.filter(|d| d.is_dir()) {
        cmd.current_dir(d);
    } else if let Some(h) = dirs::home_dir() {
        cmd.current_dir(h);
    }
    // SAFETY: setsid / close_range / fcntl are async-signal-safe syscalls.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            super::mark_inherited_fds_cloexec();
            Ok(())
        });
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return fail(format!("{e}")),
    };
    let pid = child.id() as i32;
    let (done_tx, done_rx) = crossbeam_channel::unbounded::<()>();
    let mut bufs = vec![];
    let streams: Vec<Box<dyn Read + Send>> = [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .collect();
    let readers = streams.len();
    for mut s in streams {
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        bufs.push(buf.clone());
        let tx = done_tx.clone();
        let spawned = std::thread::Builder::new().name("script-output".into()).spawn(move || {
            let mut chunk = [0u8; 4096];
            loop {
                match s.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut b = buf.lock();
                        if b.len() < OUTPUT_CAP {
                            let take = n.min(OUTPUT_CAP - b.len());
                            b.extend_from_slice(&chunk[..take]);
                        }
                    }
                }
            }
            let _ = tx.send(());
        });
        if spawned.is_err() {
            let _ = done_tx.send(());
        }
    }
    drop(done_tx);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {}
            Err(_) => break None,
        }
        if t0.elapsed() >= timeout {
            timed_out = true;
            // SAFETY: signals the script's own session/process group (setsid above).
            unsafe { libc::kill(-pid, libc::SIGTERM) };
            let t = Instant::now();
            let st = loop {
                if let Ok(Some(st)) = child.try_wait() {
                    break Some(st);
                }
                if t.elapsed() > Duration::from_secs(2) {
                    unsafe { libc::kill(-pid, libc::SIGKILL) };
                    break child.wait().ok();
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            break st;
        }
        std::thread::sleep(Duration::from_millis(15));
    };
    // Output readers end at EOF; a process the script left running in the background can keep
    // the pipe open, so don't wait for that.
    let wait_until = Instant::now() + Duration::from_millis(300);
    for _ in 0..readers {
        if done_rx.recv_deadline(wait_until).is_err() {
            break;
        }
    }
    let first = |b: &Arc<Mutex<Vec<u8>>>| first_line(&String::from_utf8_lossy(&b.lock()));
    let first_line = bufs.iter().find_map(first);
    ScriptOutcome { code: status.and_then(|s| s.code()), timed_out, first_line, error: None, elapsed: t0.elapsed() }
}

fn first_line(s: &str) -> Option<String> {
    let l = s.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut out: String = l.chars().take(160).collect();
    if l.chars().count() > 160 {
        out.push('…');
    }
    Some(out)
}

/// Runs scripts on background threads and reports each result on a channel.
#[derive(Clone)]
pub struct ScriptRunner {
    tx: Sender<ScriptNotice>,
    ctx: egui::Context,
    running: Arc<AtomicUsize>,
}

impl ScriptRunner {
    pub fn new(ctx: &egui::Context, tx: Sender<ScriptNotice>) -> ScriptRunner {
        ScriptRunner { tx, ctx: ctx.clone(), running: Arc::new(AtomicUsize::new(0)) }
    }

    /// Run `scripts` one after the other in the background (never waits).
    pub fn spawn(&self, scripts: Vec<Script>, env: Vec<(String, String)>, cwd: Option<PathBuf>, always_report: bool) {
        if scripts.is_empty() {
            return;
        }
        let me = self.clone();
        self.running.fetch_add(1, Ordering::SeqCst);
        let r = std::thread::Builder::new().name("game-script".into()).spawn(move || {
            for s in &scripts {
                me.run_one(s, &env, cwd.as_deref(), always_report);
            }
            me.running.fetch_sub(1, Ordering::SeqCst);
            me.ctx.request_repaint();
        });
        if let Err(e) = r {
            self.running.fetch_sub(1, Ordering::SeqCst);
            log::warn!("couldn't start a script thread: {e}");
        }
    }

    /// Run one script on the calling (background) thread and report it.
    pub fn run_one(
        &self,
        s: &Script,
        env: &[(String, String)],
        cwd: Option<&Path>,
        always_report: bool,
    ) -> ScriptOutcome {
        let outcome = run_blocking(&s.text, env, cwd, SCRIPT_TIMEOUT);
        log::info!("{}", outcome.summary(&s.label));
        let _ = self.tx.send(ScriptNotice { label: s.label.clone(), outcome: outcome.clone(), always_report });
        self.ctx.request_repaint();
        outcome
    }

    /// Script batches still running.
    pub fn busy(&self) -> bool {
        self.running.load(Ordering::SeqCst) > 0
    }

    /// Count a batch run elsewhere (the pre-launch thread) as running.
    pub fn hold(&self) -> RunningGuard {
        self.running.fetch_add(1, Ordering::SeqCst);
        RunningGuard(self.running.clone())
    }
}

pub struct RunningGuard(Arc<AtomicUsize>);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn env() -> Vec<(String, String)> {
        vec![("LODESTAR_GAME_NAME".into(), "Some Game".into())]
    }

    #[test]
    fn runs_with_env_and_reports_first_line_and_code() {
        let o =
            run_blocking("echo; echo \"hi $LODESTAR_GAME_NAME\"; echo second; exit 3", &env(), None, SCRIPT_TIMEOUT);
        assert_eq!(o.code, Some(3));
        assert_eq!(o.first_line.as_deref(), Some("hi Some Game"));
        assert!(!o.ok());
        assert_eq!(o.summary("Test"), "Test failed (exit code 3): hi Some Game");
        let ok = run_blocking("echo oops >&2", &[], None, SCRIPT_TIMEOUT);
        assert!(ok.ok());
        assert_eq!(ok.first_line.as_deref(), Some("oops"), "stderr used when stdout is empty");
    }

    #[test]
    fn lodestar_internal_vars_do_not_leak_but_ours_do() {
        let o = run_blocking("echo \"[${LODESTAR_GAME_TAG-}][${LODESTAR_GAME_NAME}]\"", &env(), None, SCRIPT_TIMEOUT);
        assert_eq!(o.first_line.as_deref(), Some("[][Some Game]"));
    }

    #[test]
    fn timeout_ends_the_script_and_background_children_do_not_hold_it_up() {
        let o = run_blocking("echo started; sleep 20", &[], None, Duration::from_millis(400));
        assert!(o.timed_out);
        assert!(o.elapsed < Duration::from_secs(4), "{:?}", o.elapsed);
        assert_eq!(o.first_line.as_deref(), Some("started"));
        // A script that leaves something running exits at once and isn't waited for.
        let t = Instant::now();
        let o = run_blocking("sleep 3 & echo bg", &[], None, SCRIPT_TIMEOUT);
        assert!(o.ok());
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

    #[test]
    fn runs_in_the_given_folder() {
        let dir = tempfile::tempdir().unwrap();
        let o = run_blocking("touch made-here", &[], Some(dir.path()), SCRIPT_TIMEOUT);
        assert!(o.ok());
        assert!(dir.path().join("made-here").exists());
    }

    fn game_with_scripts(s: GameScripts) -> Game {
        let mut g = crate::db::Db::open_memory().unwrap();
        let id = g.add_manual("G", LaunchSpec::Url("x".into()), Art::default()).unwrap();
        let mut game = g.get(id).unwrap().unwrap();
        game.edits.scripts = s;
        game
    }

    #[test]
    fn global_and_game_scripts_in_order_and_opt_out() {
        let global = GameScripts {
            before_start: "gb".into(),
            after_start: "ga".into(),
            after_exit: "ge".into(),
            use_global: true,
        };
        let g =
            game_with_scripts(GameScripts { before_start: "b".into(), after_exit: "e".into(), ..Default::default() });
        let s = SessionScripts::for_game(&g, &global);
        let texts = |v: &[Script]| v.iter().map(|s| s.text.clone()).collect::<Vec<_>>();
        assert_eq!(texts(&s.before_start), ["gb", "b"]);
        assert_eq!(texts(&s.after_start), ["ga"]);
        assert_eq!(texts(&s.after_exit), ["e", "ge"]);
        assert!(s.env.iter().any(|(k, v)| k == "LODESTAR_GAME_ID" && *v == g.id.to_string()));
        let g = game_with_scripts(GameScripts { before_start: "  ".into(), use_global: false, ..Default::default() });
        assert!(SessionScripts::for_game(&g, &global).is_empty());
    }
}
