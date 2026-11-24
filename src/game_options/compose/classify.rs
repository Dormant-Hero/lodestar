//! Classifying a launch command (spec §2 launch kinds): peeling known wrappers, launchers and
//! clients off the words to find the core program, and where a gamescope sits in the chain.

use super::*;

pub(super) fn base(w: &str) -> &str {
    Path::new(w).file_name().and_then(|s| s.to_str()).unwrap_or(w)
}

pub(super) fn is_env_word(w: &str) -> bool {
    crate::launch::split_env_word(w).is_some()
}

/// Where the first gamescope sits in the chain: its word, its `--` (None when the user left it
/// out: `gamescope -f %command%`) and the first word of the command it runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GsSplit {
    pub at: usize,
    pub dashdash: Option<usize>,
    pub cmd: usize,
}

impl GsSplit {
    /// The range of the user's gamescope flags.
    pub(super) fn flags(&self) -> std::ops::Range<usize> {
        self.at + 1..self.dashdash.unwrap_or(self.cmd)
    }
}

/// The command chain (user prefix + program + args) and what [`classify`] found in it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Chain {
    pub words: Vec<String>,
    /// Basenames of the wrappers peeled off before the core program.
    pub peeled: Vec<String>,
    /// The first gamescope among the peeled words.
    pub gs: Option<GsSplit>,
    /// Index of the core program in `words`.
    pub core_at: usize,
    pub kind: Core,
    /// The Proton / wine dir the core runs with (None = Unknown).
    pub runner: Option<PathBuf>,
    /// A wine binary inside a Proton dir (`<dir>/files/bin/wine*`).
    pub proton_wine: bool,
    /// Index of a `<dir>/proton run` verb.
    pub proton_run: Option<usize>,
    /// Index of flatpak's `run` word.
    pub flatpak_run: Option<usize>,
    /// Display name of the launcher (L) or store client (C).
    pub launcher: Option<String>,
}

impl Chain {
    pub(super) fn peeled_has(&self, names: &[&str]) -> bool {
        self.peeled.iter().any(|p| names.contains(&p.as_str()))
    }
}

/// Launcher CLIs by program basename (spec §2 L).
pub(super) fn launcher_name(b: &str) -> Option<&'static str> {
    Some(match b {
        "faugus-launcher" => "Faugus",
        "lutris" => "Lutris",
        "heroic" => "Heroic",
        "bottles-cli" => "Bottles",
        "legendary" => "Legendary",
        "nile" => "Nile",
        "itch" => "itch",
        "prismlauncher" => "Prism Launcher",
        "atlauncher" => "ATLauncher",
        "moonlight" | "moonlight-qt" => "Moonlight",
        _ => return None,
    })
}

/// Launcher apps by Flatpak id.
pub fn flatpak_launcher(id: &str) -> Option<&'static str> {
    Some(match id {
        "net.lutris.Lutris" => "Lutris",
        "com.usebottles.bottles" => "Bottles",
        "com.heroicgameslauncher.hgl" => "Heroic",
        "org.prismlauncher.PrismLauncher" => "Prism Launcher",
        "com.atlauncher.ATLauncher" => "ATLauncher",
        "com.moonlight_stream.Moonlight" => "Moonlight",
        "io.github.Faugus.faugus-launcher" => "Faugus",
        _ => return None,
    })
}

/// Display name of a `--client-launch` client (`--client`, else the helper kind).
pub(super) fn client_name(words: &[String]) -> String {
    let get = |opt: &str| words.iter().position(|w| w == opt).and_then(|i| words.get(i + 1)).map(String::as_str);
    let id = get("--client").or_else(|| words.get(1).map(String::as_str)).unwrap_or("");
    match id {
        "battlenet" => "Battle.net",
        "ea" => "the EA app",
        "ubisoft" => "Ubisoft Connect",
        "rockstar" => "the Rockstar Games Launcher",
        "riot" => "the Riot Client",
        "hoyoplay" => "HoYoPlay",
        _ => "the store client",
    }
    .to_string()
}

/// Wrappers that run the rest of their arguments as the command (`taskset -c 0-7 cmd`):
/// classify looks past them for the real core even outside the user's prefix.
pub(super) const PASS_THROUGH: &[&str] = &[
    "taskset",
    "nice",
    "ionice",
    "chrt",
    "numactl",
    "strangle",
    "nohup",
    "setsid",
    "stdbuf",
    "primusrun",
    "optirun",
    "pvkrun",
];

/// Wrappers [`classify`] peels by name (no options of their own to skip).
pub(super) const PLAIN_WRAPPERS: &[&str] =
    &["gamemoderun", "mangohud", "obs-gamecapture", "prime-run", "game-performance"];

/// Gamescope options that take a value as the next word (gamescope --help; `--x=y` needs none).
pub(super) const GS_VALUED: &[&str] = &[
    "-W",
    "--output-width",
    "-H",
    "--output-height",
    "-w",
    "--nested-width",
    "-h",
    "--nested-height",
    "-r",
    "--nested-refresh",
    "-o",
    "--nested-unfocused-refresh",
    "-m",
    "--max-scale",
    "-S",
    "--scaler",
    "-F",
    "--filter",
    "--sharpness",
    "--fsr-sharpness",
    "-R",
    "--ready-fd",
    "-T",
    "--stats-path",
    "-C",
    "--hide-cursor-delay",
    "-O",
    "--prefer-output",
    "--prefer-vk-device",
    "--framerate-limit",
    "--default-touch-mode",
    "--xwayland-count",
    "--generate-drm-mode",
    "--force-orientation",
    "--force-panel-type",
    "--cursor",
    "--cursor-scale-height",
    "--cursor-hotspot",
    "--virtual-connector-strategy",
    "--backend",
    "--sdr-gamut-wideness",
    "--hdr-sdr-content-nits",
    "--hdr-itm-sdr-nits",
    "--hdr-itm-target-nits",
    "--reshade-effect",
    "--reshade-technique-idx",
    "--mura-map",
    "-s",
    "--mouse-sensitivity",
    "--custom-refresh-rates",
    "--display-index",
    "--vr-overlay-key",
    "--vr-app-overlay-key",
    "--vr-overlay-explicit-name",
    "--vr-overlay-default-name",
    "--vr-overlay-icon",
    "--vr-overlay-physical-width",
    "--vr-overlay-physical-curvature",
    "--vr-overlay-physical-pre-curve-pitch",
    "--vr-scroll-speed",
];

/// A word that reads as an option's value rather than a program (`60`, `0-7`, `0x3`, `1.5`);
/// never an option itself (`--`, `-5`: those are handled as options).
pub(super) fn value_like(w: &str) -> bool {
    !w.is_empty()
        && !w.starts_with('-')
        && w.chars().all(|c| c.is_ascii_hexdigit() || matches!(c, 'x' | ',' | '.' | ':' | '-'))
}

/// The word at `i` is a core program [`classify`] recognises (not a wrapper).
pub(super) fn is_core(w: &[String], i: usize, self_exe: Option<&str>) -> bool {
    let Some(word) = w.get(i) else { return false };
    let b = base(word);
    w.get(i + 1).is_some_and(|n| n == "--client-launch")
        || b == "umu-run"
        || b == "proton"
        || (b.starts_with("wine") && b != "wineserver" && !b.starts_with("winetricks"))
        || b == "flatpak"
        || launcher_name(b).is_some()
        || self_exe.is_some_and(|me| me == word)
        || word == "@lodestar"
}

/// Where classification can pick up again after an unrecognised word: a core program or a
/// wrapper it peels (never an option, a `VAR=` word or a word with spaces).
pub(super) fn resumable(w: &[String], i: usize, self_exe: Option<&str>) -> bool {
    let Some(word) = w.get(i) else { return false };
    if word.starts_with('-') || word.contains('=') || word.contains(char::is_whitespace) {
        return false;
    }
    let b = base(word);
    is_core(w, i, self_exe)
        || PLAIN_WRAPPERS.contains(&b)
        || matches!(b, "systemd-inhibit" | "gamescope")
        || (matches!(b, "powerprofilesctl" | "tlpctl") && w.get(i + 1).is_some_and(|x| x == "launch"))
}

/// Gamescope's own options from `at + 1`, getopt-style: (its `--`, first command word), or None
/// when nothing follows them. When a `--` follows, it ends the options unless the getopt scan
/// already reached a core program before it (`gamescope -f umu-run game.exe -- x`): a valued
/// option this list doesn't know (`--new-opt text`, from More Gamescope options) never makes its
/// value the command.
pub(super) fn gamescope_split(w: &[String], at: usize, self_exe: Option<&str>) -> Option<(Option<usize>, usize)> {
    let dashdash = w.iter().skip(at + 1).position(|x| x == "--").map(|p| p + at + 1);
    let mut j = at + 1;
    loop {
        let x = w.get(j)?;
        if x == "--" {
            return Some((Some(j), j + 1));
        }
        if !x.starts_with('-') || x.len() < 2 {
            return match dashdash {
                Some(d) if !is_core(w, j, self_exe) => Some((Some(d), d + 1)),
                _ => Some((None, j)),
            };
        }
        let inline = x.contains('=');
        let valued = !inline && GS_VALUED.contains(&x.as_str());
        // An option this list doesn't know, followed by a number and then more words: its value.
        let unknown_with_value =
            !inline && !valued && w.get(j + 1).is_some_and(|v| value_like(v)) && w.get(j + 2).is_some();
        j += if valued || unknown_with_value { 2 } else { 1 };
    }
}

/// Peel known wrappers off `words` and classify the core program (spec §3.2). `env` is the
/// process env the command will get (for `PROTONPATH`).
pub fn classify(words: Vec<String>, env: &[(String, String)], caps: &Caps, self_exe: Option<&str>) -> Chain {
    classify_with(words, 0, env, caps, self_exe)
}

/// [`classify`] for a chain whose first `prefix_len` words are the user's prefix (the words
/// before `%command%`): an unrecognised word there (a script of the user's) is looked past up to
/// the base program, so `taskset -c 0-7 %command%` around umu-run is still a Proton launch.
pub fn classify_with(
    words: Vec<String>,
    prefix_len: usize,
    env: &[(String, String)],
    caps: &Caps,
    self_exe: Option<&str>,
) -> Chain {
    let mut c = Chain { words, ..Default::default() };
    c.core_at = peel(&mut c, prefix_len, self_exe);
    classify_core(&mut c, env, caps, self_exe);
    c
}

/// Peel the known wrappers off `c.words` (recording them in `c.peeled`, gamescope in `c.gs`);
/// returns where the core program starts.
fn peel(c: &mut Chain, prefix_len: usize, self_exe: Option<&str>) -> usize {
    let w = &c.words;
    let mut i = 0;
    while let Some(word) = w.get(i) {
        let b = base(word);
        match b {
            "env" => {
                let j = after_env_options(w, i + 1);
                if w.get(j).is_none_or(|x| x.starts_with('-')) {
                    break;
                }
                i = j;
            }
            _ if PLAIN_WRAPPERS.contains(&b) => i += 1,
            "systemd-inhibit" => {
                let mut j = i + 1;
                while let Some(a) = w.get(j).filter(|a| a.starts_with("--")) {
                    let two = !a.contains('=') && matches!(a.as_str(), "--what" | "--who" | "--why" | "--mode");
                    j += if two { 2 } else { 1 };
                }
                i = j;
            }
            "powerprofilesctl" | "tlpctl" if w.get(i + 1).is_some_and(|x| x == "launch") => {
                let j = after_profile_launch_options(w, i + 2);
                if j >= w.len() {
                    break;
                }
                i = j;
            }
            "gamescope" => match gamescope_split(w, i, self_exe) {
                Some((dashdash, cmd)) => {
                    if c.gs.is_none() {
                        c.gs = Some(GsSplit { at: i, dashdash, cmd });
                    }
                    i = cmd;
                }
                None => break,
            },
            _ => {
                if is_core(w, i, self_exe) {
                    break;
                }
                // An unrecognised wrapper: look past it for the real core and leave it in place
                // (not peeled, so no toggle takes it for its own wrapper).
                let pass = PASS_THROUGH.contains(&b);
                let end = if pass {
                    w.len()
                } else if i < prefix_len {
                    (prefix_len + 1).min(w.len())
                } else {
                    break;
                };
                let mut found = None;
                for j in i + 1..end {
                    if resumable(w, j, self_exe) {
                        found = Some(j);
                        break;
                    }
                    // Past a pass-through wrapper's options the first program-like word is its
                    // command: a game's own arguments are never searched.
                    let x = &w[j];
                    if pass && j >= prefix_len && !x.starts_with('-') && !value_like(x) {
                        break;
                    }
                }
                match found {
                    Some(j) => {
                        i = j;
                        continue;
                    }
                    None => break,
                }
            }
        }
        c.peeled.push(b.to_string());
    }
    i
}

/// The word after `env`'s variables and options, from `j` (its first argument).
fn after_env_options(w: &[String], mut j: usize) -> usize {
    loop {
        match w.get(j).map(String::as_str) {
            Some(x) if is_env_word(x) => j += 1,
            Some("-u" | "--unset" | "-C" | "--chdir") => j += 2,
            Some(x)
                if matches!(x, "-" | "--" | "-i" | "--ignore-environment" | "-0" | "--null" | "-v" | "--debug")
                    || x.starts_with("--unset=")
                    || x.starts_with("--chdir=")
                    || (x.len() > 2 && (x.starts_with("-u") || x.starts_with("-C"))) =>
            {
                j += 1
            }
            _ => return j,
        }
    }
}

/// The command word of `powerprofilesctl` / `tlpctl launch [-p P] [-r R] [-i ID] [--] COMMAND`
/// (argparse REMAINDER: `--` is optional), from `j` (the word after `launch`).
fn after_profile_launch_options(w: &[String], mut j: usize) -> usize {
    loop {
        match w.get(j).map(String::as_str) {
            Some("--") => return j + 1,
            Some("-p" | "--profile" | "-r" | "--reason" | "-i" | "--appid") => j += 2,
            Some(x) if x.starts_with('-') => j += 1,
            _ => return j,
        }
    }
}

/// What kind of program the core at `c.core_at` is (Proton, Wine, a Flatpak, a launcher …) and
/// the runner or launcher it names.
fn classify_core(c: &mut Chain, env: &[(String, String)], caps: &Caps, self_exe: Option<&str>) {
    let (w, i) = (&c.words, c.core_at);
    let Some(core) = w.get(i).cloned() else { return };
    let b = base(&core).to_string();
    let next = w.get(i + 1).map(String::as_str);
    if next == Some("--client-launch") {
        c.kind = Core::Client;
        c.launcher = Some(client_name(&w[i + 1..]));
        c.runner = client_runner(&w[i + 1..], caps);
    } else if b == "umu-run" || b == "proton" {
        c.kind = Core::Proton;
        c.runner = proton_dir_for(env, &core, caps);
        if b == "proton" && next == Some("run") {
            c.proton_run = Some(i + 1);
        }
    } else if b.starts_with("wine") && b != "wineserver" && !b.starts_with("winetricks") {
        c.kind = Core::Wine;
        let p = Path::new(&core);
        let bin = p.parent();
        // `<dir>/files/bin/wine*` is Proton's layout (Lutris / Bottles wines use `<dir>/bin`), so
        // the path alone marks it; its caps come from `caps.runners` or the runner probe.
        let proton_dir = bin.filter(|d| d.ends_with("files/bin")).and_then(|d| d.parent()?.parent());
        if let Some(d) = proton_dir {
            c.proton_wine = true;
            c.runner = Some(d.to_path_buf());
        } else {
            c.runner = bin.and_then(Path::parent).filter(|r| caps.runners.contains_key(*r)).map(Path::to_path_buf);
        }
    } else if b == "flatpak" {
        let mut j = i + 1;
        while w.get(j).is_some_and(|x| x.starts_with('-')) {
            j += 1;
        }
        if w.get(j).is_some_and(|x| x == "run") {
            let opts_end = (j + 1..w.len()).find(|k| !w[*k].starts_with('-')).unwrap_or(w.len());
            let bottles = w[j + 1..opts_end]
                .iter()
                .any(|o| o.strip_prefix("--command=").is_some_and(|cmd| base(cmd) == "bottles-cli"));
            let app = w.get(opts_end).map(String::as_str).unwrap_or("");
            if let Some(l) = flatpak_launcher(app).or(bottles.then_some("Bottles")) {
                c.kind = Core::Launcher;
                c.launcher = Some(l.to_string());
            } else {
                c.kind = Core::Flatpak;
                c.flatpak_run = Some(j);
            }
        }
    } else if let Some(l) = launcher_name(&b) {
        c.kind = Core::Launcher;
        c.launcher = Some(l.to_string());
    } else if self_exe.is_some_and(|me| me == core) || core == "@lodestar" {
        c.kind = Core::Launcher;
        c.launcher = Some("Lodestar".into());
    }
}

/// The core program of a command that is already composed (`program` + `args`, Lodestar's and
/// the user's wrappers included), found again by peeling the wrappers: what
/// [`ComposeOutput::core_program`] names. Unrecognised wrappers are only looked past when they
/// are known pass-through ones (the user's prefix isn't known here).
pub fn core_program(argv: &[String], self_exe: Option<&str>) -> Option<String> {
    let c = classify(argv.to_vec(), &[], &Caps::default(), self_exe);
    c.words.get(c.core_at).cloned()
}

/// The Proton dir of a `--client-launch … --runner proton:<path>` helper.
pub(super) fn client_runner(words: &[String], caps: &Caps) -> Option<PathBuf> {
    let i = words.iter().position(|w| w == "--runner")?;
    let p = PathBuf::from(words.get(i + 1)?.strip_prefix("proton:")?);
    if caps.runners.contains_key(&p) || base(&p.to_string_lossy()) != "proton" {
        Some(p)
    } else {
        p.parent().map(Path::to_path_buf)
    }
}
