//! Moonlight (moonlight-qt) scanner: the apps of paired GameStream / Sunshine / Apollo hosts.
//!
//! Data: Moonlight's QSettings file `~/.config/Moonlight Game Streaming Project/Moonlight.conf`
//! (native) or `~/.var/app/com.moonlight_stream.Moonlight/config/Moonlight Game Streaming Project/
//! Moonlight.conf` (Flatpak). Moonlight saves only paired hosts, with the app list it last saw:
//! ```ini
//! [hosts]
//! 1\hostname=GAMING-PC
//! 1\uuid=…
//! 1\apps\1\name=Steam Big Picture
//! 1\apps\1\id=1093255277
//! 1\apps\1\hidden=false
//! 1\apps\size=2
//! size=1
//! ```
//! Box art cache: `~/.cache/Moonlight Game Streaming Project/Moonlight/boxart/<host uuid>/<app id>.png`
//! (Flatpak: under `~/.var/app/<id>/cache/`). Hidden apps are skipped.
//!
//! Games: "<App> (on <host>)", `source_id` = `<host uuid>:<app id>`, launch
//! `moonlight stream <host> <app>` (`flatpak run com.moonlight_stream.Moonlight stream …`).
//!
//! CONTRACT: `pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>>` (see scan/mod.rs).

use super::ScanContext;
use crate::model::*;
use anyhow::Context as _;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

pub mod qt_ini;

pub const FLATPAK_ID: &str = "com.moonlight_stream.Moonlight";
const ORG: &str = "Moonlight Game Streaming Project";

pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    let fp = ctx.flatpak_app(FLATPAK_ID);
    let installs = [
        (
            ctx.config_home().join(ORG).join("Moonlight.conf"),
            ctx.home.join(".cache").join(ORG).join("Moonlight/boxart"),
            false,
        ),
        (fp.join("config").join(ORG).join("Moonlight.conf"), fp.join("cache").join(ORG).join("Moonlight/boxart"), true),
    ];
    let mut out = vec![];
    let mut seen = HashSet::new();
    for (conf, boxart, flatpak) in installs {
        let text = match super::util::read_text(&conf) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e).with_context(|| format!("reading {}", conf.display())),
        };
        let native = if flatpak { String::new() } else { native_program(&ctx.home) };
        for g in parse_conf(&text, &boxart, flatpak, &native) {
            if seen.insert(g.source_id.clone()) {
                out.push(g);
            }
        }
    }
    Ok(out)
}

/// Numbered entries (`<prefix>/<n>/…`) present in the settings, ascending.
fn indices(ini: &HashMap<String, String>, prefix: &str) -> BTreeSet<u32> {
    ini.keys().filter_map(|k| k.strip_prefix(prefix)?.split('/').next()?.parse().ok()).collect()
}

fn parse_conf(text: &str, boxart: &Path, flatpak: bool, native: &str) -> Vec<ScannedGame> {
    let ini = qt_ini::parse(text);
    let get = |k: String| ini.get(&k).map(|v| v.trim()).filter(|v| !v.is_empty());
    let mut out = vec![];
    for h in indices(&ini, "hosts/") {
        let Some(host) = get(format!("hosts/{h}/hostname")) else {
            continue;
        };
        let uuid = get(format!("hosts/{h}/uuid")).unwrap_or(host);
        for a in indices(&ini, &format!("hosts/{h}/apps/")) {
            let p = format!("hosts/{h}/apps/{a}");
            let Some(app) = get(format!("{p}/name")) else {
                continue;
            };
            if get(format!("{p}/hidden")).is_some_and(|v| v.eq_ignore_ascii_case("true")) {
                continue;
            }
            let app_id = get(format!("{p}/id")).unwrap_or(app);
            let mut g = ScannedGame::new(Source::Moonlight, format!("{uuid}:{app_id}"), format!("{app} (on {host})"));
            g.launch = Some(launch(host, app, flatpak, native));
            if safe_component(uuid) && safe_component(app_id) {
                g.art.cover = super::util::file_if_exists(boxart.join(uuid).join(format!("{app_id}.png")));
            }
            out.push(g);
        }
    }
    out
}

/// The native (non-Flatpak) Moonlight: `moonlight` in PATH, else a Moonlight AppImage (same
/// config folder as the native build), else `moonlight` (the launch then says it's missing).
fn native_program(home: &Path) -> String {
    let app = crate::app_locate::MOONLIGHT;
    if let Some(b) = app.bins.iter().find(|b| crate::launch::find_in_path(b).is_some()) {
        return b.to_string();
    }
    crate::app_locate::appimages(home, app.appimage.unwrap_or("moonlight"))
        .into_iter()
        .next()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "moonlight".into())
}

fn safe_component(s: &str) -> bool {
    !s.is_empty() && !s.contains('/') && !s.contains("..")
}

fn launch(host: &str, app: &str, flatpak: bool, native: &str) -> LaunchSpec {
    let mut args: Vec<String> = vec![];
    let program = if flatpak {
        args.extend(["run".into(), FLATPAK_ID.into()]);
        "flatpak".to_string()
    } else {
        native.to_string()
    };
    args.extend(["stream".into(), host.into(), app.into()]);
    LaunchSpec::Command { program, args, cwd: None, env: vec![] }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::desktop::test_support::{ctx, fake_png};

    fn fixture() -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stores/moonlight/Moonlight.conf"),
        )
        .unwrap()
    }

    #[test]
    fn paired_hosts_and_apps() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let conf = home.join(".config").join(ORG).join("Moonlight.conf");
        std::fs::create_dir_all(conf.parent().unwrap()).unwrap();
        std::fs::write(&conf, fixture()).unwrap();
        let uuid = "1F4B7C2A-9C3E-4D5F-8A6B-7C8D9E0F1A2B";
        let art = home.join(".cache").join(ORG).join("Moonlight/boxart").join(uuid).join("1093255277.png");
        fake_png(&art, 600, 800);

        let games = scan(&ctx(home)).unwrap();
        let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Desktop (on GAMING-PC)",
                "Steam Big Picture (on GAMING-PC)",
                "Cyberpunk 2077, Ultimate (on GAMING-PC)",
                "Desktop (on living room)"
            ],
            "hidden apps skipped, hosts in order"
        );
        let steam = &games[1];
        assert_eq!(steam.source, Source::Moonlight);
        assert_eq!(steam.source_id, format!("{uuid}:1093255277"));
        assert_eq!(steam.art.cover, Some(art));
        assert_eq!(
            steam.launch,
            Some(LaunchSpec::Command {
                program: "moonlight".into(),
                args: vec!["stream".into(), "GAMING-PC".into(), "Steam Big Picture".into()],
                cwd: None,
                env: vec![],
            })
        );
        assert_eq!(games[0].art.cover, None);
    }

    #[test]
    fn flatpak_and_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        assert!(scan(&ctx(home)).unwrap().is_empty());
        let conf = home.join(".var/app").join(FLATPAK_ID).join("config").join(ORG).join("Moonlight.conf");
        std::fs::create_dir_all(conf.parent().unwrap()).unwrap();
        std::fs::write(&conf, fixture()).unwrap();
        let games = scan(&ctx(home)).unwrap();
        assert_eq!(games.len(), 4);
        assert!(matches!(&games[3].launch, Some(LaunchSpec::Command { program, args, .. })
            if program == "flatpak" && args[..4] == ["run", FLATPAK_ID, "stream", "living room"]));
        // A config with no paired hosts (fresh install).
        std::fs::write(&conf, "[General]\nlatestsupportedversion-v1=6.1\n").unwrap();
        assert!(scan(&ctx(home)).unwrap().is_empty());
    }
}
