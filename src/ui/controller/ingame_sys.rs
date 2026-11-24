//! What the in-game menu and the status corner ask the system: the running game's processes
//! (to bring its window back), a screenshot tool (spectacle, grim, gnome-screenshot), the
//! volume (wpctl, else pactl) and the battery (/sys/class/power_supply). Commands and parsing
//! are pure functions (tested); running them happens on short-lived threads.

use crate::launch::winhide_wayland::Compositor;
use crate::model::GameId;
use crate::procfs;
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------ the game's processes

/// Pids (real and pid-namespace ones, for `_NET_WM_PID` of containerised games) of the processes
/// of game `id`: those whose environment carries Lodestar's launch tag for it, Steam's reaper for
/// `steam_appid`, and all their descendants.
pub fn game_pids(proc_root: &Path, id: GameId, steam_appid: Option<u32>) -> Vec<u32> {
    let procs = procfs::read_procs_in(proc_root, true);
    let tag_key = crate::launch::track::TAG_VAR.as_bytes();
    let mut roots: Vec<i32> = vec![];
    for p in &procs {
        if p.zombie {
            continue;
        }
        let steam =
            steam_appid.is_some() && p.cmdline.as_deref().and_then(crate::launch::track::reaper_appid) == steam_appid;
        let tagged = procfs::read_environ(proc_root, p.pid).is_some_and(|env| {
            procfs::env_entries(&env).any(|(k, v)| {
                k == tag_key
                    && crate::launch::track::parse_tag(&String::from_utf8_lossy(v)).is_some_and(|(_, g)| g == id)
            })
        });
        if steam || tagged {
            roots.push(p.pid);
        }
    }
    // Descendants (a launcher's child is the game; Steam's reaper runs it).
    let mut all: Vec<i32> = roots.clone();
    let mut i = 0;
    while i < all.len() {
        let parent = all[i];
        for p in &procs {
            if p.ppid == parent && !all.contains(&p.pid) && !p.zombie {
                all.push(p.pid);
            }
        }
        i += 1;
    }
    let mut out: Vec<u32> = vec![];
    for pid in all {
        for x in std::iter::once(pid).chain(nspids(proc_root, pid)) {
            if x > 1 && !out.contains(&(x as u32)) {
                out.push(x as u32);
            }
        }
    }
    out
}

/// A process' pids in nested pid namespaces (`NSpid:` beyond the first).
fn nspids(proc_root: &Path, pid: i32) -> Vec<i32> {
    std::fs::read_to_string(proc_root.join(pid.to_string()).join("status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("NSpid:"))
                .map(|l| l.split_whitespace().skip(2).filter_map(|x| x.parse().ok()).collect())
        })
        .unwrap_or_default()
}

// ------------------------------------------------------------------ screenshots

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShotTool {
    Spectacle,
    Grim,
    GnomeScreenshot,
}

impl ShotTool {
    fn program(self) -> &'static str {
        match self {
            ShotTool::Spectacle => "spectacle",
            ShotTool::Grim => "grim",
            ShotTool::GnomeScreenshot => "gnome-screenshot",
        }
    }
}

/// The screenshot tool that works in this session: spectacle on KDE, grim on wlroots
/// compositors (Hyprland, Sway), else gnome-screenshot / spectacle (X11 too). grim can't capture
/// KWin or GNOME Wayland sessions, so it's never picked there.
pub fn screenshot_tool(wayland: bool, compositor: Compositor, has: impl Fn(&str) -> bool) -> Option<ShotTool> {
    let order: &[ShotTool] = match (wayland, compositor) {
        (true, Compositor::Kwin) => &[ShotTool::Spectacle],
        (true, Compositor::Hyprland | Compositor::Sway) => &[ShotTool::Grim],
        (true, Compositor::Other) => &[ShotTool::GnomeScreenshot, ShotTool::Spectacle],
        (false, _) => &[ShotTool::Spectacle, ShotTool::GnomeScreenshot],
    };
    order.iter().copied().find(|t| has(t.program()))
}

/// The command saving a screenshot of the active window (the game, brought back first) /
/// the screen to `path`.
pub fn screenshot_argv(tool: ShotTool, path: &Path) -> Vec<String> {
    let p = path.to_string_lossy().into_owned();
    match tool {
        // -b background (no window), -n no notification (Lodestar sends one), -a active window.
        ShotTool::Spectacle => vec!["spectacle".into(), "-b".into(), "-n".into(), "-a".into(), "-o".into(), p],
        ShotTool::Grim => vec!["grim".into(), p],
        ShotTool::GnomeScreenshot => vec!["gnome-screenshot".into(), "-w".into(), "-f".into(), p],
    }
}

/// `<Pictures>/Screenshots/<game> <YYYY-MM-DD HH-MM-SS>.png` (characters a file name can't
/// hold dropped).
pub fn screenshot_path(pictures: &Path, game: &str, ts: i64) -> PathBuf {
    let name: String =
        game.chars()
            .map(|c| {
                if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
                    ' '
                } else {
                    c
                }
            })
            .collect();
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = if name.is_empty() { "Screenshot".to_string() } else { name };
    let stamp = crate::theme::render::bind::format_date(ts, "%Y-%m-%d %H-%M");
    let secs = ts.rem_euclid(60);
    pictures.join("Screenshots").join(format!("{name} {stamp}-{secs:02}.png"))
}

// ------------------------------------------------------------------ volume

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mixer {
    Wpctl,
    Pactl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VolOp {
    Up,
    Down,
    ToggleMute,
}

pub fn mixer(has: impl Fn(&str) -> bool) -> Option<Mixer> {
    if has("wpctl") {
        Some(Mixer::Wpctl)
    } else if has("pactl") {
        Some(Mixer::Pactl)
    } else {
        None
    }
}

const STEP: &str = "5%";

/// Change the default output's volume (capped at 100 %).
pub fn volume_argv(m: Mixer, op: VolOp) -> Vec<String> {
    let v: Vec<String> = match (m, op) {
        (Mixer::Wpctl, VolOp::Up) => {
            vec!["wpctl", "set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", &format!("{STEP}+")]
                .into_iter()
                .map(String::from)
                .collect()
        }
        (Mixer::Wpctl, VolOp::Down) => {
            ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{STEP}-")].map(String::from).to_vec()
        }
        (Mixer::Wpctl, VolOp::ToggleMute) => {
            ["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"].map(String::from).to_vec()
        }
        (Mixer::Pactl, VolOp::Up) => {
            ["pactl", "set-sink-volume", "@DEFAULT_SINK@", &format!("+{STEP}")].map(String::from).to_vec()
        }
        (Mixer::Pactl, VolOp::Down) => {
            ["pactl", "set-sink-volume", "@DEFAULT_SINK@", &format!("-{STEP}")].map(String::from).to_vec()
        }
        (Mixer::Pactl, VolOp::ToggleMute) => {
            ["pactl", "set-sink-mute", "@DEFAULT_SINK@", "toggle"].map(String::from).to_vec()
        }
    };
    v
}

/// pactl has no upper limit of its own: past 100 % after Up, set 100 %.
pub fn pactl_cap_argv() -> Vec<String> {
    ["pactl", "set-sink-volume", "@DEFAULT_SINK@", "100%"].map(String::from).to_vec()
}

/// `wpctl get-volume`: "Volume: 0.65" / "Volume: 0.40 [MUTED]" → (65, false) / (40, true).
pub fn parse_wpctl(out: &str) -> Option<(u32, bool)> {
    let rest = out.trim().strip_prefix("Volume:")?.trim();
    let num = rest.split_whitespace().next()?.parse::<f32>().ok()?;
    Some(((num * 100.0).round().max(0.0) as u32, rest.contains("[MUTED]")))
}

/// `pactl get-sink-volume`: the first "NN%" (front-left).
pub fn parse_pactl_volume(out: &str) -> Option<u32> {
    out.split_whitespace().find_map(|w| w.strip_suffix('%')?.parse().ok())
}

/// `pactl get-sink-mute`: "Mute: yes" / "Mute: no".
pub fn parse_pactl_mute(out: &str) -> Option<bool> {
    match out.trim().strip_prefix("Mute:")?.trim() {
        "yes" => Some(true),
        "no" => Some(false),
        _ => None,
    }
}

fn output(argv: &[String]) -> Option<String> {
    let o = std::process::Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    o.status.success().then(|| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// The default output's volume and mute state (blocks for the tool call; use on a thread).
pub fn read_volume(m: Mixer) -> Option<(u32, bool)> {
    match m {
        Mixer::Wpctl => parse_wpctl(&output(&["wpctl", "get-volume", "@DEFAULT_AUDIO_SINK@"].map(String::from))?),
        Mixer::Pactl => {
            let v = parse_pactl_volume(&output(&["pactl", "get-sink-volume", "@DEFAULT_SINK@"].map(String::from))?)?;
            let m = parse_pactl_mute(&output(&["pactl", "get-sink-mute", "@DEFAULT_SINK@"].map(String::from))?)?;
            Some((v, m))
        }
    }
}

/// Apply `op`, then read the result back (blocks; use on a thread).
pub fn change_volume(m: Mixer, op: VolOp) -> Option<(u32, bool)> {
    let _ = output(&volume_argv(m, op));
    let mut now = read_volume(m);
    if m == Mixer::Pactl && op == VolOp::Up && now.is_some_and(|(v, _)| v > 100) {
        let _ = output(&pactl_cap_argv());
        now = read_volume(m);
    }
    now
}

// ------------------------------------------------------------------ battery

/// The machine's battery as `battery` reads it, without waiting for it on the UI thread
/// (laptop firmware can take tens of milliseconds to answer): the first reading is taken
/// right away, later ones on a helper thread while the last reading stays on screen.
pub fn battery_now(sys_root: &'static str) -> Option<(u8, bool)> {
    use std::sync::atomic::{AtomicBool, Ordering};
    static LAST: std::sync::Mutex<Option<Option<(u8, bool)>>> = std::sync::Mutex::new(None);
    static BUSY: AtomicBool = AtomicBool::new(false);
    let last = *LAST.lock().ok()?;
    match last {
        None => {
            let v = battery(Path::new(sys_root));
            *LAST.lock().ok()? = Some(v);
            v
        }
        Some(v) => {
            if !BUSY.swap(true, Ordering::AcqRel) {
                let spawned = std::thread::Builder::new().name("battery".into()).spawn(move || {
                    let v = battery(Path::new(sys_root));
                    if let Ok(mut l) = LAST.lock() {
                        *l = Some(v);
                    }
                    BUSY.store(false, Ordering::Release);
                });
                if spawned.is_err() {
                    BUSY.store(false, Ordering::Release);
                }
            }
            v
        }
    }
}

/// The computer's battery: (percent, charging). Batteries of devices (controllers, mice:
/// `scope = Device`) don't count; several system batteries are averaged.
pub fn battery(sys_root: &Path) -> Option<(u8, bool)> {
    let rd = std::fs::read_dir(sys_root.join("class/power_supply")).ok()?;
    let mut levels = vec![];
    let mut charging = false;
    let mut on_ac = false;
    for e in rd.flatten() {
        let d = e.path();
        let read = |f: &str| std::fs::read_to_string(d.join(f)).ok().map(|s| s.trim().to_string());
        match read("type").as_deref() {
            Some("Battery") => {}
            Some("Mains") => {
                on_ac |= read("online").as_deref() == Some("1");
                continue;
            }
            _ => continue,
        }
        if read("scope").is_some_and(|s| s.eq_ignore_ascii_case("device")) {
            continue;
        }
        if let Some(c) = read("capacity").and_then(|c| c.parse::<u32>().ok()) {
            levels.push(c.min(100));
            charging |= read("status").as_deref() == Some("Charging");
        }
    }
    if levels.is_empty() {
        return None;
    }
    let avg = levels.iter().sum::<u32>() / levels.len() as u32;
    Some((avg as u8, charging || (on_ac && avg < 100)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_tool_per_session() {
        let all = |_: &str| true;
        assert_eq!(screenshot_tool(true, Compositor::Kwin, all), Some(ShotTool::Spectacle));
        assert_eq!(screenshot_tool(true, Compositor::Kwin, |t| t == "grim"), None, "grim can't capture KWin");
        assert_eq!(screenshot_tool(true, Compositor::Hyprland, all), Some(ShotTool::Grim));
        assert_eq!(screenshot_tool(true, Compositor::Sway, |t| t == "spectacle"), None);
        assert_eq!(screenshot_tool(true, Compositor::Other, all), Some(ShotTool::GnomeScreenshot));
        assert_eq!(
            screenshot_tool(false, Compositor::Other, |t| t == "gnome-screenshot"),
            Some(ShotTool::GnomeScreenshot)
        );
        assert_eq!(screenshot_tool(false, Compositor::Kwin, |_| false), None);
        let p = Path::new("/p/Shots/x.png");
        assert_eq!(screenshot_argv(ShotTool::Spectacle, p), ["spectacle", "-b", "-n", "-a", "-o", "/p/Shots/x.png"]);
        assert_eq!(screenshot_argv(ShotTool::Grim, p), ["grim", "/p/Shots/x.png"]);
        assert_eq!(screenshot_argv(ShotTool::GnomeScreenshot, p), ["gnome-screenshot", "-w", "-f", "/p/Shots/x.png"]);
    }

    #[test]
    fn screenshot_file_names() {
        let p = screenshot_path(Path::new("/home/u/Pictures"), "Half-Life: Alyx / VR", 1_700_000_007);
        let s = p.to_string_lossy();
        assert!(s.starts_with("/home/u/Pictures/Screenshots/Half-Life Alyx VR "), "{s}");
        assert!(s.ends_with("-27.png"), "{s}");
        let p = screenshot_path(Path::new("/x"), "///", 0);
        assert!(p.file_name().unwrap().to_string_lossy().starts_with("Screenshot "));
    }

    #[test]
    fn volume_commands_and_parsing() {
        assert_eq!(mixer(|_| true), Some(Mixer::Wpctl));
        assert_eq!(mixer(|t| t == "pactl"), Some(Mixer::Pactl));
        assert_eq!(mixer(|_| false), None);
        assert_eq!(
            volume_argv(Mixer::Wpctl, VolOp::Up),
            ["wpctl", "set-volume", "-l", "1.0", "@DEFAULT_AUDIO_SINK@", "5%+"]
        );
        assert_eq!(volume_argv(Mixer::Wpctl, VolOp::Down), ["wpctl", "set-volume", "@DEFAULT_AUDIO_SINK@", "5%-"]);
        assert_eq!(
            volume_argv(Mixer::Wpctl, VolOp::ToggleMute),
            ["wpctl", "set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"]
        );
        assert_eq!(volume_argv(Mixer::Pactl, VolOp::Up), ["pactl", "set-sink-volume", "@DEFAULT_SINK@", "+5%"]);
        assert_eq!(volume_argv(Mixer::Pactl, VolOp::Down), ["pactl", "set-sink-volume", "@DEFAULT_SINK@", "-5%"]);
        assert_eq!(
            volume_argv(Mixer::Pactl, VolOp::ToggleMute),
            ["pactl", "set-sink-mute", "@DEFAULT_SINK@", "toggle"]
        );
        assert_eq!(parse_wpctl("Volume: 0.65\n"), Some((65, false)));
        assert_eq!(parse_wpctl("Volume: 0.40 [MUTED]\n"), Some((40, true)));
        assert_eq!(parse_wpctl("nope"), None);
        assert_eq!(
            parse_pactl_volume(
                "Volume: front-left: 42597 /  65% / -11.23 dB,   front-right: 42597 /  65% / -11.23 dB\n"
            ),
            Some(65)
        );
        assert_eq!(parse_pactl_mute("Mute: yes\n"), Some(true));
        assert_eq!(parse_pactl_mute("Mute: no"), Some(false));
    }

    #[test]
    fn battery_ignores_device_batteries() {
        let dir = tempfile::tempdir().unwrap();
        let ps = dir.path().join("class/power_supply");
        let add = |name: &str, files: &[(&str, &str)]| {
            let d = ps.join(name);
            std::fs::create_dir_all(&d).unwrap();
            for (f, v) in files {
                std::fs::write(d.join(f), format!("{v}\n")).unwrap();
            }
        };
        assert_eq!(battery(dir.path()), None, "no power_supply folder");
        add(
            "ps-controller-battery-aa",
            &[("type", "Battery"), ("scope", "Device"), ("capacity", "40"), ("status", "Discharging")],
        );
        add("AC", &[("type", "Mains"), ("online", "0")]);
        assert_eq!(battery(dir.path()), None, "a controller's battery isn't the computer's");
        add("BAT0", &[("type", "Battery"), ("capacity", "83"), ("status", "Discharging")]);
        assert_eq!(battery(dir.path()), Some((83, false)));
        add("AC", &[("type", "Mains"), ("online", "1")]);
        assert_eq!(battery(dir.path()), Some((83, true)), "plugged in");
        add("BAT0", &[("type", "Battery"), ("capacity", "100"), ("status", "Full")]);
        assert_eq!(battery(dir.path()), Some((100, false)));
    }

    #[test]
    fn game_processes_by_tag_and_steam_reaper() {
        use crate::procfs::fake::{self, P};
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        const F: i32 = 4_200_000;
        let tag7 = format!("{}={}-7-1700000000000000000", crate::launch::track::TAG_VAR, F + 99);
        let tag8 = format!("{}={}-8-1700000000000000000", crate::launch::track::TAG_VAR, F + 99);
        fake::boot(root, 1_700_000_000);
        fake::add(root, P::new(F + 1, 10, &["/g/launcher"]).env(&[&tag7]));
        fake::add(root, P::new(F + 2, 11, &["/g/game.exe"]).parent(F + 1, F + 1, F + 1));
        fake::add(root, P::new(F + 3, 12, &["/other"]).env(&[&tag8]));
        fake::add(root, P::new(F + 4, 13, &["/s/reaper", "SteamLaunch", "AppId=570", "--", "/g/dota"]));
        fake::add(root, P::new(F + 5, 14, &["/g/dota2"]).parent(F + 4, F + 4, F + 4));
        std::fs::write(root.join((F + 2).to_string()).join("status"), "Name:\tgame\nNSpid:\t4200002\t12\n").unwrap();
        let mut got = game_pids(root, 7, None);
        got.sort();
        assert_eq!(got, vec![12, (F + 1) as u32, (F + 2) as u32]);
        let mut got = game_pids(root, 99, Some(570));
        got.sort();
        assert_eq!(got, vec![(F + 4) as u32, (F + 5) as u32]);
        assert!(game_pids(root, 99, None).is_empty());
    }
}
