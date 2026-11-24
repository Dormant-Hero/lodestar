//! Faugus Launcher scanner.
//!
//! Data: `~/.local/share/faugus-launcher/games.json` (native) and
//! `~/.var/app/io.github.Faugus.faugus-launcher/data/faugus-launcher/games.json` (Flatpak).
//! `games.json` is a JSON array of objects (Faugus 1.4):
//! `{gameid, title, path, prefix, launch_arguments, game_arguments, mangohud, gamemode, runner,
//!   playtime (seconds), hidden, icon (abs path), cover, last_played ("2026-10-31T15:26:40.31", local time) …}`.
//! Boolean-ish fields are either JSON booleans or `""`.
//! Art: `covers/<gameid>.*`, `banners/<gameid>.*`, `icons/<gameid>.*` next to games.json
//! (or the absolute `icon`/`cover` paths stored per game).
//!
//! CONTRACT: `pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>>` (see scan/mod.rs).

use super::ScanContext;
use super::desktop::common;
use super::util::{file_if_exists, image_with_stem};
use crate::model::*;
use anyhow::Context as _;
use serde_json::Value;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const FLATPAK_ID: &str = "io.github.Faugus.faugus-launcher";

pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    let installs = [
        (ctx.data_home().join("faugus-launcher"), false),
        (ctx.flatpak_app(FLATPAK_ID).join("data/faugus-launcher"), true),
    ];
    let mut out = vec![];
    let mut seen = HashSet::new();
    for (dir, flatpak) in installs {
        let file = dir.join("games.json");
        if !file.is_file() {
            continue;
        }
        let text = crate::scan::util::read_text(&file).with_context(|| format!("reading {}", file.display()))?;
        let games = parse_games(&text, &dir, flatpak).with_context(|| format!("parsing {}", file.display()))?;
        for g in games {
            if seen.insert(g.source_id.clone()) {
                out.push(g);
            }
        }
    }
    Ok(out)
}

/// Parse a games.json document. `dir` is the faugus data dir (for covers/banners/icons).
pub fn parse_games(text: &str, dir: &Path, flatpak: bool) -> anyhow::Result<Vec<ScannedGame>> {
    if text.trim().is_empty() {
        return Ok(vec![]); // Faugus writes an empty file before the first game is added
    }
    let v: Value = serde_json::from_str(text)?;
    let list = match &v {
        Value::Array(a) => a.as_slice(),
        Value::Object(o) => o.get("games").and_then(|g| g.as_array()).map(|a| a.as_slice()).unwrap_or(&[]),
        _ => anyhow::bail!("unexpected games.json shape"),
    };
    Ok(list.iter().filter_map(|g| game_from_json(g, dir, flatpak)).collect())
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).map(str::trim).unwrap_or("")
}

/// Faugus stores flags as `true`/`false`, `""` or occasionally `"True"`.
fn flag(v: &Value, k: &str) -> bool {
    match v.get(k) {
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
        Some(Value::Number(n)) => n.as_i64().is_some_and(|n| n != 0),
        _ => false,
    }
}

fn game_from_json(v: &Value, dir: &Path, flatpak: bool) -> Option<ScannedGame> {
    let id = s(v, "gameid");
    let title = s(v, "title");
    // A store client added to Faugus: its games come from the Wine-prefix scanner.
    if id.is_empty()
        || title.is_empty()
        || super::clients::is_client_name(title)
        || super::clients::is_client_exe(s(v, "path"))
    {
        return None;
    }
    let mut g = ScannedGame::new(Source::Faugus, id, title);
    let path = s(v, "path");
    if !path.is_empty() {
        let p = Path::new(path);
        if p.is_absolute() {
            g.installed = p.exists();
            g.install_dir = p.parent().map(Path::to_path_buf);
        }
        g.platform = common::is_windows_exe(path).then(|| "windows".to_string());
    }
    g.launch = Some(if flatpak {
        LaunchSpec::Command {
            program: "flatpak".into(),
            args: vec!["run".into(), FLATPAK_ID.into(), "--game".into(), id.into()],
            cwd: None,
            env: vec![],
        }
    } else {
        LaunchSpec::Command {
            program: "faugus-launcher".into(),
            args: vec!["--game".into(), id.into()],
            cwd: None,
            env: vec![],
        }
    });
    g.source_launch_options = launch_options(v);
    g.playtime_secs = match v.get("playtime") {
        Some(Value::Number(n)) => n.as_u64().or_else(|| n.as_f64().map(|f| f.max(0.0) as u64)),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
    .filter(|&p| p > 0);
    g.last_played = Some(s(v, "last_played")).filter(|x| !x.is_empty()).and_then(|x| common::parse_datetime(x, true));

    let abs = |k: &str| Some(s(v, k)).filter(|p| p.starts_with('/')).and_then(file_if_exists);
    g.art.cover = abs("cover").or_else(|| image_with_stem(&dir.join("covers"), id));
    g.art.banner = abs("banner").or_else(|| image_with_stem(&dir.join("banners"), id));
    g.art.hero = g.art.banner.clone();
    g.art.icon = abs("icon")
        .or_else(|| image_with_stem(&dir.join("icons"), id))
        .or_else(|| file_if_exists(dir.join("icons").join(format!("{id}.ico"))));
    Some(g)
}

/// A command that runs a Faugus game directly with umu-run (or natively for `Linux-Native`),
/// bypassing Faugus and therefore Faugus' own launch options (MangoHud, gamemode, wrappers).
/// Lets the user decide the options in Lodestar: put [`DirectLaunch::command_line`] into
/// `LaunchOverride::command` and the override's `options` (`%command%`, `VAR=value`) apply to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectLaunch {
    pub program: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(String, String)>,
}

impl DirectLaunch {
    /// `env VAR=value … program args …`, shell-like quoted (no shell needed to run it).
    pub fn command_line(&self) -> String {
        let mut words: Vec<&str> = vec![];
        let env: Vec<String> = self.env.iter().map(|(k, v)| format!("{k}={v}")).collect();
        if !env.is_empty() {
            words.push("env");
            words.extend(env.iter().map(String::as_str));
        }
        words.push(&self.program);
        words.extend(self.args.iter().map(String::as_str));
        words.iter().map(|w| quote_word(w)).collect::<Vec<_>>().join(" ")
    }
}

fn quote_word(w: &str) -> String {
    if !w.is_empty() && w.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:+,@".contains(c)) {
        w.to_string()
    } else {
        format!("\"{}\"", w.replace('\\', "\\\\").replace('"', "\\\"").replace('$', "\\$").replace('`', "\\`"))
    }
}

/// Build the direct command for Faugus game `gameid` (reads games.json; cheap, but call it from a
/// click handler, not every frame). Mirrors how Faugus runs games: `WINEPREFIX=<prefix>`,
/// `GAMEID=umu-default`, `PROTONPATH=<runner>` (resolved in Steam's `compatibilitytools.d` when
/// it is a directory there), Faugus' bundled `umu-run` when present, `game_arguments` appended.
pub fn direct_launch(ctx: &ScanContext, gameid: &str) -> anyhow::Result<DirectLaunch> {
    for dir in data_dirs(ctx) {
        let Ok(text) = crate::scan::util::read_text(dir.join("games.json")) else {
            continue;
        };
        let v: Value = serde_json::from_str(&text).with_context(|| format!("parsing {}/games.json", dir.display()))?;
        let Some(game) = v.as_array().and_then(|a| a.iter().find(|g| s(g, "gameid") == gameid)) else {
            continue;
        };
        return direct_from_json(ctx, game, &dir);
    }
    anyhow::bail!("Faugus game {gameid:?} not found")
}

fn direct_from_json(ctx: &ScanContext, g: &Value, dir: &Path) -> anyhow::Result<DirectLaunch> {
    let exe = s(g, "path");
    if exe.is_empty() {
        anyhow::bail!("this Faugus entry has no executable path");
    }
    let cwd = Path::new(exe).parent().filter(|p| p.is_absolute()).map(Path::to_path_buf);
    let game_args = crate::scan::desktop::parse_exec(s(g, "game_arguments"));
    let runner = s(g, "runner");
    if runner == "Linux-Native" {
        return Ok(DirectLaunch { program: exe.to_string(), args: game_args, cwd, env: vec![] });
    }
    let mut env = vec![];
    let prefix = s(g, "prefix");
    if !prefix.is_empty() {
        env.push(("WINEPREFIX".to_string(), prefix.to_string()));
    }
    env.push(("GAMEID".to_string(), "umu-default".to_string()));
    let proton = match runner {
        "" | "UMU-Proton Latest" => None,
        "GE-Proton Latest (default)" | "GE-Proton Latest" | "GE-Proton" => Some("GE-Proton".to_string()),
        "Proton-EM Latest" | "Proton-EM" => Some("Proton-EM".to_string()),
        name => {
            let candidates = [
                ctx.data_home().join("Steam/compatibilitytools.d").join(name),
                ctx.home.join(".steam/root/compatibilitytools.d").join(name),
                ctx.flatpak_app("com.valvesoftware.Steam").join("data/Steam/compatibilitytools.d").join(name),
                PathBuf::from("/usr/share/steam/compatibilitytools.d").join(name),
            ];
            Some(
                super::util::first_existing(candidates)
                    .map_or_else(|| name.to_string(), |p| p.to_string_lossy().into_owned()),
            )
        }
    };
    if let Some(p) = proton {
        env.push(("PROTONPATH".to_string(), p));
    }
    let umu =
        file_if_exists(dir.join("umu-run")).map_or_else(|| "umu-run".to_string(), |p| p.to_string_lossy().into_owned());
    let mut args = vec![exe.to_string()];
    args.extend(game_args);
    Ok(DirectLaunch { program: umu, args, cwd, env })
}

/// Faugus' per-game options as a Steam-style string:
/// `MANGOHUD=1 gamemoderun <launch_arguments> %command% <game_arguments>`.
/// `launch_arguments` in Faugus are wrappers/env placed before the runner; `game_arguments`
/// go after the executable.
pub fn launch_options(v: &Value) -> Option<String> {
    let mut pre: Vec<String> = vec![];
    if flag(v, "mangohud") {
        pre.push("MANGOHUD=1".into());
    }
    if flag(v, "gamemode") {
        pre.push("gamemoderun".into());
    }
    let la = s(v, "launch_arguments");
    if !la.is_empty() {
        pre.push(la.to_string());
    }
    let ga = s(v, "game_arguments");
    match (pre.is_empty(), ga.is_empty()) {
        (true, true) => None,
        (true, false) => Some(ga.to_string()),
        (false, true) => Some(format!("{} %command%", pre.join(" "))),
        (false, false) => Some(format!("{} %command% {ga}", pre.join(" "))),
    }
}

/// Where Faugus keeps its data (native first). Exposed for other modules (e.g. metadata).
pub fn data_dirs(ctx: &ScanContext) -> Vec<PathBuf> {
    [ctx.data_home().join("faugus-launcher"), ctx.flatpak_app(FLATPAK_ID).join("data/faugus-launcher")]
        .into_iter()
        .filter(|d| d.is_dir())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::desktop::test_support::*;
    use super::*;

    #[test]
    fn parses_fixture() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dir = home.join(".local/share/faugus-launcher");
        copy_tree(&fixture("faugus/faugus-launcher"), &dir);
        let text = std::fs::read_to_string(dir.join("games.json")).unwrap();
        std::fs::write(dir.join("games.json"), text.replace("/HOME", &home.to_string_lossy())).unwrap();
        fake_png(&dir.join("icons/football-life-2026.png"), 256, 256);
        fake_png(&dir.join("banners/football-life-2026.png"), 920, 430);
        fake_png(&dir.join("covers/cyber-game.png"), 600, 900);
        let exe = home.join("Faugus/football-life/drive_c/Program Files (x86)/SP Football Life 2026/FL 2026 start.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "MZ").unwrap();

        let games = scan(&ctx(home)).unwrap();
        assert_eq!(games.len(), 3, "{games:#?}");
        let fl = &games[0];
        assert_eq!(fl.source_id, "football-life-2026");
        assert_eq!(fl.name, "Football Life 2026");
        assert!(fl.installed);
        assert_eq!(fl.install_dir.as_deref(), exe.parent());
        assert_eq!(fl.platform.as_deref(), Some("windows"));
        assert_eq!(fl.playtime_secs, Some(18755));
        assert!(fl.last_played.is_some_and(|t| t > 1_790_000_000));
        assert_eq!(
            fl.source_launch_options.as_deref(),
            Some("MANGOHUD=1 game-performance gamescope -f --adaptive-sync -W 2560 -H 1440 -- %command%")
        );
        assert_eq!(fl.art.icon, Some(dir.join("icons/football-life-2026.png")));
        assert_eq!(fl.art.banner, Some(dir.join("banners/football-life-2026.png")));
        assert_eq!(fl.art.hero, fl.art.banner);
        assert_eq!(fl.art.cover, None);
        assert_eq!(
            fl.launch,
            Some(LaunchSpec::Command {
                program: "faugus-launcher".into(),
                args: vec!["--game".into(), "football-life-2026".into()],
                cwd: None,
                env: vec![]
            })
        );

        let cg = &games[1];
        assert_eq!(cg.source_id, "cyber-game");
        assert!(!cg.installed, "exe missing");
        assert_eq!(cg.art.cover, Some(dir.join("covers/cyber-game.png")));
        assert_eq!(cg.source_launch_options.as_deref(), Some("gamemoderun %command% -skipintro"));
        assert_eq!(cg.playtime_secs, None);
        assert_eq!(cg.last_played, None);

        let hidden = &games[2];
        assert_eq!(hidden.source_id, "hidden-tool");
        assert_eq!(hidden.source_launch_options.as_deref(), Some("-windowed"));
        assert_eq!(hidden.platform, None);
    }

    #[test]
    fn direct_launch_bypasses_faugus() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let dir = home.join(".local/share/faugus-launcher");
        copy_tree(&fixture("faugus/faugus-launcher"), &dir);
        let text = std::fs::read_to_string(dir.join("games.json")).unwrap();
        std::fs::write(dir.join("games.json"), text.replace("/HOME", &home.to_string_lossy())).unwrap();
        std::fs::write(dir.join("umu-run"), "#!/bin/sh").unwrap();
        let tool = home.join(".local/share/Steam/compatibilitytools.d/Proton-CachyOS Latest");
        std::fs::create_dir_all(&tool).unwrap();
        let h = home.to_string_lossy();

        let d = direct_launch(&ctx(home), "football-life-2026").unwrap();
        assert_eq!(d.program, dir.join("umu-run").to_string_lossy());
        assert_eq!(
            d.args,
            vec![format!(
                "{h}/Faugus/football-life/drive_c/Program Files (x86)/SP Football Life 2026/FL 2026 start.exe"
            )]
        );
        assert_eq!(
            d.env,
            vec![
                ("WINEPREFIX".into(), format!("{h}/Faugus/football-life")),
                ("GAMEID".into(), "umu-default".into()),
                ("PROTONPATH".into(), tool.to_string_lossy().into_owned()),
            ]
        );
        assert_eq!(d.cwd.as_deref(), Path::new(&d.args[0]).parent());
        let cl = d.command_line();
        assert!(
            cl.starts_with(&format!("env WINEPREFIX={h}/Faugus/football-life GAMEID=umu-default \"PROTONPATH={h}")),
            "{cl}"
        );
        assert!(cl.ends_with("/FL 2026 start.exe\""), "{cl}");
        // Round-trips through the desktop Exec splitter (same quoting rules).
        assert_eq!(crate::scan::desktop::parse_exec(&cl).len(), 6);

        let gd = direct_launch(&ctx(home), "cyber-game").unwrap();
        assert_eq!(gd.env[2], ("PROTONPATH".into(), "GE-Proton".into()), "unknown runner name passed to umu as is");
        assert_eq!(gd.args[1], "-skipintro");
        let native = direct_launch(&ctx(home), "hidden-tool").unwrap();
        assert_eq!(native.program, format!("{h}/Faugus/tools/run.sh"));
        assert!(native.env.is_empty());
        assert!(direct_launch(&ctx(home), "nope").is_err());
    }

    #[test]
    fn flatpak_launch_and_dedupe() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let json = r#"[{"gameid":"a","title":"A","path":"/x/a.exe"},{"gameid":"b","title":"B","path":""}]"#;
        let native = home.join(".local/share/faugus-launcher");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(native.join("games.json"), json[..json.find(",{").unwrap()].to_string() + "]").unwrap();
        let fp = home.join(".var/app").join(FLATPAK_ID).join("data/faugus-launcher");
        std::fs::create_dir_all(&fp).unwrap();
        std::fs::write(fp.join("games.json"), json).unwrap();
        let games = scan(&ctx(home)).unwrap();
        assert_eq!(games.len(), 2);
        assert!(matches!(&games[0].launch, Some(LaunchSpec::Command { program, .. }) if program == "faugus-launcher"));
        match &games[1].launch {
            Some(LaunchSpec::Command { program, args, .. }) => {
                assert_eq!(program, "flatpak");
                assert_eq!(args, &["run", FLATPAK_ID, "--game", "b"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn errors_and_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        assert!(scan(&ctx(home)).unwrap().is_empty(), "missing launcher = Ok(empty)");
        let native = home.join(".local/share/faugus-launcher");
        std::fs::create_dir_all(&native).unwrap();
        std::fs::write(native.join("games.json"), "").unwrap();
        assert!(scan(&ctx(home)).unwrap().is_empty());
        std::fs::write(native.join("games.json"), "[{\"gameid\": ").unwrap();
        assert!(scan(&ctx(home)).is_err(), "corrupt data = Err");
        std::fs::write(
            native.join("games.json"),
            r#"[{"gameid": 5}, "x", {"gameid":"ok","title":"Ok","playtime":"12"}]"#,
        )
        .unwrap();
        let g = scan(&ctx(home)).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].playtime_secs, Some(12));
    }

    #[test]
    #[ignore]
    fn real_faugus() {
        let ctx = ScanContext::from_config(&crate::config::Config::default());
        for g in scan(&ctx).unwrap() {
            println!("{g:#?}");
        }
    }
}
