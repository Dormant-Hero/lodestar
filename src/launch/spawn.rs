//! Spawning a resolved launch: the spawn job (on its own thread), the checks before it, and
//! the per-launch log file.

use super::*;

/// Pre-launch state: before-start scripts still running.
pub(super) const PRE_PENDING: u8 = 0;

pub(super) const PRE_SPAWNED: u8 = 1;

pub(super) const PRE_CANCELLED: u8 = 2;

/// A store link (`heroic://`, `lutris:`, `itch://`) that no installed desktop entry handles (an
/// AppImage that was never integrated): hand it to the app itself instead of `xdg-open`.
pub(super) fn direct_url(r: &Resolved) -> Option<Resolved> {
    if r.kind != ResolvedKind::OpenUrl || r.program != "xdg-open" || r.args.len() != 1 {
        return None;
    }
    let home = dirs::home_dir()?;
    let (program, args) = crate::app_locate::direct_url_command(&home, &r.args[0])?;
    log::info!("no handler registered for {}; opening it with {program}", r.args[0]);
    Some(Resolved { program, args, ..r.clone() })
}

/// `heroic://…` handed to the desktop.
pub(super) fn is_heroic_link(r: &Resolved) -> bool {
    r.kind == ResolvedKind::OpenUrl && r.args.iter().any(|a| a.trim().to_ascii_lowercase().starts_with("heroic://"))
}

/// The Wine prefix a store-client launch runs in: `WINEPREFIX` of the command, or the Battle.net
/// helper's `--prefix`.
pub(super) fn launch_prefix(r: &Resolved) -> Option<PathBuf> {
    if let Some((_, v)) = r.env.iter().rev().find(|(k, _)| k == "WINEPREFIX") {
        return Some(PathBuf::from(v));
    }
    let i = r.args.iter().position(|a| a == "--prefix")?;
    r.args.get(i + 1).map(PathBuf::from)
}

/// Everything needed to spawn a resolved launch (on the UI thread or after the before-start
/// scripts on the pre-launch thread).
pub(super) struct SpawnJob {
    pub(super) id: GameId,
    pub(super) name: String,
    pub(super) r: Resolved,
    pub(super) program: String,
    pub(super) log_dir: PathBuf,
    pub(super) log_path: PathBuf,
    pub(super) mode: track::Mode,
    pub(super) folder: Option<PathBuf>,
    pub(super) announced: bool,
    /// `track::TAG_VAR` value for this launch.
    pub(super) tag: String,
}

impl SpawnJob {
    pub(super) fn spawn(self, monitor: &Sender<track::Msg>) -> anyhow::Result<()> {
        use std::os::unix::process::CommandExt;
        let SpawnJob { id, name, r, program, log_dir, log_path, mode, folder, announced, tag } = self;
        // The steam command may *become* the Steam client (when Steam wasn't running) and print
        // for as long as Steam runs; keep that out of the game log. Steam games' own output
        // goes to Steam anyway.
        let steam = matches!(r.kind, ResolvedKind::Steam { .. } | ResolvedKind::SteamShortcut { .. });
        let log = if steam { None } else { open_log(&log_dir, &log_path, &name, &r) };
        for d in &r.make_dirs {
            if let Err(e) = std::fs::create_dir_all(d) {
                log::warn!("{}: {e}", d.display());
            }
        }
        let mut cmd = std::process::Command::new(&program);
        cmd.args(&r.args).stdin(std::process::Stdio::null());
        for (k, _) in std::env::vars_os() {
            if k.to_str().is_some_and(strip_env_var) {
                cmd.env_remove(&k);
            }
        }
        // Lodestar's own helper (`--client-launch`) must write its client records under the
        // same root the app reads them from; the clients it starts lose the variable again.
        if is_self_program(&program)
            && let Some(home) = std::env::var_os("LODESTAR_HOME")
        {
            cmd.env("LODESTAR_HOME", home);
        }
        cmd.envs(r.env.iter().map(|(k, v)| (k, v)));
        // Every process of this launch inherits the tag, including ones that detach into their
        // own session (Wine/Proton do), so tracking and Stop can find all of them.
        cmd.env(track::TAG_VAR, &tag);
        if let Some(cwd) = &r.cwd {
            cmd.current_dir(cwd);
        }
        match log.as_ref().and_then(|f| Some((f.try_clone().ok()?, f.try_clone().ok()?))) {
            Some((out, err)) => {
                cmd.stdout(out).stderr(err);
            }
            None => {
                cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
            }
        }
        // SAFETY: setsid(), close_range()/fcntl() are async-signal-safe syscalls and touch no
        // memory of the parent.
        // A new session keeps the game alive when Lodestar quits or gets Ctrl-C, and gives us
        // a session id (== child pid) to follow forked launchers by.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                mark_inherited_fds_cloexec();
                Ok(())
            });
        }
        let child = cmd.spawn().with_context(|| format!("couldn't start {}", r.program))?;
        log::info!("launch {id} ({name}): {} [pid {}, {mode:?}]", r.display, child.id());
        let _ = monitor.send(track::Msg::Track { id, child, mode, log: log_path, tag, folder, announced });
        Ok(())
    }
}

/// Checks that can fail at once (shown to the user right away): the working directory exists and
/// the program can be found. Returns the program to run.
pub(super) fn check_runnable(r: &Resolved) -> anyhow::Result<String> {
    if let Some(cwd) = &r.cwd
        && !cwd.is_dir()
    {
        bail!("working directory doesn't exist: {}", cwd.display());
    }
    // `./run.sh` with a working directory means relative to that directory.
    let in_cwd = |p: &str| match &r.cwd {
        Some(cwd) if p.contains('/') && Path::new(p).is_relative() => cwd.join(p).to_string_lossy().into_owned(),
        _ => p.to_string(),
    };
    let program = in_cwd(&r.program);
    if find_in_command_path(&program, &r.env).is_none() {
        bail!("program not found: {}", r.program);
    }
    // Under wrappers (Keep screen awake, Gamescope, GameMode …) the game's own program is
    // further in: a missing one is still caught here, not after the wrapper started.
    if r.kind == ResolvedKind::Process {
        let mut argv = vec![r.program.clone()];
        argv.extend(r.args.iter().cloned());
        // An `env PATH=…` in the chain changes where a bare core is looked up.
        let mut env = r.env.clone();
        if let Some(p) = argv.iter().rev().find_map(|w| w.strip_prefix("PATH=")) {
            env.push(("PATH".into(), p.to_string()));
        }
        if let Some(core) = compose::core_program(&argv, self_exe())
            && core != r.program
            && core != "@lodestar"
            && !core.contains(char::is_whitespace)
            && find_in_command_path(&in_cwd(&core), &env).is_none()
        {
            bail!("program not found: {core}");
        }
    }
    Ok(program)
}

pub(super) fn log_file_name(id: GameId, name: &str) -> String {
    let slug: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let slug: String = slug.chars().take(40).collect();
    if slug.is_empty() { format!("{id}.log") } else { format!("{id}-{slug}.log") }
}

pub(super) fn open_log(dir: &Path, path: &Path, name: &str, r: &Resolved) -> Option<std::fs::File> {
    use std::io::Write;
    if let Err(e) = std::fs::create_dir_all(dir) {
        log::warn!("log dir {}: {e}", dir.display());
        return None;
    }
    match std::fs::File::create(path) {
        Ok(mut f) => {
            let _ = writeln!(f, "# Lodestar: {name}\n# started {}\n# {}\n", now_unix(), r.display);
            Some(f)
        }
        Err(e) => {
            log::warn!("log file {}: {e}", path.display());
            None
        }
    }
}

pub(super) fn now_unix_nanos() -> u128 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}
