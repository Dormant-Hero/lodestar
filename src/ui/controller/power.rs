//! Power options of the controller menu (shut down, restart, sleep, lock, log out) through
//! systemd-logind's command-line tools: `systemctl poweroff | reboot | suspend` and
//! `loginctl lock-session` / `loginctl terminate-session <this session>`. Each asks for a
//! confirmation first (the menu does that); screenshot runs and tests never run anything.
//!
//! CONTRACT: `Power::argv(session) -> Vec<String>` (pure); `available(p, has)`; `run(p) ->
//! Result<(), String>` spawns the command detached.

use crate::config::PowerItems;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Power {
    Shutdown,
    Restart,
    Suspend,
    Lock,
    Logout,
}

impl Power {
    pub const ALL: [Power; 5] = [Power::Shutdown, Power::Restart, Power::Suspend, Power::Lock, Power::Logout];

    /// The menu entry.
    pub fn label(self) -> &'static str {
        match self {
            Power::Shutdown => "Shut down",
            Power::Restart => "Restart",
            Power::Suspend => "Sleep",
            Power::Lock => "Lock",
            Power::Logout => "Log out",
        }
    }

    /// The confirmation's question.
    pub fn question(self) -> &'static str {
        match self {
            Power::Shutdown => "Shut down the computer?",
            Power::Restart => "Restart the computer?",
            Power::Suspend => "Put the computer to sleep?",
            Power::Lock => "Lock the screen?",
            Power::Logout => "Log out?",
        }
    }

    /// Settings → Menus.
    pub fn shown(self, p: &PowerItems) -> bool {
        match self {
            Power::Shutdown => p.shutdown,
            Power::Restart => p.restart,
            Power::Suspend => p.suspend,
            Power::Lock => p.lock,
            Power::Logout => p.logout,
        }
    }

    pub fn toggle(self, p: &mut PowerItems) {
        let v = match self {
            Power::Shutdown => &mut p.shutdown,
            Power::Restart => &mut p.restart,
            Power::Suspend => &mut p.suspend,
            Power::Lock => &mut p.lock,
            Power::Logout => &mut p.logout,
        };
        *v = !*v;
    }

    fn tool(self) -> &'static str {
        match self {
            Power::Shutdown | Power::Restart | Power::Suspend => "systemctl",
            Power::Lock | Power::Logout => "loginctl",
        }
    }

    /// The command. `session`: `$XDG_SESSION_ID` (log out ends that session; without one,
    /// logind's "self" = the caller's session).
    pub fn argv(self, session: Option<&str>) -> Vec<String> {
        let v: Vec<&str> = match self {
            Power::Shutdown => vec!["systemctl", "poweroff"],
            Power::Restart => vec!["systemctl", "reboot"],
            Power::Suspend => vec!["systemctl", "suspend"],
            Power::Lock => vec!["loginctl", "lock-session"],
            Power::Logout => {
                let s = session.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("self");
                return vec!["loginctl".into(), "terminate-session".into(), s.into()];
            }
        };
        v.into_iter().map(String::from).collect()
    }
}

/// The item can work here (its tool is installed).
pub fn available(p: Power, has: impl Fn(&str) -> bool) -> bool {
    has(p.tool())
}

/// The power items the menu shows: chosen in the settings and possible here.
pub fn menu_items(cfg: &PowerItems, has: impl Fn(&str) -> bool) -> Vec<Power> {
    Power::ALL.into_iter().filter(|p| p.shown(cfg) && available(*p, &has)).collect()
}

pub fn has_tool(t: &str) -> bool {
    crate::launch::find_in_path(t).is_some()
}

/// Run it (detached; the computer may be gone a moment later).
pub fn run(p: Power) -> Result<(), String> {
    let session = std::env::var("XDG_SESSION_ID").ok();
    let argv = p.argv(session.as_deref());
    log::info!("power: running {argv:?}");
    let child = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Couldn't run {}: {e}", argv[0]))?;
    let _ = std::thread::Builder::new().name("power".into()).spawn(move || {
        if let Ok(out) = child.wait_with_output()
            && !out.status.success()
        {
            log::warn!("{argv:?} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_of_each_power_item() {
        assert_eq!(Power::Shutdown.argv(None), ["systemctl", "poweroff"]);
        assert_eq!(Power::Restart.argv(None), ["systemctl", "reboot"]);
        assert_eq!(Power::Suspend.argv(None), ["systemctl", "suspend"]);
        assert_eq!(Power::Lock.argv(Some("3")), ["loginctl", "lock-session"]);
        assert_eq!(Power::Logout.argv(Some("3")), ["loginctl", "terminate-session", "3"]);
        assert_eq!(Power::Logout.argv(Some(" ")), ["loginctl", "terminate-session", "self"]);
        assert_eq!(Power::Logout.argv(None), ["loginctl", "terminate-session", "self"]);
    }

    #[test]
    fn menu_shows_chosen_items_whose_tool_exists() {
        let d = PowerItems::default();
        let all = |_: &str| true;
        assert_eq!(menu_items(&d, all), vec![Power::Shutdown, Power::Restart, Power::Suspend]);
        let mut p = d;
        Power::Lock.toggle(&mut p);
        Power::Shutdown.toggle(&mut p);
        assert_eq!(menu_items(&p, all), vec![Power::Restart, Power::Suspend, Power::Lock]);
        assert_eq!(menu_items(&p, |t| t == "loginctl"), vec![Power::Lock]);
        assert!(menu_items(&p, |_| false).is_empty());
    }
}
