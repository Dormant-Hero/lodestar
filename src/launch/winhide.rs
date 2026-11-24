//! Minimise (iconify) the X11 windows of given processes: how Lodestar keeps store clients out of
//! sight once the game they started runs.
//!
//! Wine clients (Battle.net, EA app, Ubisoft Connect …) run under XWayland even on a Wayland
//! desktop, and Wine's X11 driver sets `_NET_WM_PID` on every window to the owning process' pid.
//! So: list the managed top-level windows (`_NET_CLIENT_LIST`, else the root's children), keep
//! those whose `_NET_WM_PID` is one of the given pids, and ask the window manager to iconify each
//! one the ICCCM way (a `WM_CHANGE_STATE` ClientMessage with IconicState sent to the root window,
//! which every window manager honours). One short-lived connection per call, nothing polled.
//! Native Wayland windows (Heroic as a Wayland Electron app) aren't reachable this way; for them
//! this simply finds nothing.
//!
//! CONTRACT: `iconify_windows_of(display, pids) -> anyhow::Result<usize>` (windows asked to
//! iconify). `display` None = `$DISPLAY`; no display at all → Ok(0). Never panics without a WM.
//! `activate_window_of(display, pids) -> anyhow::Result<bool>`: ask the window manager to bring
//! the first normal window of `pids` to the front and focus it (`_NET_ACTIVE_WINDOW`, sent as
//! a pager would, which focus-stealing prevention lets through) — un-minimising it too.

use anyhow::Context as _;
use std::collections::HashSet;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, CLIENT_MESSAGE_EVENT, ClientMessageData, ClientMessageEvent, ConnectionExt as _, EventMask, MapState,
    Window,
};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_CLIENT_LIST,
        _NET_WM_PID,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_NORMAL,
        WM_STATE,
        WM_CHANGE_STATE,
        _NET_WM_NAME,
        UTF8_STRING,
        _NET_ACTIVE_WINDOW,
    }
}

/// ICCCM WM_STATE / WM_CHANGE_STATE values.
const ICONIC_STATE: u32 = 3;

/// Ask the window manager to iconify every normal top-level window owned by one of `pids` that
/// isn't iconified already. Transient windows (dialogs of a main window) and non-normal window
/// types (splash, tooltip, dock …) are left alone: a WM minimises a main window's transients
/// with it, and a stray error dialog of its own should stay visible.
pub fn iconify_windows_of(display: Option<&str>, pids: &[u32]) -> anyhow::Result<usize> {
    if pids.is_empty() {
        return Ok(0);
    }
    let env_display = std::env::var("DISPLAY").ok();
    let Some(name) = display.or(env_display.as_deref()).filter(|d| !d.trim().is_empty()) else {
        return Ok(0);
    };
    let (conn, screen) = x11rb::connect(Some(name)).with_context(|| format!("X display {name}"))?;
    let root = conn.setup().roots.get(screen).map(|s| s.root).context("no X screen")?;
    let atoms = Atoms::new(&conn)?.reply()?;
    let want: HashSet<u32> = pids.iter().copied().collect();
    let mut sent = 0;
    for w in candidate_windows(&conn, root, &atoms)? {
        if !wants_iconify(&conn, w, &atoms, &want)? {
            continue;
        }
        let ev = ClientMessageEvent {
            response_type: CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window: w,
            type_: atoms.WM_CHANGE_STATE,
            data: ClientMessageData::from([ICONIC_STATE, 0, 0, 0, 0]),
        };
        conn.send_event(false, root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, ev)?;
        sent += 1;
    }
    if sent > 0 {
        // A round trip so the requests are processed before the connection closes.
        conn.get_input_focus()?.reply()?;
    }
    Ok(sent)
}

/// `_NET_ACTIVE_WINDOW` source indication: a pager / taskbar (the user asked for it).
const SOURCE_PAGER: u32 = 2;

/// Ask the window manager to activate (raise, focus, un-minimise) the first normal top-level
/// window owned by one of `pids`. Ok(false): none found.
pub fn activate_window_of(display: Option<&str>, pids: &[u32]) -> anyhow::Result<bool> {
    if pids.is_empty() {
        return Ok(false);
    }
    let env_display = std::env::var("DISPLAY").ok();
    let Some(name) = display.or(env_display.as_deref()).filter(|d| !d.trim().is_empty()) else {
        return Ok(false);
    };
    let (conn, screen) = x11rb::connect(Some(name)).with_context(|| format!("X display {name}"))?;
    let root = conn.setup().roots.get(screen).map(|s| s.root).context("no X screen")?;
    let atoms = Atoms::new(&conn)?.reply()?;
    let want: HashSet<u32> = pids.iter().copied().collect();
    for w in candidate_windows(&conn, root, &atoms)? {
        if !is_main_window_of(&conn, w, &atoms, &want, true)? {
            continue;
        }
        let ev = ClientMessageEvent {
            response_type: CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window: w,
            type_: atoms._NET_ACTIVE_WINDOW,
            data: ClientMessageData::from([SOURCE_PAGER, 0, 0, 0, 0]),
        };
        conn.send_event(false, root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, ev)?;
        conn.get_input_focus()?.reply()?;
        return Ok(true);
    }
    Ok(false)
}

/// Diagnostics (`lodestar --list-windows`): every managed window as
/// `window pid state types title`.
pub fn list_windows(display: Option<&str>) -> anyhow::Result<Vec<String>> {
    let env_display = std::env::var("DISPLAY").ok();
    let name = display.or(env_display.as_deref()).filter(|d| !d.trim().is_empty()).context("no X display")?;
    let (conn, screen) = x11rb::connect(Some(name)).with_context(|| format!("X display {name}"))?;
    let root = conn.setup().roots.get(screen).map(|s| s.root).context("no X screen")?;
    let atoms = Atoms::new(&conn)?.reply()?;
    let mut out = vec![];
    for w in candidate_windows(&conn, root, &atoms)? {
        let prop32 = |prop: u32, ty: u32| -> Option<Vec<u32>> {
            conn.get_property(false, w, prop, ty, 0, 16)
                .ok()?
                .reply()
                .ok()
                .and_then(|r| r.value32().map(|v| v.collect()))
        };
        let pid = prop32(atoms._NET_WM_PID, AtomEnum::CARDINAL.into()).and_then(|v| v.first().copied());
        let state = prop32(atoms.WM_STATE, atoms.WM_STATE).and_then(|v| v.first().copied());
        let transient = prop32(AtomEnum::WM_TRANSIENT_FOR.into(), AtomEnum::WINDOW.into())
            .is_some_and(|v| v.iter().any(|&t| t != 0));
        let types = prop32(atoms._NET_WM_WINDOW_TYPE, AtomEnum::ATOM.into()).unwrap_or_default();
        let type_names: Vec<String> = types
            .iter()
            .filter_map(|&a| {
                conn.get_atom_name(a).ok()?.reply().ok().map(|r| String::from_utf8_lossy(&r.name).into_owned())
            })
            .collect();
        let title = conn
            .get_property(false, w, atoms._NET_WM_NAME, atoms.UTF8_STRING, 0, 64)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| String::from_utf8_lossy(&r.value).into_owned())
            .unwrap_or_default();
        out.push(format!(
            "0x{w:08x} pid={} state={} transient={transient} types={} {title:?}",
            pid.map(|p| p.to_string()).unwrap_or("-".into()),
            state.map(|p| p.to_string()).unwrap_or("-".into()),
            type_names.join(",")
        ));
    }
    Ok(out)
}

/// Managed top-level windows: `_NET_CLIENT_LIST` when the WM keeps one, else the root's mapped
/// children and their children (frames of a reparenting WM hold the client window inside).
fn candidate_windows(conn: &impl Connection, root: Window, atoms: &Atoms) -> anyhow::Result<Vec<Window>> {
    let list = conn.get_property(false, root, atoms._NET_CLIENT_LIST, AtomEnum::WINDOW, 0, 1 << 16)?.reply()?;
    if let Some(v) = list.value32() {
        let v: Vec<Window> = v.collect();
        if !v.is_empty() {
            return Ok(v);
        }
    }
    let mut out = vec![];
    for child in conn.query_tree(root)?.reply()?.children {
        let viewable = conn.get_window_attributes(child)?.reply().is_ok_and(|a| a.map_state == MapState::VIEWABLE);
        if !viewable {
            continue;
        }
        out.push(child);
        if let Ok(t) = conn.query_tree(child)?.reply() {
            out.extend(t.children);
        }
    }
    Ok(out)
}

fn wants_iconify(conn: &impl Connection, w: Window, atoms: &Atoms, pids: &HashSet<u32>) -> anyhow::Result<bool> {
    is_main_window_of(conn, w, atoms, pids, false)
}

/// `w` is a normal, non-transient window of one of `pids` (`iconic_ok`: also when minimised).
fn is_main_window_of(
    conn: &impl Connection,
    w: Window,
    atoms: &Atoms,
    pids: &HashSet<u32>,
    iconic_ok: bool,
) -> anyhow::Result<bool> {
    // A window may vanish between listing and asking: that's a "no", not an error.
    let prop32 = |prop: u32, ty: u32| -> anyhow::Result<Option<Vec<u32>>> {
        Ok(conn.get_property(false, w, prop, ty, 0, 16)?.reply().ok().and_then(|r| r.value32().map(|v| v.collect())))
    };
    let pid = prop32(atoms._NET_WM_PID, AtomEnum::CARDINAL.into())?.and_then(|v| v.first().copied());
    if !pid.is_some_and(|p| pids.contains(&p)) {
        return Ok(false);
    }
    if !iconic_ok && prop32(atoms.WM_STATE, atoms.WM_STATE)?.and_then(|v| v.first().copied()) == Some(ICONIC_STATE) {
        return Ok(false);
    }
    if prop32(AtomEnum::WM_TRANSIENT_FOR.into(), AtomEnum::WINDOW.into())?.is_some_and(|v| v.iter().any(|&t| t != 0)) {
        return Ok(false);
    }
    let types = prop32(atoms._NET_WM_WINDOW_TYPE, AtomEnum::ATOM.into())?.unwrap_or_default();
    Ok(types.is_empty() || types.contains(&atoms._NET_WM_WINDOW_TYPE_NORMAL))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::fd::FromRawFd;
    use std::time::{Duration, Instant};
    use x11rb::protocol::Event;
    use x11rb::protocol::xproto::{ChangeWindowAttributesAux, CreateWindowAux, PropMode, WindowClass};
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    /// A private Xvfb (no window manager) for one test; killed on drop.
    struct Xvfb {
        child: std::process::Child,
        display: String,
    }

    impl Drop for Xvfb {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn xvfb() -> Option<Xvfb> {
        crate::launch::find_in_path("Xvfb")?;
        let mut fds = [0; 2];
        // SAFETY: plain pipe(2); both ends are owned below.
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return None;
        }
        let child = std::process::Command::new("Xvfb")
            .args(["-displayfd", &fds[1].to_string(), "-nolisten", "tcp", "-screen", "0", "320x240x24"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok();
        // SAFETY: fds come from pipe(2) above and are closed / owned exactly once.
        unsafe { libc::close(fds[1]) };
        let mut r = unsafe { std::fs::File::from_raw_fd(fds[0]) };
        let child = child?;
        let mut s = String::new();
        let mut buf = [0u8; 16];
        while !s.contains('\n') {
            match r.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => s.push_str(&String::from_utf8_lossy(&buf[..n])),
            }
        }
        let n = s.trim();
        if n.is_empty() {
            let mut c = child;
            let _ = c.kill();
            let _ = c.wait();
            return None;
        }
        Some(Xvfb { child, display: format!(":{n}") })
    }

    fn window_with_pid(conn: &RustConnection, root: Window, pid: u32, atoms: &Atoms) -> Window {
        let w = conn.generate_id().unwrap();
        conn.create_window(0, w, root, 0, 0, 50, 50, 0, WindowClass::INPUT_OUTPUT, 0, &CreateWindowAux::new()).unwrap();
        conn.change_property32(PropMode::REPLACE, w, atoms._NET_WM_PID, AtomEnum::CARDINAL, &[pid]).unwrap();
        conn.map_window(w).unwrap();
        conn.flush().unwrap();
        w
    }

    /// Iconify requests (window ids) the observer saw on the root window within `wait`.
    fn iconify_requests(obs: &RustConnection, atoms: &Atoms, wait: Duration) -> Vec<Window> {
        let t = Instant::now();
        let mut out = vec![];
        while t.elapsed() < wait {
            match obs.poll_for_event().unwrap() {
                Some(Event::ClientMessage(e))
                    if e.type_ == atoms.WM_CHANGE_STATE && e.data.as_data32()[0] == ICONIC_STATE =>
                {
                    out.push(e.window)
                }
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        out
    }

    #[test]
    fn iconifies_windows_of_the_given_pids_under_xvfb() {
        let Some(x) = xvfb() else {
            eprintln!("Xvfb not available; skipping");
            return;
        };
        let (conn, screen) = x11rb::connect(Some(&x.display)).unwrap();
        let root = conn.setup().roots[screen].root;
        let atoms = Atoms::new(&conn).unwrap().reply().unwrap();
        // The observer plays the window manager's part: it listens on the root window.
        let (obs, _) = x11rb::connect(Some(&x.display)).unwrap();
        obs.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
        )
        .unwrap();
        obs.flush().unwrap();
        let client = window_with_pid(&conn, root, 4_000_001, &atoms);
        let game = window_with_pid(&conn, root, 4_000_002, &atoms);
        let dialog = window_with_pid(&conn, root, 4_000_001, &atoms);
        conn.change_property32(PropMode::REPLACE, dialog, AtomEnum::WM_TRANSIENT_FOR, AtomEnum::WINDOW, &[client])
            .unwrap();
        conn.get_input_focus().unwrap().reply().unwrap();

        // No WM, no _NET_CLIENT_LIST: the root's mapped children are looked at.
        assert_eq!(iconify_windows_of(Some(&x.display), &[4_000_001]).unwrap(), 1);
        assert_eq!(iconify_requests(&obs, &atoms, Duration::from_millis(400)), vec![client]);
        // Nobody acts on the request (no WM): nothing breaks, the window just stays.
        assert_eq!(iconify_windows_of(Some(&x.display), &[]).unwrap(), 0);
        assert_eq!(iconify_windows_of(Some(&x.display), &[12345]).unwrap(), 0);

        // With a WM's client list, only listed windows count; iconic ones are skipped.
        conn.change_property32(
            PropMode::REPLACE,
            root,
            atoms._NET_CLIENT_LIST,
            AtomEnum::WINDOW,
            &[game, client, dialog],
        )
        .unwrap();
        conn.get_input_focus().unwrap().reply().unwrap();
        assert_eq!(iconify_windows_of(Some(&x.display), &[4_000_001, 4_000_002]).unwrap(), 2);
        let mut got = iconify_requests(&obs, &atoms, Duration::from_millis(400));
        got.sort();
        let mut want = vec![client, game];
        want.sort();
        assert_eq!(got, want);
        conn.change_property32(PropMode::REPLACE, client, atoms.WM_STATE, atoms.WM_STATE, &[ICONIC_STATE, 0]).unwrap();
        conn.get_input_focus().unwrap().reply().unwrap();
        assert_eq!(iconify_windows_of(Some(&x.display), &[4_000_001]).unwrap(), 0, "already iconic");
    }

    #[test]
    fn activates_the_main_window_of_the_given_pids_under_xvfb() {
        let Some(x) = xvfb() else {
            eprintln!("Xvfb not available; skipping");
            return;
        };
        let (conn, screen) = x11rb::connect(Some(&x.display)).unwrap();
        let root = conn.setup().roots[screen].root;
        let atoms = Atoms::new(&conn).unwrap().reply().unwrap();
        let (obs, _) = x11rb::connect(Some(&x.display)).unwrap();
        obs.change_window_attributes(
            root,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_NOTIFY),
        )
        .unwrap();
        obs.flush().unwrap();
        let other = window_with_pid(&conn, root, 4_000_011, &atoms);
        let game = window_with_pid(&conn, root, 4_000_012, &atoms);
        // Minimised games are brought back too.
        conn.change_property32(PropMode::REPLACE, game, atoms.WM_STATE, atoms.WM_STATE, &[ICONIC_STATE, 0]).unwrap();
        conn.change_property32(PropMode::REPLACE, root, atoms._NET_CLIENT_LIST, AtomEnum::WINDOW, &[other, game])
            .unwrap();
        conn.get_input_focus().unwrap().reply().unwrap();
        assert!(activate_window_of(Some(&x.display), &[4_000_012]).unwrap());
        let t = Instant::now();
        let mut got = vec![];
        while t.elapsed() < Duration::from_millis(400) {
            match obs.poll_for_event().unwrap() {
                Some(Event::ClientMessage(e)) if e.type_ == atoms._NET_ACTIVE_WINDOW => {
                    got.push((e.window, e.data.as_data32()[0]))
                }
                Some(_) => {}
                None => std::thread::sleep(Duration::from_millis(20)),
            }
        }
        assert_eq!(got, vec![(game, SOURCE_PAGER)]);
        assert!(!activate_window_of(Some(&x.display), &[12345]).unwrap());
        assert!(!activate_window_of(Some(&x.display), &[]).unwrap());
    }

    #[test]
    fn no_display_is_a_no_op() {
        assert_eq!(iconify_windows_of(Some(""), &[1]).unwrap(), 0);
        // A display that isn't there: an error to log, never a panic.
        assert!(iconify_windows_of(Some(":597"), &[1]).is_err());
    }
}
