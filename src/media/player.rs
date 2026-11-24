//! The sound output: one long-running player process of the desktop's own (PipeWire's
//! `pw-cat`, PulseAudio's `pacat`, ALSA's `aplay`) reading plain 16-bit stereo samples from a
//! pipe. The pipe is kept small, so what the mixer writes is heard within a few hundredths of
//! a second, and a full pipe paces the mixer.

use super::decode::RATE;
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    PwCat,
    Pacat,
    Aplay,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::PwCat, Kind::Pacat, Kind::Aplay];

    pub fn tool(self) -> &'static str {
        match self {
            Kind::PwCat => "pw-cat",
            Kind::Pacat => "pacat",
            Kind::Aplay => "aplay",
        }
    }

    /// Bytes the pipe to the player holds: pw-cat asks for 10 ms blocks, so 1024 frames
    /// (≈ 21 ms) never runs dry; the others read bigger blocks.
    pub fn pipe_bytes(self) -> i32 {
        match self {
            Kind::PwCat => 4096,
            Kind::Pacat | Kind::Aplay => 8192,
        }
    }

    /// The player's arguments: raw s16 little-endian stereo at `RATE` from stdin, low latency.
    pub fn args(self) -> Vec<String> {
        let r = RATE.to_string();
        let v: Vec<&str> = match self {
            Kind::PwCat => vec![
                "--playback",
                "--raw",
                "--format",
                "s16",
                "--rate",
                &r,
                "--channels",
                "2",
                "--latency",
                "10ms",
                "--media-role",
                "Game",
                "-P",
                "{ application.name = \"Lodestar\" node.description = \"Lodestar sounds\" }",
                "-",
            ],
            Kind::Pacat => vec![
                "--playback",
                "--raw",
                "--format=s16le",
                "--channels=2",
                "--latency-msec=25",
                "--client-name=Lodestar",
                "--stream-name=Lodestar sounds",
                "--property=media.role=game",
            ],
            Kind::Aplay => vec!["-q", "-t", "raw", "-f", "S16_LE", "-c", "2", "-B", "40000", "-"],
        };
        let mut v: Vec<String> = v.into_iter().map(String::from).collect();
        match self {
            Kind::Pacat => v.insert(3, format!("--rate={RATE}")),
            Kind::Aplay => {
                v.insert(7, "-r".into());
                v.insert(8, r);
            }
            Kind::PwCat => {}
        }
        v
    }
}

/// The players installed, in `Kind::ALL` order.
pub fn detect_all(find: impl Fn(&str) -> Option<PathBuf>) -> Vec<(Kind, PathBuf)> {
    Kind::ALL.into_iter().filter_map(|k| find(k.tool()).map(|p| (k, p))).collect()
}

/// Where the mixer writes. A real one is a player process; tests use a fake.
pub trait Output: Send {
    /// Write interleaved stereo samples; blocks while the player's pipe is full.
    fn write(&mut self, pcm: &[i16]) -> io::Result<()>;
}

/// Opens outputs (again after a failure or a pause).
pub trait Opener: Send {
    fn open(&mut self) -> Option<Box<dyn Output>>;
    /// The player's name ("pw-cat" …), None when there is none.
    fn name(&self) -> Option<&'static str>;
    /// The last player opened stopped right away: use the next one installed.
    fn broken(&mut self) {}
}

struct Process {
    child: Child,
    stdin: ChildStdin,
    bytes: Vec<u8>,
}

impl Output for Process {
    fn write(&mut self, pcm: &[i16]) -> io::Result<()> {
        self.bytes.clear();
        self.bytes.extend(pcm.iter().flat_map(|s| s.to_le_bytes()));
        self.stdin.write_all(&self.bytes)
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn spawn(kind: Kind, exe: &Path) -> Option<Box<dyn Output>> {
    let mut child = Command::new(exe)
        .args(kind.args())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| log::info!("sound: can't start {}: {e}", kind.tool()))
        .ok()?;
    let stdin = child.stdin.take()?;
    // SAFETY: fcntl on a pipe fd we own; a failure only leaves the default size.
    unsafe {
        libc::fcntl(stdin.as_raw_fd(), libc::F_SETPIPE_SZ, kind.pipe_bytes());
    }
    log::info!("sound: playing through {}", kind.tool());
    Some(Box::new(Process { child, stdin, bytes: Vec::new() }))
}

/// The desktop's players, found once; the first that works is used.
pub struct SystemPlayer {
    found: Vec<(Kind, PathBuf)>,
}

impl SystemPlayer {
    pub fn new() -> SystemPlayer {
        SystemPlayer { found: detect_all(super::find_tool) }
    }
}

impl Opener for SystemPlayer {
    fn name(&self) -> Option<&'static str> {
        self.found.first().map(|(k, _)| k.tool())
    }

    fn open(&mut self) -> Option<Box<dyn Output>> {
        let (k, p) = self.found.first()?;
        spawn(*k, p)
    }

    fn broken(&mut self) {
        if !self.found.is_empty() {
            let (k, _) = self.found.remove(0);
            log::info!("sound: {} stopped at once; trying the next player", k.tool());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_argv() {
        let pw = Kind::PwCat.args();
        assert_eq!(pw.last().map(String::as_str), Some("-"), "pw-cat reads stdin");
        assert!(pw.windows(2).any(|w| w[0] == "--rate" && w[1] == "48000"));
        assert!(pw.contains(&"--raw".to_string()));
        let pa = Kind::Pacat.args();
        assert!(pa.contains(&"--rate=48000".to_string()) && pa.contains(&"--format=s16le".to_string()));
        let ap = Kind::Aplay.args();
        assert!(ap.windows(2).any(|w| w[0] == "-r" && w[1] == "48000"));
        assert!(ap.windows(2).any(|w| w[0] == "-c" && w[1] == "2"));
        assert_eq!(ap.last().map(String::as_str), Some("-"));
    }

    #[test]
    fn detection_order() {
        let has = |names: &'static [&'static str]| {
            move |n: &str| names.contains(&n).then(|| PathBuf::from(format!("/usr/bin/{n}")))
        };
        let kinds = |v: Vec<(Kind, PathBuf)>| v.into_iter().map(|x| x.0).collect::<Vec<_>>();
        assert_eq!(kinds(detect_all(has(&["aplay", "pw-cat"]))), [Kind::PwCat, Kind::Aplay]);
        assert_eq!(kinds(detect_all(has(&["aplay", "pacat"]))), [Kind::Pacat, Kind::Aplay]);
        assert!(detect_all(has(&[])).is_empty());
        // A player that stops at once is skipped for the next.
        let mut p = SystemPlayer { found: detect_all(has(&["pw-cat", "aplay"])) };
        assert_eq!(p.name(), Some("pw-cat"));
        p.broken();
        assert_eq!(p.name(), Some("aplay"));
        p.broken();
        assert_eq!(p.name(), None);
        assert!(p.open().is_none());
    }
}
