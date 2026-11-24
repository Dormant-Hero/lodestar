//! Bottles scanner: programs the user added to the Bottles *Library*.
//!
//! Data: `~/.local/share/bottles/library.yml` (native) or
//! `~/.var/app/com.usebottles.bottles/data/bottles/library.yml` (Flatpak). PyYAML-written map:
//! ```yaml
//! <uuid>:
//!   bottle: {name: Gaming, path: Gaming}   # (block style in practice)
//!   id: <program id in the bottle's External_Programs>
//!   name: Program Name
//!   thumbnail: grid:<file>                  # <bottle dir>/grids/<file>
//! ```
//! The bottle's own `bottles/<path>/bottle.yml` (`External_Programs.<id>`) adds the executable
//! path, folder and arguments when present.
//! Launch: `bottles-cli run -b <bottle> -p <program>` (Flatpak: `flatpak run --command=bottles-cli
//! com.usebottles.bottles run …`).
//!
//! CONTRACT: `pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>>` (see scan/mod.rs).

pub mod yaml;

use super::ScanContext;
use super::util::file_if_exists;
use crate::model::*;
use anyhow::Context as _;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use yaml::Yaml;

pub const FLATPAK_ID: &str = "com.usebottles.bottles";

pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    let roots = [(ctx.data_home().join("bottles"), false), (ctx.flatpak_app(FLATPAK_ID).join("data/bottles"), true)];
    let mut out = vec![];
    let mut seen = HashSet::new();
    for (root, flatpak) in roots {
        let lib = root.join("library.yml");
        if !lib.is_file() {
            continue;
        }
        let text = crate::scan::util::read_text(&lib).with_context(|| format!("reading {}", lib.display()))?;
        for g in parse_library(&text, &root, flatpak) {
            if seen.insert(g.source_id.clone()) {
                out.push(g);
            }
        }
    }
    Ok(out)
}

/// Parse `library.yml`. `root` is the Bottles data dir (contains `bottles/`).
pub fn parse_library(text: &str, root: &Path, flatpak: bool) -> Vec<ScannedGame> {
    let doc = yaml::parse(text);
    let mut bottle_cache: HashMap<PathBuf, Yaml> = HashMap::new();
    let mut out = vec![];
    for (uuid, entry) in doc.entries() {
        let Some(name) = entry.get_str("name") else {
            continue;
        };
        if super::clients::is_client_name(name) || entry.get_str("path").is_some_and(super::clients::is_client_exe) {
            continue; // a store client in a bottle: its games come from the Wine-prefix scanner
        }
        let bottle = entry.get("bottle");
        let Some(bottle_name) = bottle.and_then(|b| b.get_str("name")) else {
            continue;
        };
        let bottle_dir = bottle_dir(root, bottle.and_then(|b| b.get_str("path")).unwrap_or(bottle_name));

        let mut g = ScannedGame::new(Source::Bottles, uuid.as_str(), name);
        g.platform = Some("windows".into());
        g.installed = bottle_dir.is_dir();
        let args = vec!["run".into(), "-b".into(), bottle_name.to_string(), "-p".into(), name.to_string()];
        g.launch = Some(if flatpak {
            let mut a = vec!["run".to_string(), "--command=bottles-cli".into(), FLATPAK_ID.into()];
            a.extend(args);
            LaunchSpec::Command { program: "flatpak".into(), args: a, cwd: None, env: vec![] }
        } else {
            LaunchSpec::Command { program: "bottles-cli".into(), args, cwd: None, env: vec![] }
        });

        // Program details from bottle.yml (cached per bottle).
        let cfg = bottle_cache.entry(bottle_dir.clone()).or_insert_with(|| {
            crate::scan::util::read_text(bottle_dir.join("bottle.yml")).map(|t| yaml::parse(&t)).unwrap_or(Yaml::Null)
        });
        let program = cfg.get("External_Programs").and_then(|progs| {
            entry
                .get_str("id")
                .and_then(|id| progs.get(id))
                .or_else(|| progs.entries().iter().map(|(_, p)| p).find(|p| p.get_str("name") == Some(name)))
        });
        let exe = program.and_then(|p| p.get_str("path")).or_else(|| entry.get_str("path"));
        if let Some(folder) = program.and_then(|p| p.get_str("folder")).filter(|f| f.starts_with('/')) {
            g.install_dir = Some(PathBuf::from(folder));
        } else if let Some(exe) = exe.filter(|e| e.starts_with('/')) {
            g.install_dir = Path::new(exe).parent().map(Path::to_path_buf);
        }
        if let Some(exe) = exe.filter(|e| e.starts_with('/')) {
            g.installed &= Path::new(exe).exists();
        }
        g.source_launch_options = program.and_then(|p| p.get_str("arguments")).map(str::to_string);

        if let Some(thumb) = entry.get_str("thumbnail") {
            let file = thumb.strip_prefix("grid:").unwrap_or(thumb);
            if thumb.starts_with('/') {
                g.art.cover = file_if_exists(thumb);
            } else if !file.contains('/') && !file.is_empty() {
                g.art.cover = file_if_exists(bottle_dir.join("grids").join(file));
            }
        }
        out.push(g);
    }
    out
}

fn bottle_dir(root: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if p.is_absolute() { p.to_path_buf() } else { root.join("bottles").join(path) }
}

#[cfg(test)]
mod tests {
    use super::super::desktop::test_support::*;
    use super::*;

    fn setup(root: &Path) {
        copy_tree(&fixture("bottles/bottles"), root);
        let lib = root.join("library.yml");
        let t = std::fs::read_to_string(&lib).unwrap();
        std::fs::write(&lib, t.replace("/ROOT", &root.to_string_lossy())).unwrap();
        let by = root.join("bottles/Gaming/bottle.yml");
        let t = std::fs::read_to_string(&by).unwrap();
        std::fs::write(&by, t.replace("/ROOT", &root.to_string_lossy())).unwrap();
        fake_png(&root.join("bottles/Gaming/grids/Hollow Knight.png"), 600, 900);
        let exe = root.join("bottles/Gaming/drive_c/Program Files (x86)/Hollow Knight/hollow_knight.exe");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(exe, "MZ").unwrap();
    }

    #[test]
    fn parses_fixture() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let root = home.join(".local/share/bottles");
        setup(&root);
        let games = scan(&ctx(home)).unwrap();
        assert_eq!(games.len(), 3, "{games:#?}");
        let hk = games.iter().find(|g| g.name == "Hollow Knight").unwrap();
        assert_eq!(hk.source_id, "6f2a4c0e-8b1d-4f7e-9a3c-2d5e6f7a8b9c");
        assert!(hk.installed);
        assert_eq!(hk.art.cover, Some(root.join("bottles/Gaming/grids/Hollow Knight.png")));
        assert_eq!(hk.install_dir, Some(root.join("bottles/Gaming/drive_c/Program Files (x86)/Hollow Knight")));
        assert_eq!(hk.source_launch_options.as_deref(), Some("-force-d3d11"));
        assert_eq!(
            hk.launch,
            Some(LaunchSpec::Command {
                program: "bottles-cli".into(),
                args: vec!["run".into(), "-b".into(), "Gaming".into(), "-p".into(), "Hollow Knight".into()],
                cwd: None,
                env: vec![]
            })
        );
        let long = games.iter().find(|g| g.name.starts_with("A Game With")).unwrap();
        assert_eq!(long.name, "A Game With A Really Long Name That PyYAML Folds Onto The Next Line: Deluxe");
        assert!(!long.installed, "exe from bottle.yml is missing");
        let other = games.iter().find(|g| g.name == "Notepad++").unwrap();
        assert!(!other.installed, "bottle dir missing");
        assert_eq!(other.art.cover, None);
    }

    #[test]
    fn flatpak_form() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();
        let root = home.join(".var/app").join(FLATPAK_ID).join("data/bottles");
        setup(&root);
        let games = scan(&ctx(home)).unwrap();
        match &games[0].launch {
            Some(LaunchSpec::Command { program, args, .. }) => {
                assert_eq!(program, "flatpak");
                assert_eq!(&args[..3], &["run", "--command=bottles-cli", FLATPAK_ID]);
                assert_eq!(&args[3..5], &["run", "-b"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn empty_and_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(scan(&ctx(tmp.path())).unwrap().is_empty());
        let root = tmp.path().join(".local/share/bottles");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("library.yml"), "{}\n").unwrap();
        assert!(scan(&ctx(tmp.path())).unwrap().is_empty());
    }
}
