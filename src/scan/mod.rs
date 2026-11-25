//! Game discovery. Each source has a `Scanner`; `run_all` runs them in parallel threads,
//! removes desktop-entry duplicates of games other sources already found, and returns one report.
//!
//! CONTRACT for scanner modules:
//!   * `pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>>`
//!   * Never panic, never block on the network, never prompt. Missing launcher = `Ok(vec![])`.
//!   * Return `Err` only when the launcher's data exists but could not be read (so the DB keeps
//!     the previous state of that source instead of marking everything uninstalled).
//!   * All paths absolute. Resolve art to files that exist.
//!   * Must be fast: no recursive walks of game directories.

use crate::config::{Config, ScanConfig};
use crate::model::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub mod addons;
pub mod bottles;
pub mod clients;
pub mod desktop;
pub mod emulation;
pub mod faugus;
pub mod heroic;
pub mod itch;
pub mod lutris;
pub mod minecraft;
pub mod moonlight;
pub mod romm;
pub mod steam;
pub mod vdf;
pub mod wine_launchers;
pub mod winprefix;

#[cfg(test)]
mod hostile_tests;
#[cfg(test)]
mod integration_tests;

/// Everything a scanner may need. Built from config; tests point `home` at a fixture tree.
#[derive(Clone, Debug)]
pub struct ScanContext {
    /// User home (fixtures override this). Use `ctx.home.join(".local/share/...")`, never `dirs::`.
    pub home: PathBuf,
    /// `$XDG_DATA_DIRS` entries (system app dirs such as /usr/share, /var/lib/flatpak/exports/share).
    pub xdg_data_dirs: Vec<PathBuf>,
    pub config: ScanConfig,
    /// Lodestar's data folder (add-ons live in `<data_dir>/addons`).
    pub data_dir: PathBuf,
    /// Add-on settings / enabled list (for the add-ons scanner).
    pub addons: crate::config::AddonsConfig,
    /// Emulation settings (emulators, ROM folders; for the emulation scanner and registry).
    pub emulation: crate::config::EmulationConfig,
    /// Lodestar's cache of Steam app names / owned games, and what the Steam scanner found
    /// missing (see scan/steam/names.rs).
    pub steam_names: steam::names::Shared,
}

impl ScanContext {
    pub fn from_config(cfg: &Config) -> ScanContext {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let mut xdg: Vec<PathBuf> = std::env::var("XDG_DATA_DIRS")
            .unwrap_or_else(|_| "/usr/local/share:/usr/share".into())
            .split(':')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect();
        for extra in [PathBuf::from("/var/lib/flatpak/exports/share"), PathBuf::from("/usr/share")] {
            if !xdg.contains(&extra) {
                xdg.push(extra);
            }
        }
        ScanContext {
            home,
            xdg_data_dirs: xdg,
            config: cfg.scan.clone(),
            data_dir: crate::config::Paths::detect().data_dir,
            addons: cfg.addons.clone(),
            emulation: cfg.emulation.clone(),
            steam_names: steam::names::Shared::new(crate::config::Paths::detect().cache_dir),
        }
    }

    /// `$XDG_DATA_HOME` equivalent under `home`.
    pub fn data_home(&self) -> PathBuf {
        self.home.join(".local/share")
    }
    pub fn config_home(&self) -> PathBuf {
        self.home.join(".config")
    }
    /// Flatpak per-app data root: `~/.var/app/<app_id>`.
    pub fn flatpak_app(&self, app_id: &str) -> PathBuf {
        self.home.join(".var/app").join(app_id)
    }
}

#[derive(Clone, Debug, Default)]
pub struct SourceReport {
    pub found: usize,
    pub error: Option<String>,
    pub elapsed: Duration,
}

#[derive(Clone, Debug, Default)]
pub struct ScanReport {
    pub games: Vec<ScannedGame>,
    /// Sources whose scanner succeeded (pass to `Db::merge_scan`).
    pub ok_sources: Vec<Source>,
    pub per_source: Vec<(Source, SourceReport)>,
    pub duplicates_dropped: usize,
    pub elapsed: Duration,
    /// Steam apps the scanner couldn't name (for the background name lookup).
    pub steam_needs: steam::names::Needs,
}

type ScanFn = fn(&ScanContext) -> anyhow::Result<Vec<ScannedGame>>;

/// A scanner and the sources it reports (one scanner may report several stores, e.g. Heroic →
/// Epic / GOG / Amazon / sideloaded, or the Wine-prefix scanner → Battle.net / EA / Ubisoft …).
#[derive(Clone, Copy)]
pub struct ScannerDef {
    pub name: &'static str,
    pub sources: &'static [Source],
    pub scan: ScanFn,
}

pub fn scanners() -> Vec<ScannerDef> {
    use Source::*;
    let d = |name, sources, scan| ScannerDef { name, sources, scan };
    vec![
        d("steam", &[Steam], steam::scan as ScanFn),
        d("faugus", &[Faugus], faugus::scan),
        d("heroic", &[Epic, Gog, Amazon, Heroic], heroic::scan),
        d("lutris", &[Lutris], lutris::scan),
        d("bottles", &[Bottles], bottles::scan),
        d("itch", &[Itch], itch::scan),
        d("wine-launchers", &[BattleNet, Ea, Ubisoft, Rockstar, Riot, HoYoPlay], wine_launchers::scan),
        d("minecraft", &[Minecraft], minecraft::scan),
        d("moonlight", &[Moonlight], moonlight::scan),
        d("addons", &[Addon], addons::scan),
        d("emulation", &[Emulator], emulation::scan),
        d("romm", &[Romm], romm::scan),
        d("desktop", &[Desktop], desktop::scan),
    ]
}

/// Run every enabled scanner concurrently (one thread each; they are I/O bound and tiny).
pub fn run_all(ctx: &ScanContext) -> ScanReport {
    let t0 = Instant::now();
    let disabled = |s: Source| ctx.config.disabled_sources.iter().any(|k| k == s.key());
    // A scanner runs when at least one of its sources is enabled.
    let enabled: Vec<ScannerDef> = scanners().into_iter().filter(|d| d.sources.iter().any(|s| !disabled(*s))).collect();
    let results: Vec<(ScannerDef, anyhow::Result<Vec<ScannedGame>>, Duration)> = std::thread::scope(|sc| {
        let handles: Vec<_> = enabled
            .iter()
            .map(|&d| {
                sc.spawn(move || {
                    let t = Instant::now();
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (d.scan)(ctx)))
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("scanner panicked")));
                    (d, r, t.elapsed())
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().expect("scanner thread")).collect()
    });

    let mut report = ScanReport::default();
    for (d, res, elapsed) in results {
        let wanted: Vec<Source> = d.sources.iter().copied().filter(|s| !disabled(*s)).collect();
        match res {
            Ok(mut games) => {
                games.retain(|g| wanted.contains(&g.source) && !g.name.trim().is_empty() && !g.source_id.is_empty());
                for src in &wanted {
                    let found = games.iter().filter(|g| g.source == *src).count();
                    report.per_source.push((*src, SourceReport { found, error: None, elapsed }));
                    report.ok_sources.push(*src);
                }
                report.games.extend(games);
            }
            Err(e) => {
                log::warn!("scan {}: {e:#}", d.name);
                for src in &wanted {
                    report.per_source.push((*src, SourceReport { found: 0, error: Some(format!("{e:#}")), elapsed }));
                }
            }
        }
    }
    report.duplicates_dropped = dedupe(&mut report.games);
    report.duplicates_dropped += drop_shortcuts_into_failed(&mut report);
    // RomM games downloaded into a ROM folder are emulated games now.
    report.duplicates_dropped += romm::dedupe_with_emulation(ctx, &mut report.games);
    // Leftover shortcuts into a source that scanned fine but doesn't have the game installed
    // (e.g. Steam .desktop files of uninstalled games) are noise unless the user wants uninstalled games.
    if !ctx.config.include_uninstalled {
        let ok = report.ok_sources.clone();
        let before = report.games.len();
        report.games.retain(|g| !(g.duplicate_of.as_ref().is_some_and(|(s, _)| ok.contains(s)) && !g.installed));
        report.duplicates_dropped += before - report.games.len();
    }
    report.elapsed = t0.elapsed();
    report.steam_needs = ctx.steam_names.needs.lock().clone();
    report
}

/// Drop shortcuts into a library whose scan failed. They can't be matched to its games, and the
/// library keeps that source's games from before, so keeping them would list a game twice.
fn drop_shortcuts_into_failed(report: &mut ScanReport) -> usize {
    let failed: Vec<Source> = report.per_source.iter().filter(|(_, r)| r.error.is_some()).map(|(s, _)| *s).collect();
    if failed.is_empty() {
        return 0;
    }
    // Heroic shortcuts name the app, not the store: any of Heroic's stores may hold it.
    let into_failed = |s: Source| match s {
        Source::Heroic => {
            [Source::Heroic, Source::Epic, Source::Gog, Source::Amazon].iter().any(|h| failed.contains(h))
        }
        s => failed.contains(&s),
    };
    let before = report.games.len();
    report.games.retain(|g| !g.duplicate_of.as_ref().is_some_and(|(s, _)| into_failed(*s)));
    before - report.games.len()
}

/// Drop desktop entries that duplicate a game from a real source (lending their art to it if it
/// lacks some). Returns how many were dropped.
pub fn dedupe(games: &mut Vec<ScannedGame>) -> usize {
    let index: HashMap<(Source, String), usize> = games
        .iter()
        .enumerate()
        .filter(|(_, g)| g.duplicate_of.is_none())
        .map(|(i, g)| ((g.source, g.source_id.clone()), i))
        .collect();
    let mut donations: Vec<(usize, Art)> = vec![];
    let mut drop = vec![false; games.len()];
    for (i, g) in games.iter().enumerate() {
        if let Some(key) = &g.duplicate_of {
            // Heroic shortcuts name the app, not the store: it may be an Epic, GOG or Amazon game now.
            let target = index.get(key).copied().or_else(|| {
                (key.0 == Source::Heroic).then(|| {
                    [Source::Epic, Source::Gog, Source::Amazon]
                        .into_iter()
                        .find_map(|s| index.get(&(s, key.1.clone())).copied())
                })?
            });
            if let Some(target) = target {
                let mut art = g.art.clone();
                // A desktop entry's "cover" is just its (square) icon; don't pass it off as box art.
                if art.cover.is_some() && art.cover == art.icon {
                    art.cover = None;
                }
                donations.push((target, art));
                drop[i] = true;
            }
        }
    }
    for (target, art) in donations {
        games[target].art.fill_from(&art);
    }
    let before = games.len();
    let mut i = 0;
    games.retain(|_| {
        let keep = !drop[i];
        i += 1;
        keep
    });
    before - games.len()
}

/// Helpers shared by scanners.
pub mod util {
    use std::path::{Path, PathBuf};

    /// First path that exists.
    pub fn first_existing<I: IntoIterator<Item = PathBuf>>(paths: I) -> Option<PathBuf> {
        paths.into_iter().find(|p| p.exists())
    }

    /// `Some(path)` if it is a non-empty file.
    pub fn file_if_exists(p: impl AsRef<Path>) -> Option<PathBuf> {
        let p = p.as_ref();
        match std::fs::metadata(p) {
            Ok(m) if m.is_file() && m.len() > 0 => Some(p.to_path_buf()),
            _ => None,
        }
    }

    /// Find `<dir>/<stem>.{png,jpg,jpeg,webp}`.
    pub fn image_with_stem(dir: &Path, stem: &str) -> Option<PathBuf> {
        ["png", "jpg", "jpeg", "webp"].iter().find_map(|ext| file_if_exists(dir.join(format!("{stem}.{ext}"))))
    }

    /// Largest launcher data file the scanners read whole (localconfig.vdf of a big account is
    /// a few MB; Heroic's store cache tens of MB).
    pub const MAX_TEXT_FILE: u64 = 64 << 20;

    /// Read a launcher's file defensively: regular files only (a FIFO or device would block or
    /// never end) and at most `MAX_TEXT_FILE` bytes.
    pub fn read_bytes(p: impl AsRef<Path>) -> std::io::Result<Vec<u8>> {
        use std::io::Read;
        let p = p.as_ref();
        let err = |what: &str| std::io::Error::other(format!("{}: {what}", p.display()));
        let meta = std::fs::metadata(p)?;
        if !meta.is_file() {
            return Err(err("not a regular file"));
        }
        if meta.len() > MAX_TEXT_FILE {
            return Err(err("too large"));
        }
        let mut bytes = Vec::with_capacity(meta.len() as usize);
        std::fs::File::open(p)?.take(MAX_TEXT_FILE + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_TEXT_FILE {
            return Err(err("too large"));
        }
        Ok(bytes)
    }

    /// [`read_bytes`] as text, invalid UTF-8 replaced instead of failing the whole file (one odd
    /// byte in localconfig.vdf must not cost every game its playtime).
    pub fn read_text(p: impl AsRef<Path>) -> std::io::Result<String> {
        Ok(match String::from_utf8(read_bytes(p)?) {
            Ok(s) => s,
            Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
        })
    }

    /// Modification time as unix seconds.
    pub fn mtime(p: &Path) -> Option<i64> {
        let m = std::fs::metadata(p).ok()?.modified().ok()?;
        Some(m.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_drops_shortcuts_and_lends_art() {
        let steam = ScannedGame::new(Source::Steam, "10", "Game");
        let mut shortcut = ScannedGame::new(Source::Desktop, "game.desktop", "Game");
        shortcut.duplicate_of = Some((Source::Steam, "10".into()));
        shortcut.art.icon = Some("/icons/g.png".into());
        let mut orphan = ScannedGame::new(Source::Desktop, "other.desktop", "Other");
        orphan.duplicate_of = Some((Source::Steam, "99".into()));
        let mut v = vec![steam, shortcut, orphan];
        assert_eq!(dedupe(&mut v), 1);
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].art.icon.as_deref(), Some(std::path::Path::new("/icons/g.png")));
    }

    #[test]
    fn shortcuts_into_a_library_that_failed_to_scan_are_left_out() {
        let shortcut = |name: &str, into: Source| {
            let mut g = ScannedGame::new(Source::Desktop, format!("{name}.desktop"), name);
            g.duplicate_of = Some((into, name.into()));
            g
        };
        let mut report = ScanReport {
            games: vec![
                shortcut("fl-2026", Source::Faugus),
                shortcut("hades", Source::Heroic),
                shortcut("osu", Source::Lutris),
                ScannedGame::new(Source::Desktop, "runelite.desktop", "RuneLite"),
            ],
            per_source: vec![
                (Source::Faugus, SourceReport { error: Some("games.json: bad JSON".into()), ..Default::default() }),
                (Source::Gog, SourceReport { error: Some("unreadable".into()), ..Default::default() }),
                (Source::Lutris, SourceReport::default()),
            ],
            ..Default::default()
        };
        assert_eq!(drop_shortcuts_into_failed(&mut report), 2);
        let left: Vec<&str> = report.games.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(left, ["osu", "RuneLite"], "Lutris scanned fine: its shortcut is still matched (or kept) by dedupe");
    }
}
