//! What a launch spec is underneath (`Base`): Steam games, URLs and their Steam forms, plain
//! commands; and how Steam itself is invoked.

use super::*;

/// The source's command before overrides.
pub(super) enum Base {
    Cmd {
        program: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cwd: Option<PathBuf>,
    },
    Steam {
        appid: u32,
    },
    /// `gameid` is the 64-bit `steam://rungameid/` id, `appid` its upper 32 bits.
    Shortcut {
        gameid: u64,
        appid: u32,
    },
    /// Some other `steam://` URL (opened through the Steam client).
    SteamUrl(String),
    /// Non-Steam URL (xdg-open).
    Url(String),
}

pub(super) fn steam_invocation(host: &Host, args: Vec<String>) -> (String, Vec<String>) {
    match host.steam {
        SteamCmd::Native => (str_s("steam"), args),
        SteamCmd::Flatpak => {
            let mut a = vec![str_s("run"), str_s("com.valvesoftware.Steam")];
            a.extend(args);
            (str_s("flatpak"), a)
        }
        SteamCmd::XdgOpen => {
            // Only URLs work through xdg-open; `-applaunch` callers handle XdgOpen themselves.
            (str_s("xdg-open"), args)
        }
    }
}

pub(super) fn base_of(spec: &LaunchSpec) -> Base {
    match spec {
        LaunchSpec::Steam { appid } => Base::Steam { appid: *appid },
        LaunchSpec::Url(u) => {
            let u = u.trim();
            match parse_steam_url(u) {
                Some(b) => b,
                None if u.to_ascii_lowercase().starts_with("steam://") => Base::SteamUrl(u.to_string()),
                None => Base::Url(u.to_string()),
            }
        }
        LaunchSpec::Command { program, args, cwd, env } => {
            // `steam steam://rungameid/N` (desktop shortcuts, manual entries): treat as Steam.
            if is_steam_program(program, args) {
                let urls: Vec<&String> = args.iter().filter(|a| !is_flatpak_steam_arg(a)).collect();
                if let [only] = urls.as_slice()
                    && let Some(b) = parse_steam_url(only)
                {
                    return b;
                }
            }
            Base::Cmd {
                // `@lodestar` = this Lodestar binary (helpers such as the Battle.net launcher).
                program: if program == crate::client_launch::SELF_PROGRAM {
                    std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_else(|_| "lodestar".into())
                } else {
                    program.clone()
                },
                args: args.clone(),
                env: env.clone(),
                cwd: cwd.clone(),
            }
        }
    }
}

pub(super) fn is_flatpak_steam_arg(a: &str) -> bool {
    a == "run" || a == "com.valvesoftware.Steam"
}

pub(super) fn is_steam_program(program: &str, args: &[String]) -> bool {
    let base = Path::new(program).file_name().and_then(|s| s.to_str()).unwrap_or(program);
    base == "steam"
        || (base == "flatpak" && args.iter().any(|a| a == "com.valvesoftware.Steam") && args.iter().any(|a| a == "run"))
}

/// `steam://rungameid/<id>` or `steam://run/<id>[/...]` -> Steam / Shortcut.
pub(super) fn parse_steam_url(u: &str) -> Option<Base> {
    let lower = u.to_ascii_lowercase();
    let rest = lower.strip_prefix("steam://rungameid/").or_else(|| lower.strip_prefix("steam://run/"))?;
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let after = &rest[digits.len()..];
    // `steam://run/<id>//args/` carries arguments; treat that as a plain URL.
    if !(after.is_empty() || after == "/") {
        return None;
    }
    let n: u64 = digits.parse().ok()?;
    if n > u32::MAX as u64 {
        Some(Base::Shortcut { gameid: n, appid: (n >> 32) as u32 })
    } else {
        Some(Base::Steam { appid: n as u32 })
    }
}

/// If a command line starts Steam with a game, how to track it.
pub(super) fn steam_kind_of(program: &str, args: &[String]) -> Option<ResolvedKind> {
    if !is_steam_program(program, args) {
        return None;
    }
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "-applaunch" {
            let id: u32 = it.next()?.parse().ok()?;
            return Some(ResolvedKind::Steam { appid: id });
        }
        match parse_steam_url(a) {
            Some(Base::Steam { appid }) => return Some(ResolvedKind::Steam { appid }),
            Some(Base::Shortcut { appid, .. }) => {
                return Some(ResolvedKind::SteamShortcut { appid });
            }
            _ => {}
        }
    }
    None
}
