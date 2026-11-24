//! Theme videos, navigation sounds and background music — without linking any media library.
//!
//! - Videos: the desktop's own `ffmpeg` (found at run time) decodes a theme video into small
//!   picture frames on a pipe; a helper thread hands the newest one to the renderer (video.rs).
//!   No ffmpeg: the element's still picture shows instead.
//! - Sounds and music: decoded in Rust (symphonia: wav / ogg / mp3 / flac), mixed on a helper
//!   thread and written as plain samples to ONE long-running `pw-cat` / `pacat` / `aplay`
//!   (audio.rs, mixer.rs, player.rs), so a button press never waits for a process to start.
//!
//! Everything is off until used: no thread or process runs while sounds, music and videos are
//! off or nothing shows a video.

pub mod audio;
pub mod decode;
pub mod deps;
pub mod mixer;
pub mod player;
pub mod ppm;
pub mod synth;
pub mod video;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// An executable named `name` on `PATH` (first match), like `command -v`.
pub fn find_tool(name: &str) -> Option<PathBuf> {
    find_in(name, std::env::var_os("PATH").as_deref()).or_else(|| {
        // Also the user's own program folders, which a menu-started Lodestar's PATH may lack
        // (SteamOS: a static ffmpeg in ~/.local/bin, see deps.rs).
        let home = std::env::var_os("HOME")?;
        let home = PathBuf::from(home);
        let dirs = std::env::join_paths([home.join(".local/bin"), home.join("bin")]).ok()?;
        find_in(name, Some(&dirs))
    })
}

fn find_in(name: &str, path: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let path = path?;
    std::env::split_paths(path)
        .filter(|d| !d.as_os_str().is_empty())
        .map(|d| d.join(name))
        .find(|p| std::fs::metadata(p).map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false))
}

/// `ffmpeg`, looked up once.
pub fn ffmpeg() -> Option<&'static Path> {
    static F: OnceLock<Option<PathBuf>> = OnceLock::new();
    F.get_or_init(|| find_tool("ffmpeg")).as_deref()
}

/// Screenshot / test hook: `LODESTAR_FAKE_VIDEO=1` plays a generated test picture instead of
/// running ffmpeg (headless runs have no ffmpeg and can't show motion anyway).
pub fn fake_video() -> bool {
    static F: OnceLock<bool> = OnceLock::new();
    *F.get_or_init(|| std::env::var("LODESTAR_FAKE_VIDEO").is_ok_and(|v| !v.is_empty() && v != "0"))
}

/// Theme videos can play (ffmpeg is installed, or the test hook is on).
pub fn videos_available() -> bool {
    fake_video() || ffmpeg().is_some()
}

/// Seconds since the first call (a shared clock for threads).
pub(crate) fn clock() -> f64 {
    static T0: OnceLock<std::time::Instant> = OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f64()
}

/// File types the decoders read.
pub const AUDIO_EXTS: [&str; 6] = ["wav", "ogg", "oga", "mp3", "flac", "opus"];

pub fn ext_in(p: &str, exts: &[&str]) -> bool {
    p.rsplit_once('.').map(|(_, e)| exts.iter().any(|x| e.eq_ignore_ascii_case(x))).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_tools_on_path() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a");
        let b = d.path().join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("ffmpeg"), "").unwrap(); // not executable
        std::fs::write(b.join("ffmpeg"), "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(b.join("ffmpeg"), std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(find_in("ffmpeg", Some(&path)), Some(b.join("ffmpeg")));
        assert_eq!(find_in("pw-cat", Some(&path)), None);
        assert_eq!(find_in("ffmpeg", None), None);
    }

    #[test]
    fn extensions() {
        assert!(ext_in("music/a.OGG", &AUDIO_EXTS));
        assert!(!ext_in("music/a.png", &AUDIO_EXTS));
        assert!(!ext_in("noext", &AUDIO_EXTS));
    }
}
