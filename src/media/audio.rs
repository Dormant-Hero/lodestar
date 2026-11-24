//! Navigation sounds and background music (controller mode). Off by default.
//!
//! `global().sync(..)` runs every logic pass with what the settings and the theme want and
//! whether Lodestar is in front without a game (cheap when nothing changed); `play(sound)` on a
//! button press. A helper thread exists only while sounds or music are on: it decodes the
//! sounds once, mixes, and feeds one long-running player process (player.rs). While Lodestar
//! isn't in front, or a game runs, music fades out and the player is closed.

use super::decode::{self, RATE};
use super::mixer::{Mixer, Music, gain_for};
use super::player::{Opener, SystemPlayer};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// What a sound is for (the names themes use for navigation sounds).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Sound {
    /// Moving in a list or grid.
    Scroll,
    /// Moving between systems / collections.
    SystemBrowse,
    /// Jumping (LB / RB, letters).
    QuickSysSelect,
    /// A: opening something.
    Select,
    /// B: going back, closing a menu.
    Back,
    Favorite,
    /// Starting a game.
    Launch,
}

impl Sound {
    pub const ALL: [Sound; 7] = [
        Sound::Scroll,
        Sound::SystemBrowse,
        Sound::QuickSysSelect,
        Sound::Select,
        Sound::Back,
        Sound::Favorite,
        Sound::Launch,
    ];

    /// The name themes use (`[sounds] scroll = "…"`).
    pub fn key(self) -> &'static str {
        match self {
            Sound::Scroll => "scroll",
            Sound::SystemBrowse => "systembrowse",
            Sound::QuickSysSelect => "quicksysselect",
            Sound::Select => "select",
            Sound::Back => "back",
            Sound::Favorite => "favorite",
            Sound::Launch => "launch",
        }
    }

    pub fn from_key(k: &str) -> Option<Sound> {
        Sound::ALL.into_iter().find(|s| s.key().eq_ignore_ascii_case(k.trim()))
    }
}

/// What the settings and the theme ask for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wanted {
    pub sounds: bool,
    pub music: bool,
    /// 0..=100.
    pub volume: u8,
    /// The theme's own sound files (the rest use Lodestar's built-in sounds).
    pub sound_files: Vec<(Sound, PathBuf)>,
    /// Music files and folders (folders are searched).
    pub music_from: Vec<PathBuf>,
}

impl Wanted {
    pub fn any(&self) -> bool {
        self.sounds || self.music
    }

    /// From the controller settings and the theme. Music folder: the one chosen, else the
    /// theme's music and ES-DE's music folder (`~/ES-DE/music`).
    pub fn from(c: &crate::config::ControllerConfig, theme: &crate::theme::Theme, home: &Path) -> Wanted {
        let sound_files =
            theme.file.sounds.iter().filter_map(|(k, p)| Some((Sound::from_key(k)?, theme.resolve(p)?))).collect();
        let music_from = match c.music_folder.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(f) => vec![PathBuf::from(f)],
            None => auto_music(theme, home),
        };
        Wanted {
            sounds: c.sounds_enabled(&theme.file),
            music: c.music_enabled(&theme.file),
            volume: c.volume.min(100),
            sound_files,
            music_from,
        }
    }
}

/// Where "Automatic" looks for music.
pub fn auto_music(theme: &crate::theme::Theme, home: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = theme.file.music.iter().filter_map(|p| theme.resolve(p)).collect();
    v.push(home.join("ES-DE/music"));
    v
}

/// Music files under `from` (folders searched three levels deep), sorted; at most 5000.
pub fn find_music(from: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(d: &Path, depth: u32, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(d) else { return };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            if out.len() >= 5000 {
                return;
            }
            if p.is_dir() {
                if depth > 0 && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
                    walk(&p, depth - 1, out);
                }
            } else if super::ext_in(&p.to_string_lossy(), &super::AUDIO_EXTS) {
                out.push(p);
            }
        }
    }
    let mut out = vec![];
    for f in from {
        if f.is_dir() {
            walk(f, 3, &mut out);
        } else if f.is_file() && super::ext_in(&f.to_string_lossy(), &super::AUDIO_EXTS) {
            out.push(f.clone());
        }
    }
    out.dedup();
    out
}

/// What the settings page can tell the user.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// The player in use ("pw-cat" …); None = none found (when sounds are on).
    pub player: Option<&'static str>,
    /// Looked for a player and found none.
    pub no_player: bool,
    /// Songs found for the music (once looked).
    pub songs: Option<usize>,
}

enum Cmd {
    Play(Sound),
    Wanted(Wanted),
    Active(bool),
    Quit,
}

struct Inner {
    tx: Option<Sender<Cmd>>,
    wanted: Wanted,
    active: bool,
}

pub struct Audio {
    inner: Mutex<Inner>,
    status: Arc<Mutex<Status>>,
    opener: fn() -> Box<dyn Opener>,
}

/// Per logic pass: what the controller settings and theme want, and whether Lodestar is in
/// front in controller mode without a game. Nothing is done while sounds and music are off.
pub fn sync_app(st: &crate::app::AppState, active: bool) {
    let c = &st.config.controller;
    let a = global();
    if !c.sounds_enabled(&st.controller_theme.file) && !c.music_enabled(&st.controller_theme.file) {
        if a.running() {
            a.sync(&Wanted::default(), active);
        }
        return;
    }
    a.sync(&Wanted::from(c, &st.controller_theme, &st.paths.home), active);
}

/// The app's audio.
pub fn global() -> &'static Audio {
    static A: OnceLock<Audio> = OnceLock::new();
    A.get_or_init(|| Audio::with_opener(|| Box::new(SystemPlayer::new())))
}

impl Audio {
    pub fn with_opener(opener: fn() -> Box<dyn Opener>) -> Audio {
        Audio {
            inner: Mutex::new(Inner { tx: None, wanted: Wanted::default(), active: false }),
            status: Arc::default(),
            opener,
        }
    }

    /// Settle on `w` and whether Lodestar is in front without a game running. Starts / stops
    /// the helper thread; no-op when nothing changed.
    pub fn sync(&self, w: &Wanted, active: bool) {
        let mut g = self.inner.lock();
        if !w.any() {
            if let Some(tx) = g.tx.take() {
                let _ = tx.send(Cmd::Quit);
            }
            g.wanted = w.clone();
            return;
        }
        if g.tx.is_none() {
            let (tx, rx) = crossbeam_channel::unbounded();
            let (status, opener) = (self.status.clone(), self.opener);
            let ok = std::thread::Builder::new()
                .name("lodestar-audio".into())
                .spawn(move || run(rx, opener(), &status))
                .is_ok();
            if !ok {
                return;
            }
            let _ = tx.send(Cmd::Wanted(w.clone()));
            let _ = tx.send(Cmd::Active(active));
            g.tx = Some(tx);
            g.wanted = w.clone();
            g.active = active;
            return;
        }
        if g.wanted != *w {
            g.wanted = w.clone();
            if let Some(tx) = &g.tx {
                let _ = tx.send(Cmd::Wanted(w.clone()));
            }
        }
        if g.active != active {
            g.active = active;
            if let Some(tx) = &g.tx {
                let _ = tx.send(Cmd::Active(active));
            }
        }
    }

    /// A navigation sound (nothing unless sounds are on and Lodestar is in front).
    pub fn play(&self, s: Sound) {
        let g = self.inner.lock();
        if g.wanted.sounds
            && g.active
            && let Some(tx) = &g.tx
        {
            let _ = tx.send(Cmd::Play(s));
        }
    }

    /// The helper thread is running.
    pub fn running(&self) -> bool {
        self.inner.lock().tx.is_some()
    }

    pub fn status(&self) -> Status {
        self.status.lock().clone()
    }
}

/// Sounds longer than this are cut (they are meant to be clicks).
const MAX_SOUND_SECS: f32 = 6.0;
/// Frames written per block (≈ 5 ms).
const BLOCK: usize = 256;
/// With sounds on but nothing played for this long, the player is closed (the desktop's sound
/// device may then sleep); the next press opens it again.
const IDLE_CLOSE_SECS: f64 = 600.0;

fn load_sounds(files: &[(Sound, PathBuf)]) -> HashMap<Sound, Arc<Vec<f32>>> {
    Sound::ALL
        .into_iter()
        .map(|s| {
            let theme = files.iter().find(|(k, _)| *k == s).and_then(|(_, p)| decode::decode_file(p, MAX_SOUND_SECS));
            (s, Arc::new(theme.unwrap_or_else(|| super::synth::sound(s))))
        })
        .collect()
}

/// Decode `path` on a helper thread into blocks of ≈ 0.1 s (a few blocks ahead).
fn start_track(path: PathBuf) -> Music {
    let (tx, rx) = crossbeam_channel::bounded::<Vec<f32>>(6);
    let _ = std::thread::Builder::new().name("lodestar-music".into()).spawn(move || {
        // SAFETY: gettid / setpriority take no pointers; this lowers only this thread.
        unsafe {
            libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, 10);
        }
        let Some(mut s) = decode::Stream::open(&path) else { return };
        let want = RATE as usize / 10 * 2;
        loop {
            let mut block = Vec::with_capacity(want + 8192);
            let mut more = true;
            while block.len() < want && more {
                more = s.next_block(&mut block);
            }
            if !block.is_empty() && tx.send(block).is_err() {
                return; // stopped
            }
            if !more {
                return;
            }
        }
    });
    Music::new(rx)
}

/// A small random number generator for shuffling (no crate needed).
struct Shuffle(u64);

impl Shuffle {
    fn next(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n.max(1) as u64) as usize
    }
}

struct Worker {
    mixer: Mixer,
    wanted: Wanted,
    active: bool,
    /// The sound files the mixer has loaded (None: not loaded yet).
    loaded: Option<Vec<(Sound, PathBuf)>>,
    last_sound: f64,
}

impl Worker {
    /// False: quit.
    fn apply(&mut self, c: Cmd) -> bool {
        match c {
            Cmd::Quit => return false,
            Cmd::Play(s) => {
                if self.active && self.wanted.sounds {
                    self.mixer.play(s);
                    self.last_sound = super::clock();
                }
            }
            Cmd::Active(a) => {
                self.active = a;
                if a {
                    // Open the player now, so the first press is heard at once.
                    self.last_sound = super::clock();
                }
                // Going inactive lets a playing sound end (the launch sound of the game that
                // just started); only the music fades out.
            }
            Cmd::Wanted(w) => {
                if w.sounds && self.loaded.as_ref() != Some(&w.sound_files) {
                    self.mixer.set_sounds(load_sounds(&w.sound_files));
                    self.loaded = Some(w.sound_files.clone());
                }
                if w.sounds && !self.wanted.sounds {
                    self.last_sound = super::clock();
                }
                self.mixer.volume = gain_for(w.volume);
                if w.music_from != self.wanted.music_from {
                    self.mixer.music = None;
                }
                self.wanted = w;
            }
        }
        true
    }
}

fn run(rx: Receiver<Cmd>, mut opener: Box<dyn Opener>, status: &Mutex<Status>) {
    let mut wk = Worker {
        mixer: Mixer::default(),
        wanted: Wanted::default(),
        active: false,
        loaded: None,
        last_sound: f64::NEG_INFINITY,
    };
    let mut out: Option<Box<dyn super::player::Output>> = None;
    let mut retry_at = 0.0f64;
    let mut opened_at = 0.0f64;
    let mut songs: Vec<PathBuf> = vec![];
    let mut songs_for: Option<Vec<PathBuf>> = None;
    let mut bad_tracks = 0usize;
    let mut rng = Shuffle(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64 | 1)
            .unwrap_or(0x9e37_79b9),
    );
    let mut buf = vec![0i16; BLOCK * 2];
    loop {
        // Commands first (a press is mixed into the very next block).
        loop {
            match rx.try_recv() {
                Ok(c) => {
                    if !wk.apply(c) {
                        return;
                    }
                }
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => return,
            }
        }
        // Music.
        let music_on = wk.active && wk.wanted.music;
        wk.mixer.music_target = if music_on { 1.0 } else { 0.0 };
        if music_on {
            if songs_for.as_ref() != Some(&wk.wanted.music_from) {
                songs = find_music(&wk.wanted.music_from);
                songs_for = Some(wk.wanted.music_from.clone());
                bad_tracks = 0;
                status.lock().songs = Some(songs.len());
            }
            if let Some(m) = wk.mixer.music.take_if(|m| m.ended) {
                bad_tracks = if m.played == 0 { bad_tracks + 1 } else { 0 };
            }
            if wk.mixer.music.is_none() && !songs.is_empty() && bad_tracks < songs.len().min(20) {
                let pick = songs[rng.next(songs.len())].clone();
                log::debug!("music: {}", pick.display());
                wk.mixer.music = Some(start_track(pick));
            }
        } else if wk.mixer.music_silent() {
            wk.mixer.music = None;
        }
        let recent = super::clock() - wk.last_sound < IDLE_CLOSE_SECS;
        let need = wk.mixer.busy() || wk.active && (music_on && !songs.is_empty() || wk.wanted.sounds && recent);
        if !need {
            out = None;
            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(c) => {
                    if !wk.apply(c) {
                        return;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            continue;
        }
        if out.is_none() {
            if super::clock() < retry_at {
                wk.mixer.stop_sounds();
                match rx.recv_timeout(Duration::from_millis(100)) {
                    Ok(c) => {
                        if !wk.apply(c) {
                            return;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
                continue;
            }
            out = opener.open();
            opened_at = super::clock();
            let mut st = status.lock();
            st.player = opener.name();
            st.no_player = out.is_none();
            if out.is_none() {
                retry_at = super::clock() + 30.0;
                continue;
            }
        }
        wk.mixer.mix(&mut buf);
        if let Some(o) = &mut out
            && o.write(&buf).is_err()
        {
            out = None;
            if super::clock() - opened_at < 2.0 {
                // It never ran (doesn't take these arguments?): the next player, if any.
                opener.broken();
                retry_at = super::clock() + 0.5;
            } else {
                log::info!("sound: the player stopped; trying again shortly");
                retry_at = super::clock() + 3.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::player::Output;

    static WRITTEN: Mutex<Vec<i16>> = Mutex::new(Vec::new());
    static OPENED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    struct Fake;
    impl Output for Fake {
        fn write(&mut self, pcm: &[i16]) -> std::io::Result<()> {
            WRITTEN.lock().extend_from_slice(pcm);
            std::thread::sleep(Duration::from_micros(500)); // a player paces the writes
            Ok(())
        }
    }
    struct FakeOpener;
    impl Opener for FakeOpener {
        fn name(&self) -> Option<&'static str> {
            Some("fake")
        }
        fn open(&mut self) -> Option<Box<dyn Output>> {
            OPENED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(Box::new(Fake))
        }
    }

    fn wait(cond: impl Fn() -> bool) -> bool {
        let t0 = std::time::Instant::now();
        while t0.elapsed() < Duration::from_secs(5) {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn keys_round_trip() {
        for s in Sound::ALL {
            assert_eq!(Sound::from_key(s.key()), Some(s));
        }
        assert_eq!(Sound::from_key(" Scroll "), Some(Sound::Scroll));
        assert_eq!(Sound::from_key("bgsound"), None);
    }

    #[test]
    fn audio_end_to_end_with_a_fake_player() {
        let a = Audio::with_opener(|| Box::new(FakeOpener));
        // Off: no thread, presses do nothing.
        a.sync(&Wanted::default(), true);
        a.play(Sound::Select);
        assert!(!a.running(), "no work while sounds and music are off");
        assert_eq!(OPENED.load(std::sync::atomic::Ordering::Relaxed), 0);
        // On but not in front: thread, no player.
        let w = Wanted { sounds: true, volume: 100, ..Default::default() };
        a.sync(&w, false);
        assert!(a.running());
        a.play(Sound::Select);
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(OPENED.load(std::sync::atomic::Ordering::Relaxed), 0, "no player while a game runs");
        // In front: a press is heard (the built-in select sound).
        a.sync(&w, true);
        a.play(Sound::Select);
        assert!(wait(|| WRITTEN.lock().iter().any(|s| s.unsigned_abs() > 1000)), "the sound reached the player");
        assert_eq!(OPENED.load(std::sync::atomic::Ordering::Relaxed), 1, "one player, kept open");
        assert_eq!(a.status().player, Some("fake"));
        a.play(Sound::Scroll);
        a.play(Sound::Back);
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(OPENED.load(std::sync::atomic::Ordering::Relaxed), 1, "presses don't start processes");
        // Off again: the thread ends.
        a.sync(&Wanted::default(), true);
        assert!(!a.running());
    }

    #[test]
    fn theme_sounds_replace_the_built_in_ones() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("select.wav");
        decode::write_wav(&p, 48_000, 2, &[12000; 200]);
        let m = load_sounds(&[(Sound::Select, p)]);
        assert_eq!(m[&Sound::Select].len(), 200);
        assert!((m[&Sound::Select][0] - 12000.0 / 32768.0).abs() < 1e-3);
        assert_eq!(*m[&Sound::Back], super::super::synth::sound(Sound::Back), "the rest are built in");
        // An unreadable theme sound falls back too.
        let bad = d.path().join("bad.wav");
        std::fs::write(&bad, "x").unwrap();
        let m = load_sounds(&[(Sound::Scroll, bad)]);
        assert_eq!(*m[&Sound::Scroll], super::super::synth::sound(Sound::Scroll));
    }

    #[test]
    fn music_is_found_in_folders() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("a/b/c/d/e")).unwrap();
        for f in ["x.mp3", "a/y.OGG", "a/b/z.flac", "a/b/c/d/e/too-deep.mp3", "a/cover.jpg", ".hidden/q.mp3"] {
            let p = d.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
        let found = find_music(&[d.path().to_path_buf(), d.path().join("missing")]);
        let names: Vec<String> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["z.flac", "y.OGG", "x.mp3"], "sorted by path, three folders deep, no hidden folders");
    }

    #[test]
    fn wanted_from_settings() {
        let d = tempfile::tempdir().unwrap();
        let mut c = crate::config::ControllerConfig::default();
        assert_eq!((c.nav_sounds, c.music), (crate::config::AudioMode::Auto, crate::config::AudioMode::Auto));
        let mut file = crate::theme::model::ThemeFile::default();
        file.sounds.insert("scroll".into(), "snd/scroll.wav".into());
        file.sounds.insert("bgsound".into(), "x.wav".into());
        file.music.push("music".into());
        let t = crate::theme::Theme { key: "t".into(), file, dir: Some(d.path().to_path_buf()) };
        // Automatic: the theme brings sounds and music, so both play.
        let w = Wanted::from(&c, &t, Path::new("/home/u"));
        assert!(w.sounds && w.music);
        let plain = crate::theme::Theme { key: "p".into(), file: Default::default(), dir: None };
        assert!(!Wanted::from(&c, &plain, Path::new("/home/u")).any(), "no theme audio: silent");
        c.nav_sounds = crate::config::AudioMode::Off;
        c.music = crate::config::AudioMode::Off;
        let w = Wanted::from(&c, &t, Path::new("/home/u"));
        assert!(!w.any());
        assert_eq!(w.sound_files, vec![(Sound::Scroll, d.path().join("snd/scroll.wav"))]);
        assert_eq!(w.music_from, vec![d.path().join("music"), PathBuf::from("/home/u/ES-DE/music")]);
        c.music_folder = Some("/m".into());
        c.music = crate::config::AudioMode::On;
        let w = Wanted::from(&c, &t, Path::new("/home/u"));
        assert_eq!(w.music_from, vec![PathBuf::from("/m")]);
        assert!(w.any());
    }
}
