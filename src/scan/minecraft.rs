//! Minecraft launchers' instances: Prism Launcher (and PolyMC, same format), MultiMC, ATLauncher.
//! Each instance is one game, "Minecraft: <instance>".
//!
//! * Prism / PolyMC / MultiMC (QSettings files, read with `moonlight::qt_ini`):
//!   - data roots: `~/.local/share/PrismLauncher` (native), `~/.var/app/org.prismlauncher.
//!     PrismLauncher/data/PrismLauncher` (Flatpak), `~/.local/share/PolyMC` (+ Flatpak
//!     org.polymc.PolyMC), `~/.local/share/multimc`;
//!   - `<root>/prismlauncher.cfg` (`polymc.cfg`, `multimc.cfg`): `InstanceDir=` (default
//!     `instances`, relative to the root or absolute), `IconsDir=` (default `icons`);
//!   - `<instances>/<id>/instance.cfg`: `name`, `iconKey`, `lastLaunchTime` (unix ms),
//!     `totalTimePlayed` (seconds); `mmc-pack.json` `components[{uid, version}]` gives the
//!     Minecraft version and mod loader for the description;
//!   - icon: `<icons>/<iconKey>.{png,jpg,…}` when it is a custom icon (built-in ones live in the
//!     launcher binary);
//!   - launch: `prismlauncher -l <id>` / `flatpak run org.prismlauncher.PrismLauncher -l <id>` /
//!     `polymc -l <id>` / `<root>/MultiMC -l <id>` (or `multimc` on PATH);
//!   - Prism as an AppImage (Lutris' `prism-launcher` installer, or in `~/Applications`,
//!     `~/AppImages`, `~/.local/bin`, `~/bin`): `<AppImage> --dir <root> -l <id>`, for the native
//!     folder when no `prismlauncher` is on PATH, and for the AppImage's own data folders —
//!     portable mode (`portable.txt` / `prismlauncher.cfg` / `instances` next to it, or in a
//!     `data` folder there), `<AppImage>.home/.local/share/PrismLauncher`, and, when Flatpak
//!     Lutris runs it, Lutris' sandbox `~/.var/app/net.lutris.Lutris/data/PrismLauncher`
//!     (`source_id` `prism-appimage:<id>` for those).
//! * ATLauncher: `~/.local/share/ATLauncher` or `~/.local/share/atlauncher` (AUR package), the
//!   Flatpak `com.atlauncher.ATLauncher`'s data folder, `~/ATLauncher` (portable jar);
//!   `instances/<dir>/instance.json`: `launcher.name`, `launcher.lastPlayed`,
//!   `launcher.totalPlayTime` (seconds), `id` (Minecraft version), `launcher.loaderVersion.type`;
//!   icon `instance.png`; launch `atlauncher --launch <name>` (Flatpak: `flatpak run
//!   com.atlauncher.ATLauncher --launch <name>`; portable: `java -jar <root>/ATLauncher.jar …`).
//!
//! `source_id` = `<launcher>:<instance folder>` (`prism:`, `polymc:`, `multimc:`, `atlauncher:`);
//! a native and a Flatpak install of the same launcher share ids (native first wins).
//!
//! CONTRACT: `pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>>` (see scan/mod.rs).

use super::ScanContext;
use super::moonlight::qt_ini;
use crate::model::*;
use anyhow::Context as _;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub const PRISM_FLATPAK: &str = "org.prismlauncher.PrismLauncher";
pub const POLYMC_FLATPAK: &str = "org.polymc.PolyMC";
pub const ATLAUNCHER_FLATPAK: &str = "com.atlauncher.ATLauncher";

/// Most instance folders read per launcher.
const MAX_INSTANCES: usize = 2000;

/// A MultiMC-family launcher install.
struct Mmc {
    root: PathBuf,
    cfg: &'static str,
    key: &'static str,
    /// Program + leading args; `-l <id>` is appended.
    cmd: Vec<String>,
}

fn sv(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// A Prism Launcher AppImage and whether Flatpak Lutris runs it (then its data lives in
/// Lutris' sandbox: `~/.var/app/net.lutris.Lutris/data/PrismLauncher`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct AppImage {
    path: PathBuf,
    lutris_flatpak: bool,
}

fn is_prism_appimage(p: &Path) -> bool {
    let name = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    name.ends_with(".appimage") && name.contains("prism") && p.is_file()
}

/// Prism AppImages: the ones Lutris installed (`games/*.yml` `game: exe:`), then the usual
/// AppImage folders (`~/Applications`, `~/AppImages`, `~/.local/bin`, `~/bin`).
fn prism_appimages(ctx: &ScanContext) -> Vec<AppImage> {
    let mut out: Vec<AppImage> = vec![];
    let add = |out: &mut Vec<AppImage>, path: PathBuf, lutris_flatpak: bool| {
        if is_prism_appimage(&path) && !out.iter().any(|a| a.path == path) {
            out.push(AppImage { path, lutris_flatpak });
        }
    };
    let fp = ctx.flatpak_app(super::lutris::FLATPAK_ID);
    for (dir, flatpak) in [
        (ctx.config_home().join("lutris/games"), false),
        (ctx.data_home().join("lutris/games"), false),
        (fp.join("config/lutris/games"), true),
        (fp.join("data/lutris/games"), true),
    ] {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .take(2000)
            .map(|e| e.path())
            .filter(|p| {
                let n = p.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase());
                n.is_some_and(|n| n.ends_with(".yml") && n.contains("prism"))
            })
            .collect();
        files.sort();
        for f in files {
            let Ok(text) = super::util::read_text(&f) else {
                continue;
            };
            let y = super::winprefix::lutris_yaml(&text);
            if let Some(exe) = y.get("game.exe").filter(|e| e.starts_with('/')) {
                add(&mut out, PathBuf::from(exe), flatpak);
            }
        }
    }
    for dir in ["Applications", "AppImages", ".local/bin", "bin"] {
        let Ok(rd) = std::fs::read_dir(ctx.home.join(dir)) else {
            continue;
        };
        let mut v: Vec<PathBuf> = rd.flatten().take(2000).map(|e| e.path()).collect();
        v.sort();
        for p in v {
            add(&mut out, p, false);
        }
    }
    out
}

/// Data folders of a Prism AppImage other than the native default: portable mode (a
/// `portable.txt` / config next to the AppImage, or a `data` folder there), the AppImage's own
/// `<AppImage>.home`, and Flatpak Lutris' sandbox.
fn appimage_roots(ctx: &ScanContext, a: &AppImage) -> Vec<PathBuf> {
    let mut v = vec![];
    if let Some(dir) = a.path.parent() {
        let portable = |d: &Path| {
            d.join("portable.txt").is_file() || d.join("prismlauncher.cfg").is_file() || d.join("instances").is_dir()
        };
        if portable(dir) {
            v.push(dir.to_path_buf());
        }
        if portable(&dir.join("data")) {
            v.push(dir.join("data"));
        }
    }
    let mut home = a.path.clone().into_os_string();
    home.push(".home");
    v.push(PathBuf::from(home).join(".local/share/PrismLauncher"));
    if a.lutris_flatpak {
        v.push(ctx.flatpak_app(super::lutris::FLATPAK_ID).join("data/PrismLauncher"));
    }
    v
}

/// `<AppImage> --dir <root>` (Prism's data-folder option; `-l <id>` is appended).
fn appimage_cmd(a: &Path, root: &Path) -> Vec<String> {
    vec![a.to_string_lossy().into_owned(), "--dir".into(), root.to_string_lossy().into_owned()]
}

fn mmc_launchers(ctx: &ScanContext) -> Vec<Mmc> {
    let data = ctx.data_home();
    let appimages = prism_appimages(ctx);
    // The native data folder belongs to whichever Prism is installed: the package, else a
    // (non-Lutris-Flatpak) AppImage.
    let native_root = data.join("PrismLauncher");
    let native_cmd = match appimages.iter().find(|a| !a.lutris_flatpak) {
        Some(a) if super::winprefix::which("prismlauncher").is_none() => appimage_cmd(&a.path, &native_root),
        _ => sv(&["prismlauncher"]),
    };
    let multimc = data.join("multimc");
    let multimc_cmd = ["MultiMC", "bin/MultiMC"]
        .iter()
        .map(|p| multimc.join(p))
        .find(|p| p.is_file())
        .map(|p| vec![p.to_string_lossy().into_owned()])
        .unwrap_or_else(|| sv(&["multimc"]));
    let mut v = vec![
        Mmc { root: native_root.clone(), cfg: "prismlauncher.cfg", key: "prism", cmd: native_cmd },
        Mmc {
            root: ctx.flatpak_app(PRISM_FLATPAK).join("data/PrismLauncher"),
            cfg: "prismlauncher.cfg",
            key: "prism",
            cmd: sv(&["flatpak", "run", PRISM_FLATPAK]),
        },
        Mmc { root: data.join("PolyMC"), cfg: "polymc.cfg", key: "polymc", cmd: sv(&["polymc"]) },
        Mmc {
            root: ctx.flatpak_app(POLYMC_FLATPAK).join("data/PolyMC"),
            cfg: "polymc.cfg",
            key: "polymc",
            cmd: sv(&["flatpak", "run", POLYMC_FLATPAK]),
        },
        Mmc { root: multimc, cfg: "multimc.cfg", key: "multimc", cmd: multimc_cmd },
    ];
    for a in &appimages {
        for root in appimage_roots(ctx, a) {
            if root != native_root && !v.iter().any(|m| m.root == root) {
                v.push(Mmc {
                    cmd: appimage_cmd(&a.path, &root),
                    root,
                    cfg: "prismlauncher.cfg",
                    key: "prism-appimage",
                });
            }
        }
    }
    v
}

pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    let mut out = vec![];
    let mut seen = HashSet::new();
    let mut push = |g: ScannedGame, out: &mut Vec<ScannedGame>| {
        if seen.insert(g.source_id.clone()) {
            out.push(g);
        }
    };
    for l in mmc_launchers(ctx) {
        if !l.root.is_dir() {
            continue;
        }
        for g in scan_mmc(&l)? {
            push(g, &mut out);
        }
    }
    for (root, cmd) in atlauncher_roots(ctx) {
        if !root.join("instances").is_dir() {
            continue;
        }
        for g in scan_atlauncher(&root, &cmd)? {
            push(g, &mut out);
        }
    }
    Ok(out)
}

fn read_ini(p: &Path) -> Option<HashMap<String, String>> {
    super::util::read_text(p).ok().map(|t| qt_ini::parse(&t))
}

/// A folder setting: absolute, or relative to `root`; `default` when unset.
fn dir_setting(cfg: &HashMap<String, String>, key: &str, root: &Path, default: &str) -> PathBuf {
    match cfg.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
        Some(p) if p.starts_with('/') => PathBuf::from(p),
        Some(p) => root.join(p),
        None => root.join(default),
    }
}

/// Instance folders: directories not starting with `.` or `_` (`.LAUNCHER_TEMP`, `_MMC_TEMP`).
fn instance_dirs(dir: &Path) -> anyhow::Result<Vec<(String, PathBuf)>> {
    let rd = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e).with_context(|| format!("reading {}", dir.display())),
    };
    let mut v: Vec<(String, PathBuf)> = rd
        .flatten()
        .take(MAX_INSTANCES * 2)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            (!name.starts_with('.') && !name.starts_with('_')).then(|| (name, e.path()))
        })
        .collect();
    v.sort();
    v.truncate(MAX_INSTANCES);
    Ok(v)
}

fn scan_mmc(l: &Mmc) -> anyhow::Result<Vec<ScannedGame>> {
    let cfg = read_ini(&l.root.join(l.cfg)).unwrap_or_default();
    let instances = dir_setting(&cfg, "InstanceDir", &l.root, "instances");
    let icons = dir_setting(&cfg, "IconsDir", &l.root, "icons");
    let mut out = vec![];
    for (id, dir) in instance_dirs(&instances)? {
        let Some(inst) = read_ini(&dir.join("instance.cfg")) else {
            continue;
        };
        let get = |k: &str| inst.get(k).map(|s| s.trim()).filter(|s| !s.is_empty());
        let name = get("name").unwrap_or(&id);
        let mut g = ScannedGame::new(Source::Minecraft, format!("{}:{id}", l.key), format!("Minecraft: {name}"));
        let mut args = l.cmd[1..].to_vec();
        args.extend(["-l".into(), id.clone()]);
        g.launch = Some(LaunchSpec::Command { program: l.cmd[0].clone(), args, cwd: None, env: vec![] });
        g.install_dir = Some(dir.clone());
        g.platform = Some("linux".into());
        g.playtime_secs = get("totalTimePlayed").and_then(|s| s.parse::<u64>().ok()).filter(|&s| s > 0);
        g.last_played =
            get("lastLaunchTime").and_then(|s| s.parse::<i64>().ok()).filter(|&ms| ms > 0).map(|ms| ms / 1000);
        if let Some(key) = get("iconKey").filter(|k| !k.contains('/') && !k.contains("..")) {
            g.art.icon = super::util::image_with_stem(&icons, key);
        }
        g.meta.developer = Some("Mojang Studios".into());
        g.meta.description = mmc_pack_description(&dir.join("mmc-pack.json"));
        out.push(g);
    }
    Ok(out)
}

/// "Minecraft 1.20.1 with Fabric Loader 0.15.3" from `mmc-pack.json`.
fn mmc_pack_description(p: &Path) -> Option<String> {
    let v: Value = serde_json::from_str(&super::util::read_text(p).ok()?).ok()?;
    let comps = v.get("components")?.as_array()?;
    let version = |uid: &str| {
        comps
            .iter()
            .find(|c| c.get("uid").and_then(Value::as_str) == Some(uid))
            .and_then(|c| c.get("version").and_then(Value::as_str))
    };
    let mc = version("net.minecraft")?;
    let loader = [
        ("net.fabricmc.fabric-loader", "Fabric Loader"),
        ("org.quiltmc.quilt-loader", "Quilt Loader"),
        ("net.neoforged", "NeoForge"),
        ("net.minecraftforge", "Forge"),
    ]
    .into_iter()
    .find_map(|(uid, label)| version(uid).map(|v| format!("{label} {v}")));
    Some(match loader {
        Some(l) => format!("Minecraft {mc} with {l}"),
        None => format!("Minecraft {mc}"),
    })
}

// ---------------------------------------------------------------------------------------------
// ATLauncher

fn atlauncher_roots(ctx: &ScanContext) -> Vec<(PathBuf, Vec<String>)> {
    let data = ctx.data_home();
    let fp = ctx.flatpak_app(ATLAUNCHER_FLATPAK).join("data");
    let mut v: Vec<(PathBuf, Vec<String>)> = vec![];
    for root in [data.join("ATLauncher"), data.join("atlauncher"), ctx.home.join("ATLauncher")] {
        let jar = root.join("ATLauncher.jar");
        let cmd = if jar.is_file() {
            vec!["java".into(), "-jar".into(), jar.to_string_lossy().into_owned()]
        } else {
            sv(&["atlauncher"])
        };
        v.push((root, cmd));
    }
    for root in [fp.join("ATLauncher"), fp.join("atlauncher"), fp] {
        v.push((root, sv(&["flatpak", "run", ATLAUNCHER_FLATPAK])));
    }
    v
}

fn scan_atlauncher(root: &Path, cmd: &[String]) -> anyhow::Result<Vec<ScannedGame>> {
    let mut out = vec![];
    for (dir_name, dir) in instance_dirs(&root.join("instances"))? {
        let Ok(text) = super::util::read_text(dir.join("instance.json")) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<Value>(&text) else {
            log::warn!("minecraft: unreadable {}", dir.join("instance.json").display());
            continue;
        };
        let l = v.get("launcher").unwrap_or(&Value::Null);
        let name = l.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).unwrap_or(&dir_name);
        let mut g = ScannedGame::new(Source::Minecraft, format!("atlauncher:{dir_name}"), format!("Minecraft: {name}"));
        let mut args = cmd[1..].to_vec();
        args.extend(["--launch".into(), name.to_string()]);
        g.launch = Some(LaunchSpec::Command { program: cmd[0].clone(), args, cwd: None, env: vec![] });
        g.install_dir = Some(dir.clone());
        g.platform = Some("linux".into());
        g.playtime_secs = l.get("totalPlayTime").and_then(Value::as_u64).filter(|&s| s > 0);
        g.last_played = l.get("lastPlayed").and_then(timestamp);
        g.art.icon = super::util::file_if_exists(dir.join("instance.png"));
        g.meta.developer = Some("Mojang Studios".into());
        if let Some(mc) = v.get("id").and_then(Value::as_str) {
            let loader = l.pointer("/loaderVersion/type").and_then(Value::as_str).filter(|s| !s.is_empty());
            let pack = l.get("pack").and_then(Value::as_str).filter(|p| !p.is_empty() && *p != name);
            let mut d = format!("Minecraft {mc}");
            if let Some(lo) = loader {
                d.push_str(&format!(" with {lo}"));
            }
            if let Some(p) = pack {
                d.push_str(&format!(" ({p} modpack)"));
            }
            g.meta.description = Some(d);
        }
        out.push(g);
    }
    Ok(out)
}

/// A timestamp as unix seconds: a number (ms or s) or an ISO date-time string.
fn timestamp(v: &Value) -> Option<i64> {
    let t = match v {
        Value::Number(n) => {
            let n = n.as_i64()?;
            if n > 100_000_000_000 { n / 1000 } else { n }
        }
        Value::String(s) => super::desktop::common::parse_datetime(s, false)?,
        _ => return None,
    };
    (t > 0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scan::desktop::test_support::{copy_tree, ctx, fake_png};

    fn fixture(rel: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stores/minecraft").join(rel)
    }

    fn cmd(program: &str, args: &[&str]) -> Option<LaunchSpec> {
        Some(LaunchSpec::Command { program: program.into(), args: sv(args), cwd: None, env: vec![] })
    }

    #[test]
    fn prism_instances() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let root = home.join(".local/share/PrismLauncher");
        copy_tree(&fixture("PrismLauncher"), &root);
        std::fs::create_dir_all(root.join("instances/.LAUNCHER_TEMP")).unwrap();
        std::fs::create_dir_all(root.join("instances/_MMC_TEMP")).unwrap();
        fake_png(&root.join("icons/fabulously-optimized.png"), 128, 128);

        let games = scan(&ctx(home)).unwrap();
        let ids: Vec<&str> = games.iter().map(|g| g.source_id.as_str()).collect();
        assert_eq!(ids, vec!["prism:1.20.1 Fabric", "prism:Vanilla"], "{games:#?}");
        let fo = &games[0];
        assert_eq!(fo.name, "Minecraft: Fabulously Optimized, 1.20");
        assert_eq!(fo.launch, cmd("prismlauncher", &["-l", "1.20.1 Fabric"]));
        assert_eq!(fo.playtime_secs, Some(45_296));
        assert_eq!(fo.last_played, Some(1_731_000_000));
        assert_eq!(fo.art.icon, Some(root.join("icons/fabulously-optimized.png")));
        assert_eq!(fo.meta.description.as_deref(), Some("Minecraft 1.20.1 with Fabric Loader 0.15.11"));
        assert_eq!(fo.install_dir, Some(root.join("instances/1.20.1 Fabric")));
        let van = &games[1];
        assert_eq!(van.name, "Minecraft: Vanilla");
        assert_eq!(van.art.icon, None, "built-in icon");
        assert_eq!(van.playtime_secs, None);
    }

    #[test]
    fn custom_instance_dir_flatpak_multimc_atlauncher() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        // Flatpak Prism with instances in a custom absolute folder.
        let root = home.join(".var/app").join(PRISM_FLATPAK).join("data/PrismLauncher");
        let elsewhere = home.join("Minecraft instances");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("prismlauncher.cfg"),
            format!("[General]\nInstanceDir={}\nIconsDir=icons\n", elsewhere.display()),
        )
        .unwrap();
        copy_tree(&fixture("PrismLauncher/instances/Vanilla"), &elsewhere.join("Vanilla"));
        // MultiMC with an old section-less instance.cfg and its own binary.
        let mmc = home.join(".local/share/multimc");
        copy_tree(&fixture("multimc"), &mmc);
        std::fs::write(mmc.join("MultiMC"), "#!/bin/sh\n").unwrap();
        // ATLauncher.
        let atl = home.join(".local/share/ATLauncher");
        copy_tree(&fixture("ATLauncher"), &atl);
        fake_png(&atl.join("instances/AllTheMods9/instance.png"), 64, 64);

        let games = scan(&ctx(home)).unwrap();
        let ids: Vec<&str> = games.iter().map(|g| g.source_id.as_str()).collect();
        assert_eq!(ids, vec!["prism:Vanilla", "multimc:old", "atlauncher:AllTheMods9"], "{games:#?}");
        assert_eq!(games[0].launch, cmd("flatpak", &["run", PRISM_FLATPAK, "-l", "Vanilla"]));
        assert_eq!(games[1].name, "Minecraft: Old Survival");
        assert_eq!(games[1].launch, cmd(&mmc.join("MultiMC").to_string_lossy(), &["-l", "old"]));
        let atm = &games[2];
        assert_eq!(atm.name, "Minecraft: All the Mods 9");
        assert_eq!(atm.launch, cmd("atlauncher", &["--launch", "All the Mods 9"]));
        assert_eq!(atm.playtime_secs, Some(7200));
        assert_eq!(atm.last_played, Some(1_730_000_000));
        assert!(atm.art.icon.is_some());
        assert_eq!(
            atm.meta.description.as_deref(),
            Some("Minecraft 1.20.1 with Forge (All the Mods 9 - ATM9 modpack)")
        );
    }

    /// Prism as an AppImage: installed by Flatpak Lutris (data in Lutris' sandbox, as on a real
    /// machine), and a portable one in ~/Applications.
    #[test]
    fn prism_appimage_lutris_and_portable() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let lutris = home.join(".var/app").join(super::super::lutris::FLATPAK_ID);
        let games = lutris.join("data/lutris/games");
        std::fs::create_dir_all(&games).unwrap();
        let yml = std::fs::read_to_string(fixture("lutris/prism-launcher-appimage-1795300561.yml"))
            .unwrap()
            .replace("/HOME", &home.to_string_lossy());
        std::fs::write(games.join("prism-launcher-appimage-1795300561.yml"), yml).unwrap();
        let appimage = home.join("Games/prism-launcher/PrismLauncher-Linux-x86_64.AppImage");
        std::fs::create_dir_all(appimage.parent().unwrap()).unwrap();
        std::fs::write(&appimage, "AI").unwrap();
        let sandbox = lutris.join("data/PrismLauncher");
        copy_tree(&fixture("PrismLauncher"), &sandbox);
        // Portable AppImage: portable.txt + instances next to it.
        let apps = home.join("Applications");
        let portable = apps.join("PrismLauncher-9.4.AppImage");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(&portable, "AI").unwrap();
        std::fs::write(apps.join("portable.txt"), "").unwrap();
        copy_tree(&fixture("PrismLauncher/instances/Vanilla"), &apps.join("instances/Portable One"));
        // Not an AppImage of Prism.
        std::fs::write(apps.join("Other.AppImage"), "AI").unwrap();

        let found = scan(&ctx(home)).unwrap();
        let ids: Vec<&str> = found.iter().map(|g| g.source_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["prism-appimage:1.20.1 Fabric", "prism-appimage:Vanilla", "prism-appimage:Portable One"],
            "{found:#?}"
        );
        let a = appimage.to_string_lossy();
        let sb = sandbox.to_string_lossy();
        assert_eq!(found[1].launch, cmd(&a, &["--dir", &sb, "-l", "Vanilla"]));
        assert_eq!(found[1].install_dir, Some(sandbox.join("instances/Vanilla")));
        assert_eq!(
            found[2].launch,
            cmd(&portable.to_string_lossy(), &["--dir", &apps.to_string_lossy(), "-l", "Portable One"])
        );
    }

    #[test]
    fn nothing_installed() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(scan(&ctx(tmp.path())).unwrap().is_empty());
        // A launcher folder without instances yet.
        std::fs::create_dir_all(tmp.path().join(".local/share/PrismLauncher")).unwrap();
        assert!(scan(&ctx(tmp.path())).unwrap().is_empty());
    }
}
