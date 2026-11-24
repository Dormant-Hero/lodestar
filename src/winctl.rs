//! Hiding Lodestar's own window to the system tray and bringing it back.
//!
//! "Hidden" means gone from the taskbar (and Alt+Tab), not just minimised. How depends on the
//! session, chosen once by [`platform`]:
//! * X11 (also Lodestar under XWayland): winit unmaps the window (`ViewportCommand::Visible`);
//!   showing maps it again and asks the window manager to activate it.
//! * KDE Plasma on Wayland: Wayland lets no program hide its own window from the taskbar, so a
//!   tiny KWin script (loaded over D-Bus like the one launcher hygiene uses, winhide_wayland.rs)
//!   sets `skipTaskbar` / `skipSwitcher` / `minimized` on the windows of Lodestar's pid; showing
//!   clears them and makes the window the active one.
//! * Other Wayland compositors: minimise (the window stays in the taskbar); showing can only ask
//!   for attention (winit can't un-minimise or focus a window there).
//!
//! eframe keeps running the app's `logic` while the window is hidden or minimised (a few times a
//! second), so the UI thread still sees tray and second-instance requests.
//!
//! Showing also raises and focuses: X11 additionally sends `_NET_ACTIVE_WINDOW` for our own
//! window as a pager would (window managers' focus-stealing prevention ignores winit's plain
//! focus request from a background program, e.g. when the Guide button summons Lodestar);
//! Hyprland / Sway are asked to focus our pid; other Wayland compositors only get a request
//! for attention.
//!
//! `activate_pids(ctx, p, pids, on_fail)` brings another program's window (the running game,
//! the in-game menu's Resume) to the front the same ways; `on_fail` runs when no route worked.
//!
//! CONTRACT: `platform(wayland, compositor) -> Platform` (pure); `hide(ctx, p)` / `show(ctx, p)`
//! / `activate_pids` never block the caller (KWin / X11 / tool calls run on a short-lived
//! thread).

use crate::launch::winhide_wayland::{self, Compositor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    /// X11 or XWayland: unmap / map.
    X11,
    /// KDE Plasma Wayland: a KWin script.
    Kwin,
    /// Another Wayland compositor: minimise only.
    Wayland,
}

/// How this session hides the window (`wayland`: winit runs on Wayland).
pub fn platform(wayland: bool, compositor: Compositor) -> Platform {
    match (wayland, compositor) {
        (false, _) => Platform::X11,
        (true, Compositor::Kwin) => Platform::Kwin,
        (true, _) => Platform::Wayland,
    }
}

impl Platform {
    /// This session's platform.
    pub fn detect() -> Platform {
        platform(crate::app::is_wayland(|k| std::env::var_os(k)), winhide_wayland::detect())
    }

    /// Hiding takes the window out of the taskbar (else it only minimises).
    pub fn hides_from_taskbar(self) -> bool {
        self != Platform::Wayland
    }
}

/// The KWin script that hides (`hide`) or shows the normal windows of `pid`. KWin 6
/// (`windowList`, `activeWindow`) and 5 (`clientList`, `activeClient`).
pub fn kwin_script(pid: u32, hide: bool) -> String {
    let body = if hide {
        "w.skipTaskbar = true;\n        w.skipSwitcher = true;\n        w.minimized = true;\n"
    } else {
        "w.minimized = false;\n        w.skipTaskbar = false;\n        w.skipSwitcher = false;\n        \
         if (workspace.windowList) {\n            workspace.activeWindow = w;\n        } else {\n            \
         workspace.activeClient = w;\n        }\n"
    };
    format!(
        "const all = workspace.windowList ? workspace.windowList() : workspace.clientList();\n\
         for (const w of all) {{\n\
         \x20   if (w.pid === {pid} && w.normalWindow) {{\n\
         \x20       {body}\
         \x20   }}\n\
         }}\n"
    )
}

/// Hide the window (see the module docs).
pub fn hide(ctx: &egui::Context, p: Platform) {
    match p {
        Platform::X11 => ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false)),
        Platform::Kwin => {
            // Minimised at once by the compositor; out of the taskbar once the script ran (if it
            // can't run, the window just stays minimised).
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            kwin_async(false, ctx.clone());
        }
        Platform::Wayland => ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true)),
    }
}

/// Show the window again and bring it to the front (see the module docs).
pub fn show(ctx: &egui::Context, p: Platform) {
    match p {
        Platform::X11 => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            // Once mapped: ask the window manager as a pager would (beats focus-stealing
            // prevention for a program in the background).
            let me = std::process::id();
            spawn("x11-activate", move || {
                std::thread::sleep(std::time::Duration::from_millis(120));
                let _ = crate::launch::winhide::activate_window_of(None, &[me]);
            });
        }
        Platform::Kwin => kwin_async(true, ctx.clone()),
        Platform::Wayland => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            let me = std::process::id();
            let c = ctx.clone();
            spawn("wl-activate", move || {
                if winhide_wayland::activate(&[me]).is_none() {
                    attention(&c);
                }
            });
        }
    }
    ctx.request_repaint();
}

/// Bring the first normal window of `pids` to the front (X11 / XWayland first, then the
/// compositor); `on_fail` runs (on that thread) when neither found one.
pub fn activate_pids(p: Platform, pids: Vec<u32>, on_fail: impl FnOnce() + Send + 'static) {
    spawn("activate-game", move || {
        let x11 = std::panic::catch_unwind(|| crate::launch::winhide::activate_window_of(None, &pids))
            .ok()
            .and_then(Result::ok)
            .unwrap_or(false);
        let ok = x11 || (p != Platform::X11 && winhide_wayland::activate(&pids).is_some());
        log::info!("activating the windows of {pids:?}: {}", if ok { "done" } else { "none found" });
        if !ok {
            on_fail();
        }
    });
}

fn spawn(name: &str, f: impl FnOnce() + Send + 'static) {
    if std::thread::Builder::new().name(name.into()).spawn(f).is_err() {
        log::warn!("couldn't start a thread for {name}");
    }
}

fn attention(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
}

/// Run the KWin script on a thread of its own; showing falls back to asking for attention.
fn kwin_async(show: bool, ctx: egui::Context) {
    let script = kwin_script(std::process::id(), !show);
    let spawned = std::thread::Builder::new().name("kwin-window".into()).spawn(move || {
        let ok = winhide_wayland::run_kwin_script(if show { "show" } else { "hide" }, &script);
        if !ok {
            log::warn!("KWin script to {} the window failed", if show { "show" } else { "hide" });
            if show {
                attention(&ctx);
            }
        }
        ctx.request_repaint();
    });
    if spawned.is_err() {
        log::warn!("couldn't start a thread for the KWin script");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_choice() {
        for c in [Compositor::Kwin, Compositor::Hyprland, Compositor::Sway, Compositor::Other] {
            assert_eq!(platform(false, c), Platform::X11, "X11 / XWayland always unmaps");
        }
        assert_eq!(platform(true, Compositor::Kwin), Platform::Kwin);
        assert_eq!(platform(true, Compositor::Hyprland), Platform::Wayland);
        assert_eq!(platform(true, Compositor::Sway), Platform::Wayland);
        assert_eq!(platform(true, Compositor::Other), Platform::Wayland);
        assert!(Platform::X11.hides_from_taskbar() && Platform::Kwin.hides_from_taskbar());
        assert!(!Platform::Wayland.hides_from_taskbar());
    }

    #[test]
    fn kwin_scripts() {
        let h = kwin_script(4321, true);
        assert!(h.contains("w.pid === 4321 && w.normalWindow"), "{h}");
        assert!(
            h.contains("w.skipTaskbar = true")
                && h.contains("w.skipSwitcher = true")
                && h.contains("w.minimized = true")
        );
        assert!(!h.contains("activeWindow"));
        let s = kwin_script(4321, false);
        assert!(s.contains("w.skipTaskbar = false") && s.contains("w.minimized = false"));
        assert!(s.contains("workspace.activeWindow = w") && s.contains("workspace.activeClient = w"));
        // Balanced braces: a syntax error would make KWin refuse the whole script.
        for t in [&h, &s] {
            assert_eq!(t.matches('{').count(), t.matches('}').count(), "{t}");
        }
    }
}
