//! Installing a theme from a list (download → convert) and importing one from a folder or
//! file, all ending in the same plain-words report. Blocking: run on a helper thread.

use super::{Entry, Index, Kind, Record, net, pnite, write_record};
use crate::theme::playnite;
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Shared with the UI while a job runs.
#[derive(Default)]
pub struct Progress {
    /// Bytes downloaded so far.
    pub bytes: AtomicU64,
    pub cancel: AtomicBool,
    /// What's happening ("Downloading…", "Converting…").
    pub stage: Mutex<String>,
}

impl Progress {
    fn set(&self, s: &str) {
        *self.stage.lock() = s.to_string();
    }
}

/// What an install / import did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// Theme key (folder name).
    pub key: String,
    pub name: String,
    /// Desktop (Playnite desktop) or controller (Playnite fullscreen).
    pub controller: bool,
    /// "Playnite fullscreen theme" …
    pub what: String,
    pub version: String,
    pub carried: Vec<String>,
    pub missed: Vec<String>,
    /// Came from a community list (shows the "may not look exactly like the original" note).
    pub community: bool,
    /// Something this computer lacks for part of the theme (ffmpeg for its videos): what to
    /// install, and the command to copy when there is one.
    pub needs: Vec<crate::media::deps::Hint>,
    /// The report by area (Playnite imports; empty: show `carried` / `missed`).
    pub groups: Vec<playnite::Group>,
    /// Playnite imports: what's needed to import it again with other choices.
    pub playnite: Option<Rerun>,
}

/// A Playnite import's choices, to run it again from the report ("Looks wrong? Try: …",
/// "Include its videos and sounds").
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rerun {
    /// The package / folder it came from.
    pub source: PathBuf,
    /// `source` is a download kept only while the report is open (removed by `discard`).
    pub temporary: bool,
    pub family: Option<playnite::Family>,
    pub suggested: Option<playnite::Family>,
    pub media_bytes: u64,
    pub media_included: bool,
    /// A fullscreen theme (the layout choices apply to it).
    pub fullscreen: bool,
    /// The theme's own screens could be translated (false: "Translated" can't be chosen).
    pub translatable: bool,
}

impl Rerun {
    /// The layouts worth offering to import it again with: all, but "Translated" only when
    /// the theme's own screens could be translated.
    pub fn offers(&self, f: playnite::Family) -> bool {
        f != playnite::Family::Translated || self.translatable
    }
}

/// A fresh work folder (removed on drop).
struct Work(PathBuf);

impl Work {
    fn new(parent: &Path, tag: &str) -> anyhow::Result<Work> {
        use std::sync::atomic::AtomicU32;
        static N: AtomicU32 = AtomicU32::new(0);
        let p = parent.join(format!("work-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p)?;
        Ok(Work(p))
    }
}

impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// File-name-safe version of an id.
fn file_safe(s: &str) -> String {
    let s: String =
        s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(80).collect();
    if s.is_empty() { "theme".into() } else { s }
}

/// How a theme's videos or sounds and music play, in words that are true wherever the report
/// is read: on by default, changed in controller mode (Settings → Visuals / Audio) or on the desktop
/// (Settings → Controller → More options).
/// `plural`: "videos play" / "music plays".
pub(crate) fn media_line(what: &str, plural: bool, video: bool) -> String {
    let play = if plural { "play" } else { "plays" };
    if video {
        format!(
            "{what} {play} automatically in controller mode when ffmpeg is installed, else {} \
             (change it in Settings → Controller → More options, or in controller mode: Settings → Visuals)",
            if plural { "their still pictures show" } else { "its still picture shows" },
        )
    } else {
        format!(
            "{what} {play} automatically in controller mode (change it in Settings → Controller → More options, or in controller mode: Settings → Audio)"
        )
    }
}

/// What to tell about a missing program, for this computer's Linux family.
fn missing(need: crate::media::deps::Need) -> crate::media::deps::Hint {
    let mut h = crate::media::deps::hint(crate::media::deps::family(), need);
    let lead = match need {
        crate::media::deps::Need::Video => {
            "Its videos need ffmpeg, which isn't installed (the still pictures show meanwhile)."
        }
        crate::media::deps::Need::Audio => "Its sounds need a sound player (pw-cat, pacat or aplay); none was found.",
    };
    h.text = format!("{lead} {}", h.text);
    h
}

fn playnite_report(rep: &playnite::ImportReport, community: bool, temporary: bool) -> Report {
    let fullscreen = rep.mode == playnite::Mode::Fullscreen;
    let mut needs = vec![];
    if rep.needs_video {
        needs.push(missing(crate::media::deps::Need::Video));
    }
    if rep.needs_audio {
        needs.push(missing(crate::media::deps::Need::Audio));
    }
    Report {
        key: rep.key.clone(),
        name: rep.name.clone(),
        controller: fullscreen,
        what: format!("Playnite {} theme", rep.mode.label().to_lowercase()),
        version: rep.version.clone(),
        carried: rep.carried.clone(),
        missed: rep.missed.clone(),
        community,
        needs,
        groups: rep.groups.clone(),
        playnite: Some(Rerun {
            source: rep.source_path.clone(),
            temporary,
            family: rep.family,
            suggested: rep.suggested,
            media_bytes: rep.media_bytes,
            media_included: rep.media_included,
            fullscreen,
            translatable: rep.translatable,
        }),
    }
}

/// Where downloaded packages wait while their report is open (one at a time).
fn kept_dir(work_dir: &Path) -> PathBuf {
    work_dir.join("kept")
}

/// Import a Playnite theme again with other choices (from its report). Keeps the theme's
/// install record (where it came from) and the report's wording.
pub fn reimport_playnite(
    prev: &Report,
    opts: &playnite::ImportOptions,
    themes_dir: &Path,
    p: &Progress,
) -> anyhow::Result<Report> {
    let rerun =
        prev.playnite.as_ref().ok_or_else(|| anyhow::anyhow!("only Playnite themes can be imported again this way"))?;
    if !rerun.source.exists() {
        anyhow::bail!(
            "the theme's package isn't there any more ({}); install or import it again",
            rerun.source.display()
        );
    }
    p.set("Converting…");
    let record = super::read_record(&themes_dir.join(&prev.key));
    let rep = playnite::import_with(&rerun.source, themes_dir, opts)?;
    if let Some(r) = record {
        write_record(&rep.dir, &r)?;
    }
    let mut out = playnite_report(&rep, prev.community, rerun.temporary);
    if prev.community {
        out.version = prev.version.clone();
    }
    Ok(out)
}

/// The report is closed: a download kept for it goes.
pub fn discard(r: &Report) {
    if let Some(x) = &r.playnite
        && x.temporary
    {
        let _ = std::fs::remove_file(&x.source);
    }
}

/// Install `entry` from its list: newest package → download → convert, then
/// `install.toml` next to the theme. `work_dir` holds the download (removed afterwards).
pub fn install(entry: &Entry, themes_dir: &Path, work_dir: &Path, p: &Progress) -> anyhow::Result<Report> {
    std::fs::create_dir_all(work_dir)?;
    let work = Work::new(work_dir, &file_safe(&entry.id))?;
    let err = |e: String| anyhow::anyhow!(e);
    match entry.index {
        Index::Playnite => {
            p.set("Looking up the newest version…");
            let text = net::get(&entry.source, 1 << 20).map_err(err)?;
            let pkg = pnite::latest_package(&String::from_utf8_lossy(&text))
                .ok_or_else(|| anyhow::anyhow!("the theme has no downloadable release"))?;
            p.set("Downloading…");
            let file = work.0.join(format!("{}.pthm", file_safe(&entry.id)));
            net::download(&pkg.url, &file, net::MAX_ARCHIVE_BYTES, &p.bytes, &p.cancel).map_err(err)?;
            p.set("Converting…");
            // Kept while the report is open, so its choices can be tried without downloading
            // again (one package at a time).
            let kept = kept_dir(work_dir);
            let _ = std::fs::remove_dir_all(&kept);
            let _ = std::fs::create_dir_all(&kept);
            let keep = kept.join(file.file_name().unwrap_or_default());
            let (file, temporary) = if std::fs::rename(&file, &keep).is_ok() { (keep, true) } else { (file, false) };
            let rep = playnite::import(&file, themes_dir)?;
            let want_fullscreen = entry.kind == Kind::Controller;
            if (rep.mode == playnite::Mode::Fullscreen) != want_fullscreen {
                log::info!("Playnite theme {}: list says {:?}, package says {:?}", entry.id, entry.kind, rep.mode);
            }
            write_record(
                &rep.dir,
                &Record {
                    index: Some(Index::Playnite),
                    id: entry.id.clone(),
                    name: rep.name.clone(),
                    version: if pkg.version.is_empty() { rep.version.clone() } else { pkg.version.clone() },
                    page: entry.page.clone(),
                    source: entry.source.clone(),
                    installed: super::now(),
                },
            )?;
            let mut r = playnite_report(&rep, true, temporary);
            if !pkg.version.is_empty() {
                r.version = pkg.version;
            }
            Ok(r)
        }
    }
}

/// Import a Playnite theme package / folder by hand.
pub fn import_playnite(src: &Path, themes_dir: &Path, p: &Progress) -> anyhow::Result<Report> {
    p.set("Converting…");
    let rep = playnite::import(src, themes_dir)?;
    Ok(playnite_report(&rep, false, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translated_is_offered_again_only_when_the_theme_could_be_translated() {
        let mut r = Rerun { fullscreen: true, ..Default::default() };
        assert!(!r.offers(playnite::Family::Translated));
        assert!(r.offers(playnite::Family::Row) && r.offers(playnite::Family::Classic));
        r.translatable = true;
        assert!(r.offers(playnite::Family::Translated));
    }

    #[test]
    fn media_lines_say_they_play_on_their_own_and_where_to_change_that() {
        let v = media_line("Theme videos", true, true);
        assert_eq!(
            v,
            "Theme videos play automatically in controller mode when ffmpeg is installed, else their still pictures show \
             (change it in Settings → Controller → More options, or in controller mode: Settings → Visuals)"
        );
        assert_eq!(
            media_line("The theme's background music", false, false),
            "The theme's background music plays automatically in controller mode (change it in Settings → Controller → More options, or in controller mode: Settings → Audio)"
        );
    }
}
