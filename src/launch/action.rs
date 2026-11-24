//! The game's own actions (Edit → Actions): `{InstallDir}`-style variables, and how an action
//! runs, including a Windows program run with the game's Proton and prefix.

use super::*;

/// Values for `{InstallDir}`, `{Name}`, `{GameId}`, `{Prefix}`, `{Proton}` in actions, and for
/// emulated games `{Rom}` (the ROM that is launched), `{RomDir}` (its folder) and `{Platform}`
/// (the console's platform id, e.g. `sony_playstation2`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionVars {
    pub install_dir: Option<String>,
    pub name: String,
    pub id: String,
    pub prefix: Option<String>,
    pub proton: Option<String>,
    pub rom: Option<String>,
    pub rom_dir: Option<String>,
    pub platform: Option<String>,
}

impl ActionVars {
    pub fn of(game: &Game) -> ActionVars {
        let env: &[(String, String)] = match &game.launch {
            Some(LaunchSpec::Command { env, .. }) => env,
            _ => &[],
        };
        let get = |k: &str| env.iter().rev().find(|(ek, _)| ek == k).map(|(_, v)| v.clone()).filter(|v| !v.is_empty());
        let steam_prefix = || {
            // <library>/steamapps/common/<game> -> <library>/steamapps/compatdata/<appid>/pfx
            let appid = game.steam_appid()?;
            let common = game.install_dir.as_ref()?.parent()?;
            (common.file_name()? == "common").then_some(())?;
            Some(common.parent()?.join(format!("compatdata/{appid}/pfx")).to_string_lossy().into_owned())
        };
        let emu = game.emu.as_ref();
        ActionVars {
            install_dir: game.install_dir.as_ref().map(|d| d.to_string_lossy().into_owned()),
            name: game.display_name().to_string(),
            id: game.id.to_string(),
            prefix: get("WINEPREFIX")
                .or_else(|| get("STEAM_COMPAT_DATA_PATH").map(|p| format!("{}/pfx", p.trim_end_matches('/'))))
                .or_else(steam_prefix),
            proton: get("PROTONPATH").or_else(|| get("PROTON_PATH")),
            rom: emu.map(|e| e.rom.to_string_lossy().into_owned()).filter(|s| !s.is_empty()),
            rom_dir: emu
                .filter(|e| !e.rom.as_os_str().is_empty())
                .and_then(|e| e.rom.parent())
                .map(|d| d.to_string_lossy().into_owned()),
            platform: emu.map(|e| e.platform.clone()).filter(|s| !s.is_empty()),
        }
    }

    /// Replace the variables in `s`; returns the names of variables that had no value (they
    /// expand to nothing).
    pub fn expand(&self, s: &str, missing: &mut Vec<&'static str>) -> String {
        if !s.contains('{') {
            return s.to_string();
        }
        let mut out = s.to_string();
        let vars: [(&'static str, Option<&str>); 8] = [
            ("{InstallDir}", self.install_dir.as_deref()),
            ("{Name}", Some(&self.name)),
            ("{GameId}", Some(&self.id)),
            ("{Prefix}", self.prefix.as_deref()),
            ("{Proton}", self.proton.as_deref()),
            ("{Rom}", self.rom.as_deref()),
            ("{RomDir}", self.rom_dir.as_deref()),
            ("{Platform}", self.platform.as_deref()),
        ];
        for (k, v) in vars {
            if out.contains(k) {
                if v.is_none() && !missing.contains(&k) {
                    missing.push(k);
                }
                out = out.replace(k, v.unwrap_or(""));
            }
        }
        out
    }
}

/// `~` / `~/…` -> the home directory.
pub(super) fn expand_tilde(s: &str, host: &Host) -> String {
    match (&host.home, s.strip_prefix('~')) {
        (Some(h), Some(rest)) if rest.is_empty() || rest.starts_with('/') => {
            h.join(rest.trim_start_matches('/')).to_string_lossy().into_owned()
        }
        _ => s.to_string(),
    }
}

/// A user-defined action: Command (program + shell-like args, the game's launch options applied
/// around it), Url (system handler) or File (default application).
pub(super) fn resolve_user_action(
    game: &Game,
    a: &GameAction,
    ov: Option<&LaunchOverride>,
    host: &Host,
    t: &Toggles,
) -> anyhow::Result<Resolved> {
    let vars = ActionVars::of(game);
    let mut missing = vec![];
    let label = if a.name.trim().is_empty() {
        "This action".to_string()
    } else {
        format!("The action “{}”", a.name.trim())
    };
    // Emulator actions, and Program actions without a program on an emulated game: the game's
    // emulator (its sandbox flags, the ROM and its `--` filled in), the action's arguments added.
    if a.kind == ActionKind::Emulator || a.kind == ActionKind::Command && game.emu.is_some() && a.path.trim().is_empty()
    {
        let spec = crate::emulation::registry::action_spec_now(game, a).map_err(|e| anyhow!("{label}: {e}"))?;
        let opts_only = ov.map(|o| LaunchOverride { options: o.options.clone(), command: None, cwd: None });
        return resolve_parts(Some(&spec), opts_only.as_ref(), None, host, Some(t));
    }
    let path = vars.expand(a.path.trim(), &mut missing);
    if path.trim().is_empty() {
        bail!("{label} has nothing to run — set its program, URL or file");
    }
    let mut r = match a.kind {
        ActionKind::Emulator => bail!("{label}: not an emulated game"),
        ActionKind::Command => {
            let program = expand_tilde(&path, host);
            let args: Vec<String> = split_words(&a.args, host, true)
                .with_context(|| format!("{label}: arguments"))?
                .into_iter()
                .map(|w| vars.expand(&w.text, &mut missing))
                .collect();
            let cwd = match a.cwd.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
                Some(c) => Some(PathBuf::from(expand_tilde(&vars.expand(c, &mut missing), host))),
                // Games often expect to start in their own folder.
                None => Path::new(&program)
                    .is_absolute()
                    .then(|| Path::new(&program).parent().map(Path::to_path_buf))
                    .flatten(),
            };
            let explicit_cwd = a.cwd.as_deref().is_some_and(|c| !c.trim().is_empty());
            let mut win_notes = vec![];
            let spec = windows_action_spec(
                &program,
                &args,
                explicit_cwd.then(|| cwd.clone()).flatten(),
                &vars,
                host,
                &mut win_notes,
            )
            .unwrap_or(LaunchSpec::Command { program, args, cwd, env: vec![] });
            // The game's launch options wrap the action; its custom command is what the action
            // replaces.
            let opts_only = ov.map(|o| LaunchOverride { options: o.options.clone(), command: None, cwd: None });
            let mut r = resolve_parts(Some(&spec), opts_only.as_ref(), None, host, Some(t))?;
            r.notes.extend(win_notes);
            if ov.is_some_and(|o| o.command.as_deref().is_some_and(|c| !c.trim().is_empty())) {
                r.notes.push(format!("{label} runs instead of the custom command (the launch options still apply)."));
            }
            r
        }
        ActionKind::Url => {
            let mut r = resolve_parts(Some(&LaunchSpec::Url(path)), None, None, host, Some(t))?;
            if !a.args.trim().is_empty() {
                r.notes.push("Arguments are ignored for URL actions.".into());
            }
            r
        }
        // Documents: game options don't apply to opening a file.
        ActionKind::File => {
            let p = expand_tilde(&path, host);
            let mut r = finish(str_s("xdg-open"), vec![p], vec![], None, ResolvedKind::OpenUrl, vec![]);
            if !a.args.trim().is_empty() {
                r.notes.push("Arguments are ignored for file actions.".into());
            }
            r
        }
    };
    if !missing.is_empty() {
        r.notes.push(format!("Not known for this game, left empty: {}", missing.join(", ")));
    }
    Ok(r)
}

/// A Windows program (`.exe` / `.bat` / `.msi`) as a command action runs with Proton, like the
/// game: in the game's Wine prefix (the launch's `WINEPREFIX`, Steam's compatdata, else the
/// prefix the program is installed in) with the game's Proton build (`PROTONPATH`, else the one
/// that prefix was last run with). None (with a note) when that can't be told: it then runs as
/// written, so a hand-written `umu-run …` / `wine …` action is left alone.
pub(super) fn windows_action_spec(
    program: &str,
    args: &[String],
    cwd: Option<PathBuf>,
    vars: &ActionVars,
    host: &Host,
    notes: &mut Vec<String>,
) -> Option<LaunchSpec> {
    let exe = Path::new(program);
    let ext = exe.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if !matches!(ext.as_str(), "exe" | "bat" | "msi") {
        return None;
    }
    let Some(prefix) = vars.prefix.as_ref().map(PathBuf::from).or_else(|| crate::addgame::prefix_of(exe)) else {
        notes.push(
            "A Windows program, but no Wine prefix is known for this game: it runs as written \
             (start it with umu-run, or put it inside the game's prefix)."
                .into(),
        );
        return None;
    };
    let home = host.home.clone().unwrap_or_else(|| PathBuf::from("/"));
    let proton = match &vars.proton {
        Some(p) => crate::addgame::ProtonVersion {
            name: Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
            path: PathBuf::from(p),
        },
        None => {
            let protons = cached_protons(&home);
            match crate::addgame::proton_for_prefix(&prefix, &protons) {
                Some(p) => p.clone(),
                None => {
                    notes.push("A Windows program, but no Proton build was found: it runs as written.".into());
                    return None;
                }
            }
        }
    };
    let umu = crate::addgame::find_umu(&home);
    let mut spec = crate::addgame::windows_launch_with(exe, &proton, &prefix, umu, &home);
    if let LaunchSpec::Command { args: a, cwd: c, .. } = &mut spec {
        a.extend(args.iter().cloned());
        if cwd.is_some() {
            *c = cwd;
        }
    }
    notes.push(format!("Runs with {} in the prefix {}.", proton.name, prefix.display()));
    Some(spec)
}

/// Installed Proton builds (a directory walk of a few ms), kept for 30 s: the action editor
/// previews on every edit of an action.
pub(super) fn cached_protons(home: &Path) -> Vec<crate::addgame::ProtonVersion> {
    type Cache = Option<(std::time::Instant, PathBuf, Vec<crate::addgame::ProtonVersion>)>;
    static CACHE: std::sync::Mutex<Cache> = std::sync::Mutex::new(None);
    let mut c = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, h, v)) = c.as_ref()
        && h == home
        && at.elapsed() < Duration::from_secs(30)
    {
        return v.clone();
    }
    let mut ctx = crate::scan::ScanContext::from_config(&crate::config::Config::default());
    ctx.home = home.to_path_buf();
    let v = crate::addgame::proton_versions(&ctx);
    *c = Some((std::time::Instant::now(), home.to_path_buf(), v.clone()));
    v
}
