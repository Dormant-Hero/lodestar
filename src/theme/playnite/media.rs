//! A Playnite theme's videos and sounds, carried into the Lodestar theme (`crate::media`
//! plays them): the background videos of the Main and GameDetails views
//! (MediaElement) → the background picture element's `video`; `audio/navigation.*` /
//! `audio/activation.*` → `[sounds]`; `audio/background.*` → `music`. Copying them is a
//! choice made at import (they can be big): on by default while they are under
//! `DEFAULT_LIMIT`.

use super::look::{MediaLook, SoundRole, ThemeVideo, VideoRole};
use super::source::Source;
use crate::theme::model::Fit;
use std::collections::BTreeMap;
use std::path::Path;

/// Videos and sounds come along by default up to this much.
pub const DEFAULT_LIMIT: u64 = 150 << 20;
/// Biggest single video copied.
const VIDEO_CAP: u64 = 64 << 20;
const MUSIC_CAP: u64 = 32 << 20;
const SOUND_CAP: u64 = 4 << 20;

/// Lodestar's navigation sounds each Playnite sound plays as.
const NAVIGATION: [&str; 4] = ["scroll", "systembrowse", "quicksysselect", "back"];
const ACTIVATION: [&str; 3] = ["select", "launch", "favorite"];

/// What would be copied.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Plan {
    /// (view, video) — at most one for Main and one for GameDetails.
    pub videos: Vec<ThemeVideo>,
    pub music: Vec<(String, u64)>,
    /// (Lodestar sound names, theme file, bytes).
    pub sounds: Vec<(&'static [&'static str], String, u64)>,
    /// Theme videos left out (too big, or not a background), file names.
    pub left_out: Vec<String>,
    pub bytes: u64,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.videos.is_empty() && self.music.is_empty() && self.sounds.is_empty()
    }
}

pub(crate) fn plan(m: &MediaLook, files: &[(String, u64)]) -> Plan {
    let size = |f: &str| files.iter().find(|(x, _)| x == f).map_or(0, |(_, s)| *s);
    let mut p = Plan::default();
    for v in &m.videos {
        let bytes = if v.bytes > 0 { v.bytes } else { size(&v.file) };
        let used = v.role == VideoRole::Background
            && matches!(v.view.as_str(), "Main" | "GameDetails")
            && !p.videos.iter().any(|x| x.view == v.view)
            && bytes > 0
            && bytes <= VIDEO_CAP;
        if used {
            p.videos.push(ThemeVideo { bytes, ..v.clone() });
            p.bytes += bytes;
        } else {
            p.left_out.push(v.file.clone());
        }
    }
    p.left_out.extend(m.unused_videos.iter().cloned());
    for f in m.music.iter().take(3) {
        let b = size(f);
        if b > 0 && b <= MUSIC_CAP {
            p.music.push((f.clone(), b));
            p.bytes += b;
        }
    }
    for s in &m.sounds {
        let names: &'static [&'static str] = match s.role {
            SoundRole::Navigation => &NAVIGATION,
            SoundRole::Activation => &ACTIVATION,
            SoundRole::Other(_) => continue,
        };
        let b = if s.bytes > 0 { s.bytes } else { size(&s.file) };
        if b > 0 && b <= SOUND_CAP && !p.sounds.iter().any(|(n, _, _)| *n == names) {
            p.sounds.push((names, s.file.clone(), b));
            p.bytes += b;
        }
    }
    p
}

/// What was copied into the new theme (paths relative to it).
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Copied {
    pub main_video: Option<(String, Fit)>,
    pub details_video: Option<(String, Fit)>,
    pub sounds: BTreeMap<String, String>,
    pub music: Vec<String>,
    /// Files that couldn't be read.
    pub failed: Vec<String>,
}

pub(crate) fn dest_name(rel: &str) -> String {
    let base = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
    base.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect()
}

fn fit(v: &ThemeVideo) -> Fit {
    match v.stretch {
        Some(super::look::Stretch::Uniform) => Fit::Contain,
        Some(super::look::Stretch::Fill) => Fit::Stretch,
        _ => Fit::Cover,
    }
}

/// Copy the planned files into `dir` (`media/`).
pub(crate) fn copy(p: &Plan, source: &Source, dir: &Path) -> anyhow::Result<Copied> {
    let mut out = Copied::default();
    if p.is_empty() {
        return Ok(out);
    }
    std::fs::create_dir_all(dir.join("media"))?;
    let put = |rel: &str, limit: u64| -> Option<String> {
        let dest = format!("media/{}", dest_name(rel));
        if dir.join(&dest).exists() {
            return Some(dest);
        }
        source.put(rel, &dir.join(&dest), limit).then_some(dest)
    };
    for v in &p.videos {
        match put(&v.file, VIDEO_CAP) {
            Some(d) if v.view == "GameDetails" => out.details_video = Some((d, fit(v))),
            Some(d) => out.main_video = Some((d, fit(v))),
            None => out.failed.push(v.file.clone()),
        }
    }
    for (f, _) in &p.music {
        match put(f, MUSIC_CAP) {
            Some(d) => out.music.push(d),
            None => out.failed.push(f.clone()),
        }
    }
    for (names, f, _) in &p.sounds {
        match put(f, SOUND_CAP) {
            Some(d) => {
                for n in names.iter() {
                    out.sounds.entry((*n).to_string()).or_insert_with(|| d.clone());
                }
            }
            None => out.failed.push(f.clone()),
        }
    }
    Ok(out)
}

/// Sound players on this computer (pw-cat / pacat / aplay).
pub(crate) fn sound_player_found() -> bool {
    !crate::media::player::detect_all(crate::media::find_tool).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::playnite::look::ThemeSound;

    fn video(file: &str, view: &str, role: VideoRole, bytes: u64) -> ThemeVideo {
        ThemeVideo {
            file: file.into(),
            bytes,
            role,
            view: view.into(),
            repeat: true,
            muted: true,
            stretch: None,
            opacity: None,
        }
    }

    #[test]
    fn plans_one_background_video_per_view_within_the_caps() {
        let m = MediaLook {
            videos: vec![
                video("Videos/bg.mp4", "Main", VideoRole::Background, 10 << 20),
                video("Videos/bg2.mp4", "Main", VideoRole::Background, 5 << 20),
                video("Videos/details.mp4", "GameDetails", VideoRole::Background, 3 << 20),
                video("Videos/badge.mp4", "Main", VideoRole::Other, 1 << 20),
                video("Videos/huge.mp4", "GameDetails", VideoRole::Background, 90 << 20),
            ],
            unused_videos: vec!["Videos/spare.mp4".into()],
            music: vec!["audio/background.mp3".into()],
            sounds: vec![
                ThemeSound { role: SoundRole::Navigation, file: "audio/navigation.wav".into(), bytes: 8000 },
                ThemeSound { role: SoundRole::Activation, file: "audio/activation.wav".into(), bytes: 9000 },
                ThemeSound { role: SoundRole::Other("x".into()), file: "audio/x.wav".into(), bytes: 1 },
            ],
            bytes: 0,
        };
        let files = vec![("audio/background.mp3".to_string(), 4u64 << 20)];
        let p = plan(&m, &files);
        let vids: Vec<&str> = p.videos.iter().map(|v| v.file.as_str()).collect();
        assert_eq!(vids, ["Videos/bg.mp4", "Videos/details.mp4"]);
        assert!(p.left_out.contains(&"Videos/huge.mp4".to_string()));
        assert!(p.left_out.contains(&"Videos/badge.mp4".to_string()));
        assert!(p.left_out.contains(&"Videos/spare.mp4".to_string()));
        assert_eq!(p.music.len(), 1);
        assert_eq!(p.sounds.len(), 2);
        assert_eq!(p.bytes, (10 << 20) + (3 << 20) + (4 << 20) + 8000 + 9000);
    }

    #[test]
    fn copies_and_maps_sounds() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        for (f, d) in [
            ("theme.yaml", "Name: T"),
            ("audio/navigation.wav", "nav"),
            ("audio/activation.ogg", "act"),
            ("Videos/Main BG.mp4", "vid"),
        ] {
            let p = src.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, d).unwrap();
        }
        let source = Source::open(src.path()).unwrap();
        let p = Plan {
            videos: vec![video("Videos/Main BG.mp4", "Main", VideoRole::Background, 3)],
            music: vec![],
            sounds: vec![
                (&NAVIGATION, "audio/navigation.wav".into(), 3),
                (&ACTIVATION, "audio/activation.ogg".into(), 3),
            ],
            left_out: vec![],
            bytes: 9,
        };
        let c = copy(&p, &source, out.path()).unwrap();
        assert_eq!(c.main_video, Some(("media/Main_BG.mp4".into(), Fit::Cover)));
        assert_eq!(c.sounds.get("scroll").map(String::as_str), Some("media/navigation.wav"));
        assert_eq!(c.sounds.get("select").map(String::as_str), Some("media/activation.ogg"));
        assert_eq!(c.sounds.get("back").map(String::as_str), Some("media/navigation.wav"));
        assert_eq!(std::fs::read(out.path().join("media/Main_BG.mp4")).unwrap(), b"vid");
        for k in c.sounds.keys() {
            assert!(crate::media::audio::Sound::from_key(k).is_some(), "{k}");
        }
    }
}
