//! Finding a launcher app however it was installed: a program in PATH, a Flatpak, or an AppImage
//! (in the usual folders, integrated by Gear Lever / AppImageLauncher, or simply running).
//!
//! Store links (`heroic://`, `lutris:`, `itch://`) normally go to the desktop's URL handler. An
//! AppImage that was never integrated registers no handler, so `xdg-open` would fail: then the
//! link is handed to the app itself (`<app> <link>`, exactly what its handler's `Exec=… %u`
//! would run).

use std::path::{Path, PathBuf};

/// A launcher app Lodestar knows how to find.
#[derive(Clone, Copy, Debug)]
pub struct App {
    /// Program names in PATH.
    pub bins: &'static [&'static str],
    pub flatpak: Option<&'static str>,
    /// Lowercase word an AppImage file name must contain (`Heroic-2.18.1.AppImage`).
    pub appimage: Option<&'static str>,
    /// Paths under home the app's own installer uses (`.itch/itch`).
    pub home_bins: &'static [&'static str],
}

pub const HEROIC: App =
    App { bins: &["heroic"], flatpak: Some("com.heroicgameslauncher.hgl"), appimage: Some("heroic"), home_bins: &[] };
pub const LUTRIS: App =
    App { bins: &["lutris"], flatpak: Some("net.lutris.Lutris"), appimage: Some("lutris"), home_bins: &[] };
pub const ITCH: App =
    App { bins: &["itch"], flatpak: Some("io.itch.itch"), appimage: Some("itch"), home_bins: &[".itch/itch"] };
pub const MOONLIGHT: App = App {
    bins: &["moonlight", "moonlight-qt"],
    flatpak: Some("com.moonlight_stream.Moonlight"),
    appimage: Some("moonlight"),
    home_bins: &[],
};

/// The app behind a URL scheme.
pub fn app_for_scheme(scheme: &str) -> Option<App> {
    Some(match scheme.to_ascii_lowercase().as_str() {
        "heroic" => HEROIC,
        "lutris" => LUTRIS,
        "itch" | "itchio" => ITCH,
        _ => return None,
    })
}

/// `heroic://launch?…` → `heroic`.
pub fn scheme_of(url: &str) -> Option<&str> {
    let (s, _) = url.trim().split_once(':')?;
    (!s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))).then_some(s)
}

/// Folders AppImages are usually kept in (Gear Lever: `~/AppImages`; AppImageLauncher:
/// `~/Applications`).
const APPIMAGE_DIRS: &[&str] = &[
    "Applications",
    "AppImages",
    "appimages",
    ".local/bin",
    "bin",
    ".local/share/AppImage",
    "Games",
    "Downloads",
    "Desktop",
    "opt",
];

fn is_exec(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

fn appimage_name_matches(p: &Path, word: &str) -> bool {
    p.file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .is_some_and(|n| n.ends_with(".appimage") && n.contains(word))
}

/// AppImages whose file name contains `word`, newest first: the usual folders, `/opt`, and the
/// targets of desktop entries (an integrated AppImage can live anywhere). Only runnable ones.
pub fn appimages(home: &Path, word: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = vec![];
    let mut add = |p: PathBuf| {
        if appimage_name_matches(&p, word) && is_exec(&p) && !found.contains(&p) {
            found.push(p);
        }
    };
    let mut dirs: Vec<PathBuf> = APPIMAGE_DIRS.iter().map(|d| home.join(d)).collect();
    dirs.push(PathBuf::from("/opt"));
    for dir in &dirs {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.flatten().take(3000) {
            let p = e.path();
            add(p.clone());
            // `/opt/heroic/Heroic.AppImage`, `~/Applications/Heroic/Heroic.AppImage`.
            if p.is_dir()
                && p.file_name().is_some_and(|n| n.to_string_lossy().to_ascii_lowercase().contains(word))
                && let Ok(inner) = std::fs::read_dir(&p)
            {
                for f in inner.flatten().take(200) {
                    add(f.path());
                }
            }
        }
    }
    for exec in desktop_execs(&home.join(".local/share/applications")) {
        add(PathBuf::from(exec));
    }
    found.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    found
}

/// First program word of every desktop entry's `Exec=` in `dir`.
fn desktop_execs(dir: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut out = vec![];
    for e in rd.flatten().take(3000) {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "desktop") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        if let Some(prog) = text.lines().find_map(|l| l.strip_prefix("Exec=")).and_then(exec_program) {
            out.push(prog);
        }
    }
    out
}

/// The program an `Exec=` line runs (skipping `env VAR=x`; quotes honoured).
fn exec_program(exec: &str) -> Option<String> {
    let words = crate::scan::desktop::parse_exec(exec);
    let mut it = words.into_iter().peekable();
    if it.peek().is_some_and(|w| w == "env" || w.ends_with("/env")) {
        it.next();
        while it.peek().is_some_and(|w| w.contains('=') && !w.starts_with('/')) {
            it.next();
        }
    }
    it.next()
}

/// An AppImage of `word` that is running now (its `APPIMAGE` variable): the install the user
/// actually uses wins over copies lying around.
pub fn running_appimage(word: &str) -> Option<PathBuf> {
    let rd = std::fs::read_dir("/proc").ok()?;
    for e in rd.flatten() {
        let name = e.file_name();
        if !name.to_string_lossy().bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }
        let Ok(env) = std::fs::read(e.path().join("environ")) else { continue };
        for (k, v) in crate::procfs::env_entries(&env) {
            if k == b"APPIMAGE" {
                let p = PathBuf::from(String::from_utf8_lossy(v).into_owned());
                if appimage_name_matches(&p, word) && is_exec(&p) {
                    return Some(p);
                }
            }
        }
    }
    None
}

fn flatpak_installed(home: &Path, id: &str) -> bool {
    crate::scan::desktop::common::flatpak_app_installed(home, id) && crate::launch::find_in_path("flatpak").is_some()
}

/// The command that starts `app` (program, leading args): PATH, then the app's own installer
/// location, then a running AppImage, then Flatpak, then an AppImage on disk.
pub fn command(app: &App, home: &Path) -> Option<(String, Vec<String>)> {
    for b in app.bins {
        if crate::launch::find_in_path(b).is_some() {
            return Some((b.to_string(), vec![]));
        }
    }
    for b in app.home_bins {
        let p = home.join(b);
        if is_exec(&p) {
            return Some((p.to_string_lossy().into_owned(), vec![]));
        }
    }
    if let Some(word) = app.appimage
        && let Some(p) = running_appimage(word)
    {
        return Some((p.to_string_lossy().into_owned(), vec![]));
    }
    if let Some(id) = app.flatpak
        && flatpak_installed(home, id)
    {
        return Some(("flatpak".into(), vec!["run".into(), id.into()]));
    }
    let word = app.appimage?;
    appimages(home, word).into_iter().next().map(|p| (p.to_string_lossy().into_owned(), vec![]))
}

/// Desktop app folders, user first.
fn app_dirs(home: &Path) -> Vec<PathBuf> {
    let mut v =
        vec![home.join(".local/share/applications"), home.join(".local/share/flatpak/exports/share/applications")];
    let xdg = std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
    for d in xdg.split(':').filter(|d| !d.is_empty()) {
        v.push(Path::new(d).join("applications"));
    }
    for d in ["/var/lib/flatpak/exports/share/applications", "/usr/share/applications"] {
        let d = PathBuf::from(d);
        if !v.contains(&d) {
            v.push(d);
        }
    }
    v
}

/// Some installed desktop entry handles `scheme` links and its program exists (a handler whose
/// AppImage was moved or deleted doesn't count).
pub fn scheme_handled(home: &Path, scheme: &str) -> bool {
    let mime = format!("x-scheme-handler/{}", scheme.to_ascii_lowercase());
    for dir in app_dirs(home) {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten().take(5000) {
            let p = e.path();
            if p.extension().is_none_or(|x| x != "desktop") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&p) else { continue };
            let handles = text.lines().any(|l| {
                l.strip_prefix("MimeType=").is_some_and(|m| m.split(';').any(|t| t.trim().eq_ignore_ascii_case(&mime)))
            });
            if !handles {
                continue;
            }
            let runnable = text
                .lines()
                .find_map(|l| l.strip_prefix("Exec="))
                .and_then(exec_program)
                .is_some_and(|prog| crate::launch::find_in_path(&prog).is_some());
            if runnable {
                return true;
            }
        }
    }
    false
}

/// For a link nobody handles: the app's own command with the link appended.
pub fn direct_url_command(home: &Path, url: &str) -> Option<(String, Vec<String>)> {
    let scheme = scheme_of(url)?;
    let app = app_for_scheme(scheme)?;
    if scheme_handled(home, scheme) {
        return None;
    }
    let (program, mut args) = command(&app, home)?;
    args.push(url.to_string());
    Some((program, args))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::write_exe;

    #[test]
    fn schemes() {
        assert_eq!(scheme_of("heroic://launch?appName=x"), Some("heroic"));
        assert_eq!(scheme_of("lutris:rungameid/3"), Some("lutris"));
        assert_eq!(scheme_of("/not/a/url"), None);
        assert!(app_for_scheme("HEROIC").is_some());
        assert!(app_for_scheme("steam").is_none());
    }

    #[test]
    fn appimages_in_usual_folders_and_desktop_entries() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        write_exe(&home.join("Applications/Heroic-2.18.1-linux-x86_64.AppImage"), "#!/bin/sh\n");
        write_exe(&home.join("Somewhere/odd/heroic-integrated.AppImage"), "#!/bin/sh\n");
        // Not runnable (a browser download): skipped.
        std::fs::create_dir_all(home.join("Downloads")).unwrap();
        std::fs::write(home.join("Downloads/Heroic-old.AppImage"), "x").unwrap();
        write_exe(&home.join("AppImages/Moonlight-6.1.0-x86_64.AppImage"), "#!/bin/sh\n");
        let apps = home.join(".local/share/applications");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(
            apps.join("appimagekit_abc-Heroic.desktop"),
            format!(
                "[Desktop Entry]\nName=Heroic\nExec=env DESKTOPINTEGRATION=1 \"{}\" %U\n",
                home.join("Somewhere/odd/heroic-integrated.AppImage").display()
            ),
        )
        .unwrap();
        let found = appimages(home, "heroic");
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|p| is_exec(p)));
        assert_eq!(appimages(home, "moonlight").len(), 1);
        assert!(appimages(home, "lutris").is_empty());
    }

    #[test]
    fn exec_programs() {
        assert_eq!(exec_program("\"/a b/Heroic.AppImage\" %u").as_deref(), Some("/a b/Heroic.AppImage"));
        assert_eq!(exec_program("env A=1 B=2 /x/y %U").as_deref(), Some("/x/y"));
        assert_eq!(exec_program("/usr/bin/env A=1 heroic").as_deref(), Some("heroic"));
    }

    #[test]
    fn registered_handlers_need_a_program_that_exists() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path();
        let apps = home.join(".local/share/applications");
        std::fs::create_dir_all(&apps).unwrap();
        // A handler whose AppImage is gone.
        std::fs::write(
            apps.join("heroic.desktop"),
            "[Desktop Entry]\nExec=/gone/Heroic.AppImage %u\nMimeType=x-scheme-handler/heroic;\n",
        )
        .unwrap();
        assert!(!scheme_handled(home, "lodestar-test-scheme"));
        let ai = home.join("Applications/Heroic.AppImage");
        write_exe(&ai, "#!/bin/sh\n");
        // Not handled by anything that exists → the AppImage gets the link itself
        // (unless this machine has Heroic in PATH / a real handler).
        if !scheme_handled(home, "heroic") && crate::launch::find_in_path("heroic").is_none() {
            let (p, a) = direct_url_command(home, "heroic://launch?appName=x").unwrap();
            assert!(p == ai.to_string_lossy() || p == "flatpak" || p.ends_with(".AppImage"), "{p}");
            assert_eq!(a.last().map(String::as_str), Some("heroic://launch?appName=x"));
        }
        std::fs::write(
            apps.join("heroic.desktop"),
            format!("[Desktop Entry]\nExec={} %u\nMimeType=x-scheme-handler/heroic;\n", ai.display()),
        )
        .unwrap();
        assert!(scheme_handled(home, "heroic"));
        assert!(direct_url_command(home, "heroic://launch?appName=x").is_none());
    }
}
