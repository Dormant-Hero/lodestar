//! Minimising native Wayland windows (Heroic as a Wayland Electron app, Wine's Wayland driver …),
//! which X11 (winhide.rs) can't see. Wayland itself lets no program minimise another's windows,
//! so this asks the compositor through its own control interface:
//!
//! * KDE Plasma (KWin): a tiny KWin script loaded over D-Bus (`org.kde.KWin /Scripting`) that
//!   sets `minimized` on the windows of the given pids, then is unloaded again.
//! * Hyprland: `hyprctl dispatch movetoworkspacesilent special:lodestar,pid:N` (Hyprland has
//!   no minimise; the special workspace is its scratchpad).
//! * Sway: `swaymsg [pid=N] move scratchpad`.
//! * GNOME (Mutter) offers nothing for this: Wayland-native windows stay where they are there.
//!
//! The D-Bus calls go through `busctl` (systemd), else `gdbus`, else `qdbus6` / `qdbus`: whichever
//! the desktop has, no library needed. Pids: real ones (Wayland clients) and pid-namespace ones
//! (XWayland windows of containerised Wine carry those in `_NET_WM_PID`).
//!
//! CONTRACT: `minimise(pids) -> Option<&'static str>` — the route used (`"kwin"`, `"hyprland"`,
//! `"sway"`), None when no compositor route applies or it failed. `activate(pids)` the same way
//! brings the first normal window of `pids` to the front and focuses it (KWin: un-minimise +
//! `workspace.activeWindow`; Hyprland `focuswindow pid:N`; Sway `[pid=N] focus`). Never blocks
//! for long (each tool call is a short-lived process).

use std::process::{Command, Stdio};

fn has(program: &str) -> bool {
    crate::launch::find_in_path(program).is_some()
}

fn quiet(cmd: &mut Command) -> Option<std::process::Output> {
    cmd.stdin(Stdio::null()).stderr(Stdio::null()).output().ok()
}

fn env_has(k: &str) -> bool {
    std::env::var_os(k).is_some_and(|v| !v.is_empty())
}

/// Which compositor runs this session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compositor {
    Kwin,
    Hyprland,
    Sway,
    Other,
}

pub fn detect() -> Compositor {
    if env_has("HYPRLAND_INSTANCE_SIGNATURE") {
        return Compositor::Hyprland;
    }
    if env_has("SWAYSOCK") {
        return Compositor::Sway;
    }
    let desk = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default().to_ascii_lowercase();
    if desk.split(':').any(|d| d == "kde") || std::env::var("KDE_FULL_SESSION").is_ok_and(|v| v == "true") {
        return Compositor::Kwin;
    }
    Compositor::Other
}

/// Minimise the windows of `pids` through the compositor (see the module docs).
pub fn minimise(pids: &[u32]) -> Option<&'static str> {
    if pids.is_empty() || !env_has("WAYLAND_DISPLAY") {
        return None;
    }
    match detect() {
        Compositor::Kwin => kwin(pids).then_some("kwin"),
        Compositor::Hyprland => hyprland(pids).then_some("hyprland"),
        Compositor::Sway => sway(pids).then_some("sway"),
        Compositor::Other => None,
    }
}

/// Bring the first normal window of `pids` to the front and focus it (see the module docs).
pub fn activate(pids: &[u32]) -> Option<&'static str> {
    if pids.is_empty() || !env_has("WAYLAND_DISPLAY") {
        return None;
    }
    match detect() {
        Compositor::Kwin => run_kwin_script("activate", &kwin_activate_script(pids)).then_some("kwin"),
        Compositor::Hyprland => has("hyprctl")
            .then(|| {
                pids.iter().any(|p| {
                    quiet(Command::new("hyprctl").args(hyprland_focus_args(*p)))
                        .is_some_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "ok")
                })
            })
            .filter(|ok| *ok)
            .map(|_| "hyprland"),
        Compositor::Sway => has("swaymsg")
            .then(|| {
                pids.iter()
                    .any(|p| quiet(Command::new("swaymsg").arg(sway_focus_arg(*p))).is_some_and(|o| o.status.success()))
            })
            .filter(|ok| *ok)
            .map(|_| "sway"),
        Compositor::Other => None,
    }
}

pub fn hyprland_focus_args(pid: u32) -> [String; 3] {
    ["dispatch".into(), "focuswindow".into(), format!("pid:{pid}")]
}

pub fn sway_focus_arg(pid: u32) -> String {
    format!("[pid={pid}] focus")
}

/// KWin script: un-minimise the first normal window of `pids` and make it the active one.
pub fn kwin_activate_script(pids: &[u32]) -> String {
    let list: Vec<String> = pids.iter().map(u32::to_string).collect();
    format!(
        "const pids = [{}];\n\
         const all = workspace.windowList ? workspace.windowList() : workspace.clientList();\n\
         for (const w of all) {{\n\
         \x20   if (pids.includes(w.pid) && w.normalWindow && !w.transient) {{\n\
         \x20       w.minimized = false;\n\
         \x20       if (workspace.windowList) {{\n\
         \x20           workspace.activeWindow = w;\n\
         \x20       }} else {{\n\
         \x20           workspace.activeClient = w;\n\
         \x20       }}\n\
         \x20       break;\n\
         \x20   }}\n\
         }}\n",
        list.join(", ")
    )
}

/// The KWin script: KWin 6 (`windowList`) and 5 (`clientList`); only normal, non-transient
/// windows (a WM minimises a window's dialogs with it; stray error dialogs stay visible).
pub fn kwin_script(pids: &[u32]) -> String {
    let list: Vec<String> = pids.iter().map(u32::to_string).collect();
    format!(
        "const pids = [{}];\n\
         const all = workspace.windowList ? workspace.windowList() : workspace.clientList();\n\
         for (const w of all) {{\n\
         \x20   if (pids.includes(w.pid) && w.normalWindow && !w.transient && !w.minimized) {{\n\
         \x20       w.minimized = true;\n\
         \x20   }}\n\
         }}\n",
        list.join(", ")
    )
}

/// One D-Bus method call on the session bus; returns its stdout. `args` are (D-Bus type, value).
fn dbus_call(dest: &str, path: &str, iface: &str, method: &str, args: &[(&str, &str)]) -> Option<String> {
    let out = if has("busctl") {
        let sig: String = args.iter().map(|(t, _)| *t).collect();
        let mut c = Command::new("busctl");
        c.args(["--user", "call", dest, path, iface, method]);
        if !sig.is_empty() {
            c.arg(sig);
            c.args(args.iter().map(|(_, v)| *v));
        }
        quiet(&mut c)?
    } else if has("gdbus") {
        let mut c = Command::new("gdbus");
        c.args(["call", "--session", "--dest", dest, "--object-path", path, "--method"]);
        c.arg(format!("{iface}.{method}"));
        for (t, v) in args {
            // gdbus takes GVariant text: strings quoted, numbers bare.
            if *t == "s" {
                c.arg(format!("'{}'", v.replace('\\', "\\\\").replace('\'', "\\'")));
            } else {
                c.arg(*v);
            }
        }
        quiet(&mut c)?
    } else {
        let q = ["qdbus6", "qdbus"].into_iter().find(|q| has(q))?;
        let mut c = Command::new(q);
        c.args([dest, path, &format!("{iface}.{method}")]);
        c.args(args.iter().map(|(_, v)| *v));
        quiet(&mut c)?
    };
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The script id in `loadScript`'s reply: busctl `i 3`, gdbus `(3,)`, qdbus `3`.
fn parse_id(reply: &str) -> Option<i64> {
    reply.split(|c: char| !(c.is_ascii_digit() || c == '-')).rfind(|w| !w.is_empty())?.parse().ok()
}

fn kwin(pids: &[u32]) -> bool {
    run_kwin_script("minimise", &kwin_script(pids))
}

/// Load `script` into KWin over D-Bus, run it, and unload it again a few seconds later. `tag`
/// names it (`lodestar-<tag>-<pid>-<n>`). False when KWin (or every D-Bus tool) is missing or
/// refused it. Blocks for the D-Bus calls (short-lived processes, tens of ms).
pub(crate) fn run_kwin_script(tag: &str, script: &str) -> bool {
    let dir = crate::config::Paths::detect().cache_dir.join("kwin");
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = format!("lodestar-{tag}-{}-{n}", std::process::id());
    let file = dir.join(format!("{name}.js"));
    if std::fs::write(&file, script).is_err() {
        return false;
    }
    let path = file.to_string_lossy().into_owned();
    let Some(reply) =
        dbus_call("org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting", "loadScript", &[("s", &path), ("s", &name)])
    else {
        return false;
    };
    let Some(id) = parse_id(&reply).filter(|id| *id >= 0) else {
        return false;
    };
    // KWin 6 names script objects /Scripting/Script<id>; KWin 5 /<id>.
    let ran = dbus_call("org.kde.KWin", &format!("/Scripting/Script{id}"), "org.kde.kwin.Script", "run", &[]).is_some()
        || dbus_call("org.kde.KWin", &format!("/{id}"), "org.kde.kwin.Script", "run", &[]).is_some();
    // KWin reads and runs the file asynchronously: unload it a little later.
    let _ = std::thread::Builder::new().name("kwin-unload".into()).spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(5));
        let _ = dbus_call("org.kde.KWin", "/Scripting", "org.kde.kwin.Scripting", "unloadScript", &[("s", &name)]);
        let _ = std::fs::remove_file(&file);
    });
    ran
}

fn hyprland(pids: &[u32]) -> bool {
    if !has("hyprctl") {
        return false;
    }
    let mut any = false;
    for p in pids {
        let arg = format!("special:lodestar,pid:{p}");
        any |= quiet(Command::new("hyprctl").args(["dispatch", "movetoworkspacesilent", &arg]))
            .is_some_and(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "ok");
    }
    any
}

fn sway(pids: &[u32]) -> bool {
    if !has("swaymsg") {
        return false;
    }
    let mut any = false;
    for p in pids {
        any |= quiet(Command::new("swaymsg").arg(format!("[pid={p}] move scratchpad")))
            .is_some_and(|o| o.status.success());
    }
    any
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_and_reply_parsing() {
        let s = kwin_script(&[12, 345]);
        assert!(s.contains("const pids = [12, 345];"));
        assert!(s.contains("w.minimized = true"));
        assert_eq!(parse_id("i 3\n"), Some(3));
        assert_eq!(parse_id("(7,)\n"), Some(7));
        assert_eq!(parse_id("12\n"), Some(12));
        assert_eq!(parse_id("i -1"), Some(-1));
        assert_eq!(parse_id(""), None);
    }

    #[test]
    fn activate_scripts_and_commands() {
        let s = kwin_activate_script(&[42, 7]);
        assert!(s.contains("const pids = [42, 7];"), "{s}");
        assert!(
            s.contains("w.minimized = false")
                && s.contains("workspace.activeWindow = w")
                && s.contains("workspace.activeClient = w")
        );
        assert!(s.contains("break;"), "only the first window");
        assert_eq!(s.matches('{').count(), s.matches('}').count(), "{s}");
        assert_eq!(hyprland_focus_args(42), ["dispatch", "focuswindow", "pid:42"]);
        assert_eq!(sway_focus_arg(42), "[pid=42] focus");
        assert_eq!(activate(&[]), None);
    }
}
