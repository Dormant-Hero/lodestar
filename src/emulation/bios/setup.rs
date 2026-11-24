//! "Set up everything" for one console: what would go where (one list the user confirms once),
//! doing it, and the one plain sentence Settings → Emulation shows per console ("Ready in
//! PCSX2 ✓", "PS2 BIOS found in ~/Documents/bios files — not in PCSX2 yet").
//!
//! The plan only adds what is missing: found BIOS files / keys an emulator doesn't have (never
//! replacing a different file it has), Switch firmware where an emulator has none, PS3 firmware
//! into one RPCS3 when none has any (the console's own emulator first). Replacing firmware stays
//! a separate, deliberate button. Emulators that aren't allowed to run yet are left out.

use super::*;

/// One thing "Set up everything" does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Copy found BIOS files / keys into an emulator's folder: (source, destination).
    Copy { emulator: String, dir: PathBuf, files: Vec<(PathBuf, PathBuf)> },
    /// Copy Switch firmware into an emulator's NAND.
    SwitchFirmware { emulator: String, dir: PathBuf, from: PathBuf, label: String, files: usize },
    /// RPCS3 installs PS3 firmware itself (its own window).
    Ps3Firmware { emulator: String, pup: PathBuf, label: String },
}

impl Step {
    pub fn emulator(&self) -> &str {
        match self {
            Step::Copy { emulator, .. }
            | Step::SwitchFirmware { emulator, .. }
            | Step::Ps3Firmware { emulator, .. } => emulator,
        }
    }

    /// The folders (or zips) its files come from, for "found in …" (worked out from the paths
    /// alone: the sentence is drawn every frame).
    pub fn sources(&self) -> Vec<PathBuf> {
        match self {
            Step::Copy { files, .. } => files.iter().map(|(s, _)| folder_of(s)).collect(),
            Step::SwitchFirmware { from, .. } => vec![from.clone()],
            Step::Ps3Firmware { pup, .. } => vec![folder_of(pup)],
        }
    }
}

/// Everything "Set up everything" would do for this console. `prefer`: the emulator the console
/// uses (its PS3 firmware goes there first).
pub fn setup_plan(reg: &Registry, status: &BiosStatus, prefer: Option<&str>) -> Vec<Step> {
    let allowed = |id: &str| reg.emulator(id).is_some_and(|e| !e.blocked);
    let mut out = vec![];
    for t in status.targets.iter().filter(|t| allowed(&t.emulator)) {
        let files: Vec<(PathBuf, PathBuf)> = t
            .missing
            .iter()
            .filter_map(|src| dest_for(reg, status, &t.emulator, src).map(|d| (src.clone(), d)))
            .filter(|(_, d)| !d.exists())
            .collect();
        if !files.is_empty() {
            out.push(Step::Copy { emulator: t.emulator.clone(), dir: t.dir.clone(), files });
        }
    }
    for k in &status.packages {
        match k.kind {
            PackageKind::SwitchFirmware => {
                for t in k.targets.iter().filter(|t| t.installed.is_none() && allowed(&t.emulator)) {
                    out.push(Step::SwitchFirmware {
                        emulator: t.emulator.clone(),
                        dir: t.dir.clone(),
                        from: k.path.clone(),
                        label: k.label(),
                        files: k.files,
                    });
                }
            }
            PackageKind::Ps3Firmware => {
                let usable: Vec<&PackageTarget> = k.targets.iter().filter(|t| allowed(&t.emulator)).collect();
                if usable.iter().any(|t| t.installed.is_some()) {
                    continue;
                }
                let pick = usable.iter().find(|t| Some(t.emulator.as_str()) == prefer).or(usable.first());
                if let Some(t) = pick {
                    out.push(Step::Ps3Firmware { emulator: t.emulator.clone(), pup: k.path.clone(), label: k.label() });
                }
            }
        }
    }
    out
}

/// Do the plan's copies and Switch firmware installs (a helper thread; the user confirmed).
/// PS3 firmware steps are skipped: RPCS3 opens its own window, the caller starts it. Returns
/// how many files were written. Stops at the first error.
pub fn run_setup(status: &BiosStatus, steps: &[Step]) -> anyhow::Result<usize> {
    let mut n = 0;
    for s in steps {
        match s {
            Step::Copy { files, .. } => {
                for (src, dest) in files {
                    if dest.exists() {
                        continue;
                    }
                    copy_source(src, dest)?;
                    n += 1;
                }
            }
            Step::SwitchFirmware { emulator, .. } => n += install_firmware(status, emulator, false)?,
            Step::Ps3Firmware { .. } => {}
        }
    }
    Ok(n)
}

/// How a console's line looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mood {
    /// Everything is in place.
    Ready,
    /// Found, one click away.
    Todo,
    /// Needed and not found (or a bad dump).
    Missing,
    /// Optional / nothing to do.
    Info,
}

/// The one plain sentence per console, a short help text, and the main button.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub text: String,
    pub help: Option<String>,
    pub mood: Mood,
    /// The main button's label (None: nothing to do). With a plan it runs the plan; without
    /// one it lets the user choose files.
    pub button: Option<String>,
    /// The button only adds extras (the console is ready already): shown as a plain button.
    pub extra: bool,
}

/// "PlayStation 2 BIOS", "Switch keys and firmware", "PS3 firmware".
pub fn what_of(platform: &str) -> String {
    match platform {
        "nintendo_switch" => "Switch keys and firmware".into(),
        "sony_playstation3" => "PS3 firmware".into(),
        "sony_vita" => "Vita firmware".into(),
        "nintendo_wiiu" => "Wii U keys".into(),
        p => match super::super::platforms::by_id(p) {
            Some(pl) => format!("{} BIOS", super::super::console_label(pl)),
            None => "BIOS".into(),
        },
    }
}

/// "a and b", "a, b and c".
pub fn join_and(v: &[String]) -> String {
    match v {
        [] => String::new(),
        [a] => a.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The folder (or zip) a found file is in (from the path alone, nothing is looked at).
fn folder_of(p: &Path) -> PathBuf {
    match zipread::split_by_name(p) {
        Some((zip, _)) => zip,
        None => p.parent().unwrap_or(p).to_path_buf(),
    }
}

/// The places files come from (folders or zips, see [`Step::sources`]): "~/Documents/bios
/// files" / "… and 2 other folders".
fn places(folders: &[PathBuf], home: &Path) -> String {
    let mut v: Vec<String> = vec![];
    for f in folders {
        let p = describe(f, home);
        if !v.contains(&p) {
            v.push(p);
        }
    }
    match v.len() {
        0 => String::new(),
        1 => v.remove(0),
        2 => format!("{} and {}", v[0], v[1]),
        n => format!("{} and {} other folders", v[0], n - 1),
    }
}

/// The main button for a plan: the one action's own words, or "Set up everything".
pub fn plan_button(platform: &str, steps: &[Step], name: &dyn Fn(&str) -> String) -> Option<String> {
    match steps {
        [] => None,
        [Step::Copy { emulator, .. }] if platform == "nintendo_switch" => {
            Some(format!("Copy keys to {}", name(emulator)))
        }
        [Step::Copy { emulator, .. }] => Some(format!("Copy to {}", name(emulator))),
        [Step::SwitchFirmware { emulator, .. }] => Some(format!("Install firmware into {}", name(emulator))),
        [Step::Ps3Firmware { emulator, .. }] => Some(format!("Install into {}", name(emulator))),
        _ => Some("Set up everything".into()),
    }
}

/// One line of the confirmation: what goes where.
pub fn step_line(s: &Step, name: &dyn Fn(&str) -> String, home: &Path) -> String {
    let short = |p: &Path| describe(p, home);
    match s {
        Step::Copy { emulator, dir, files } => {
            let names: Vec<String> = files
                .iter()
                .map(|(_, d)| d.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
                .collect();
            let list = if names.len() <= 4 { join_and(&names) } else { format!("{} files", names.len()) };
            format!("Copy {list} to {} ({})", name(emulator), short(dir))
        }
        Step::SwitchFirmware { emulator, dir, label, files, .. } => {
            format!("Install {label} into {} ({files} files, {})", name(emulator), short(dir))
        }
        Step::Ps3Firmware { emulator, label, .. } => {
            format!("{} installs {label} (its own window opens for a minute)", name(emulator))
        }
    }
}

/// The sentence for a console. `name`: an emulator id → its name in the UI; emulators that aren't
/// allowed to run yet are left out by the caller's `allowed`.
pub fn summary(
    status: &BiosStatus,
    steps: &[Step],
    home: &Path,
    name: &dyn Fn(&str) -> String,
    allowed: &dyn Fn(&str) -> bool,
) -> Summary {
    let pf = status.platform.as_str();
    let targets: Vec<&BiosTarget> = status.targets.iter().filter(|t| allowed(&t.emulator)).collect();
    let ready: Vec<String> = targets.iter().filter(|t| t.ready).map(|t| name(&t.emulator)).collect();
    if !steps.is_empty() {
        return summary_with_steps(pf, steps, &targets, &ready, home, name);
    }
    if !ready.is_empty() {
        return Summary {
            text: ready_text(pf, &targets, &ready),
            help: None,
            mood: Mood::Ready,
            button: None,
            extra: false,
        };
    }
    if pf == "nintendo_switch" && !targets.is_empty() {
        return switch_summary(status, &targets, name);
    }
    state_summary(status, home)
}

/// "Ready in …", "Firmware 4.92 installed in …": the emulators that have what they need.
fn ready_text(pf: &str, targets: &[&BiosTarget], ready: &[String]) -> String {
    match pf {
        "sony_playstation3" => {
            let t = targets.iter().find(|t| t.ready);
            let v = t.and_then(|t| t.firmware.clone()).filter(|v| v != "installed");
            match v {
                Some(v) => format!("Firmware {} installed in {} ✓", v.trim_start_matches('0'), join_and(ready)),
                None => format!("Firmware installed in {} ✓", join_and(ready)),
            }
        }
        "nintendo_switch" => format!("Keys and firmware are in {} ✓", join_and(ready)),
        _ => format!("Ready in {} ✓", join_and(ready)),
    }
}

/// Something found can be put in place: "found in … — not in … yet", or, when an emulator is
/// ready already, "Ready in …" with the rest as extra help.
fn summary_with_steps(
    pf: &str,
    steps: &[Step],
    targets: &[&BiosTarget],
    ready: &[String],
    home: &Path,
    name: &dyn Fn(&str) -> String,
) -> Summary {
    let what = what_of(pf);
    let mut todo: Vec<String> = vec![];
    for s in steps {
        let n = name(s.emulator());
        if !todo.contains(&n) {
            todo.push(n);
        }
    }
    let srcs: Vec<PathBuf> = steps.iter().flat_map(Step::sources).collect();
    let button = plan_button(pf, steps, name);
    let place = places(&srcs, home);
    if ready.is_empty() {
        let found = if place.is_empty() { format!("{what} found") } else { format!("{what} found in {place}") };
        return Summary {
            text: format!("{found} — not in {} yet.", join_and(&todo)),
            help: Some(match pf {
                "sony_playstation3" => "RPCS3 installs it itself; it opens its own window for a minute.".into(),
                _ if steps.len() > 1 => "One click puts everything where each emulator looks for it.".into(),
                _ => "One click copies it where the emulator looks for it.".into(),
            }),
            mood: Mood::Todo,
            button,
            extra: false,
        };
    }
    let more: usize = steps
        .iter()
        .map(|s| match s {
            Step::Copy { files, .. } => files.len(),
            _ => 1,
        })
        .sum();
    let doesnt = if todo.len() == 1 { "doesn't" } else { "don't" };
    Summary {
        text: ready_text(pf, targets, ready),
        help: Some(format!(
            "Also found: {more} more {} in {place} that {} {doesnt} have yet.",
            if more == 1 { "file" } else { "files" },
            join_and(&todo)
        )),
        mood: Mood::Ready,
        button,
        extra: true,
    }
}

/// Switch with nothing to put in place: say which half (keys, firmware) is there.
fn switch_summary(status: &BiosStatus, targets: &[&BiosTarget], name: &dyn Fn(&str) -> String) -> Summary {
    let keys_in: Vec<String> = targets
        .iter()
        .filter(|t| {
            status.files.iter().any(|(b, s, p)| {
                b.file == "prod.keys" && *s == BiosState::Ok && p.as_ref().is_some_and(|p| p.starts_with(&t.dir))
            })
        })
        .map(|t| name(&t.emulator))
        .collect();
    let fw_in: Vec<String> = targets.iter().filter(|t| t.firmware.is_some()).map(|t| name(&t.emulator)).collect();
    let help = "Keys: choose your prod.keys (a .zip works too). Firmware: a folder or .zip of your console's firmware \
                in a folder named like “bios”, or the emulator's own Tools → Install Firmware.";
    let text = match (keys_in.is_empty(), fw_in.is_empty()) {
        (false, true) => format!("Keys are in {}; no Switch firmware found.", join_and(&keys_in)),
        (true, false) => format!("Firmware is in {}; no Switch keys (prod.keys) found.", join_and(&fw_in)),
        _ => "No Switch keys or firmware found.".into(),
    };
    Summary {
        text,
        help: Some(help.into()),
        mood: Mood::Missing,
        button: Some("Choose keys or firmware…".into()),
        extra: false,
    }
}

/// Nothing to put in place and no emulator ready: the sentence for the check's state.
fn state_summary(status: &BiosStatus, home: &Path) -> Summary {
    let pf = status.platform.as_str();
    let what = what_of(pf);
    let found: Vec<&PathBuf> = status
        .files
        .iter()
        .filter(|(b, s, _)| *s == BiosState::Ok && b.file != "firmware")
        .filter_map(|(_, _, p)| p.as_ref())
        .collect();
    match status.state {
        BiosState::NotNeeded => {
            Summary { text: "No BIOS needed.".into(), help: None, mood: Mood::Info, button: None, extra: false }
        }
        BiosState::Wrong => {
            let bad: Vec<String> = status
                .files
                .iter()
                .filter(|(_, s, _)| *s == BiosState::Wrong)
                .filter_map(|(_, _, p)| p.as_ref().map(|p| describe(p, home)))
                .collect();
            Summary {
                text: format!("{what}: {} isn't a good dump.", join_and(&bad)),
                help: Some("Emulators may refuse it. Choose another copy dumped from your console.".into()),
                mood: Mood::Missing,
                button: Some("Choose BIOS files…".into()),
                extra: false,
            }
        }
        BiosState::Ok if !found.is_empty() => {
            let place = places(&found.iter().map(|p| folder_of(p)).collect::<Vec<_>>(), home);
            Summary {
                text: format!("{what} found in {place}."),
                help: None,
                mood: Mood::Ready,
                button: None,
                extra: false,
            }
        }
        BiosState::Ok => Summary {
            text: format!("{what}: nothing missing."),
            help: None,
            mood: Mood::Ready,
            button: None,
            extra: false,
        },
        BiosState::Optional => Summary {
            text: format!("No {what} found — it's optional, games work without it."),
            help: None,
            mood: Mood::Info,
            button: Some("Choose BIOS files…".into()),
            extra: false,
        },
        BiosState::Missing => {
            let (help, button) = match pf {
                "sony_playstation3" => (
                    "Get PS3UPDAT.PUP from PlayStation's website and put it in a folder named like “bios” (or choose it) — \
                     Lodestar then has RPCS3 install it. Or use RPCS3's own File → Install Firmware.",
                    "Choose firmware file…",
                ),
                "sony_vita" => ("Install the firmware from Vita3K's own first-run setup.", "Choose firmware file…"),
                _ => (
                    "Lodestar never downloads BIOS files. Choose the ones you dumped from your console, or put them in a \
                     folder named like “bios” and Lodestar finds them.",
                    "Choose BIOS files…",
                ),
            };
            Summary {
                text: format!("No {what} found."),
                help: Some(help.into()),
                mood: Mood::Missing,
                button: Some(button.into()),
                extra: false,
            }
        }
    }
}
