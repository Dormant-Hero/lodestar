//! One Lodestar window per library: starting the GUI again while one runs shows
//! the running window and exits, instead of opening a second copy.
//!
//! The first instance holds an exclusive `flock` on `$XDG_RUNTIME_DIR/<key>.lock` for as long as
//! it lives and listens on the unix socket `<key>.sock` next to it. A later start that can't get
//! the lock connects to the socket, sends one line (`activate`, or `activate controller` /
//! `activate desktop` for `--controller` / `--desktop`), waits for `ok` and exits.
//!
//! Robust after a crash: the kernel drops the lock with the process, so a stale socket file never
//! blocks anything (the next first instance removes it, under the lock). The lock file is opened
//! close-on-exec, so games Lodestar started never inherit it. When the lock is held but nobody
//! answers (an instance hung while starting), the new start goes ahead on its own after a few
//! seconds rather than doing nothing. An instance that is quitting answers `quitting`: the new
//! start then waits for its lock and runs as the first (no window would be shown otherwise).
//!
//! `<key>` is `lodestar-<hash of the data dir>`: instances with another `LODESTAR_HOME` are
//! separate. Only the GUI uses this (main.rs); `--play`, `--launch`, screenshot runs and the
//! diagnostics never do.
//!
//! CONTRACT: `acquire(dir, key, message, wait) -> Start`; `Instance::serve(on_message)`.

use std::io::{BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The first instance: keeps the lock while it lives.
#[derive(Debug)]
pub struct Instance {
    _lock: std::fs::File,
    listener: UnixListener,
}

#[derive(Debug)]
pub enum Start {
    /// This is the only instance.
    First(Instance),
    /// Another instance got the message: exit.
    HandedOver,
    /// Another instance holds the lock but didn't answer (or the runtime dir is unusable): run
    /// on its own, without single-instance handling.
    Alone,
}

/// The instance key for a data dir (stable across runs and Rust versions: FNV-1a).
pub fn key(data_dir: &Path) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in data_dir.as_os_str().as_encoded_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("lodestar-{h:016x}")
}

/// `$XDG_RUNTIME_DIR` (private to the user), else a private dir under the temp dir.
pub fn runtime_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    // SAFETY: plain libc call.
    let uid = unsafe { libc::getuid() };
    let d = std::env::temp_dir().join(format!("lodestar-{uid}"));
    let _ = std::fs::create_dir_all(&d);
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o700));
    d
}

/// Become the first instance, or hand `message` to the one that runs (see the module docs).
/// `wait`: how long to keep trying to reach an instance that holds the lock. One that is just
/// quitting answers `quitting`: then this start waits for its lock and becomes the first.
pub fn acquire(dir: &Path, key: &str, message: &str, wait: Duration) -> Start {
    let lock_path = dir.join(format!("{key}.lock"));
    let socket = dir.join(format!("{key}.sock"));
    let t = Instant::now();
    loop {
        let lock = match std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&lock_path) {
            Ok(f) => f,
            Err(e) => {
                log::warn!("single instance: {}: {e}", lock_path.display());
                return Start::Alone;
            }
        };
        // SAFETY: flock on a descriptor we own.
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            // Ours: any socket file left is stale (its instance is gone with its lock).
            let _ = std::fs::remove_file(&socket);
            return match UnixListener::bind(&socket) {
                Ok(listener) => Start::First(Instance { _lock: lock, listener }),
                Err(e) => {
                    log::warn!("single instance: {}: {e}", socket.display());
                    Start::Alone
                }
            };
        }
        drop(lock);
        // Another instance runs: its socket may not be there yet if it is just starting.
        match hand_over(&socket, message) {
            Ok(()) => return Start::HandedOver,
            Err(e) if t.elapsed() >= wait => {
                log::warn!("another Lodestar runs but didn't answer ({e}); starting anyway");
                return Start::Alone;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn hand_over(socket: &Path, message: &str) -> std::io::Result<()> {
    let mut s = UnixStream::connect(socket)?;
    s.set_read_timeout(Some(Duration::from_secs(3)))?;
    s.set_write_timeout(Some(Duration::from_secs(3)))?;
    writeln!(s, "{}", message.trim())?;
    let mut reply = String::new();
    BufReader::new(&s).read_line(&mut reply)?;
    if reply.trim() == "ok" { Ok(()) } else { Err(std::io::Error::other(format!("unexpected reply {reply:?}"))) }
}

impl Instance {
    /// Answer later starts on a thread of its own: each message (one line) goes to `on_message`,
    /// which says whether it was taken (false: this instance is quitting, the later start is to
    /// wait for it to be gone and run itself).
    pub fn serve(self, on_message: impl Fn(String) -> bool + Send + 'static) {
        let spawned = std::thread::Builder::new().name("instance".into()).spawn(move || {
            let me = self;
            for conn in me.listener.incoming() {
                let Ok(mut s) = conn else { continue };
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                if BufReader::new(&s).read_line(&mut line).is_ok_and(|n| n > 0) {
                    let reply: &[u8] = if on_message(line.trim().to_string()) { b"ok\n" } else { b"quitting\n" };
                    let _ = s.write_all(reply);
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("single instance: no thread ({e})");
        }
    }
}

/// A message's mode: `activate controller` / `activate desktop`.
pub fn requested_mode(message: &str) -> Option<crate::app::Mode> {
    match message.split_whitespace().nth(1) {
        Some("controller") => Some(crate::app::Mode::Controller),
        Some("desktop") => Some(crate::app::Mode::Desktop),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Mode;

    const WAIT: Duration = Duration::from_millis(600);

    #[test]
    fn second_start_hands_over_and_exits() {
        let dir = tempfile::tempdir().unwrap();
        let Start::First(first) = acquire(dir.path(), "k", "activate", WAIT) else { panic!("not first") };
        let (tx, rx) = crossbeam_channel::unbounded();
        first.serve(move |m| tx.send(m).is_ok());
        assert!(matches!(acquire(dir.path(), "k", "activate", WAIT), Start::HandedOver));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "activate");
        assert!(matches!(acquire(dir.path(), "k", "activate controller\n", WAIT), Start::HandedOver));
        let m = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!((m.as_str(), requested_mode(&m)), ("activate controller", Some(Mode::Controller)));
        // Another key (another LODESTAR_HOME) is a separate instance.
        assert!(matches!(acquire(dir.path(), "other", "activate", WAIT), Start::First(_)));
    }

    #[test]
    fn stale_socket_after_a_crash_is_recovered() {
        let dir = tempfile::tempdir().unwrap();
        // A crashed instance: its socket file is left, nobody listens, nobody holds the lock.
        let sock = dir.path().join("k.sock");
        drop(UnixListener::bind(&sock).unwrap());
        std::fs::write(dir.path().join("k.lock"), "").unwrap();
        assert!(sock.exists());
        let Start::First(first) = acquire(dir.path(), "k", "activate", WAIT) else {
            panic!("stale socket blocked start")
        };
        let (tx, rx) = crossbeam_channel::unbounded();
        first.serve(move |m| tx.send(m).is_ok());
        assert!(matches!(acquire(dir.path(), "k", "activate", WAIT), Start::HandedOver));
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), "activate");
    }

    #[test]
    fn a_start_while_the_first_quits_waits_and_runs_itself() {
        let dir = tempfile::tempdir().unwrap();
        let Start::First(first) = acquire(dir.path(), "k", "activate", WAIT) else { panic!("not first") };
        // Quitting: answers but doesn't take the message (as `serve` does with `|_| false`), and is
        // gone (its lock with it) a moment later.
        let Instance { _lock: lock, listener } = first;
        listener.set_nonblocking(true).unwrap();
        let (gone_tx, gone_rx) = std::sync::mpsc::channel::<()>();
        let answered = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let holder = std::thread::spawn({
            let answered = answered.clone();
            move || {
                while gone_rx.try_recv().is_err() {
                    if let Ok((mut s, _)) = listener.accept() {
                        s.set_nonblocking(false).unwrap();
                        let mut line = String::new();
                        BufReader::new(&s).read_line(&mut line).unwrap();
                        s.write_all(b"quitting\n").unwrap();
                        answered.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                drop((lock, listener));
            }
        });
        let later = std::thread::spawn({
            let d = dir.path().to_path_buf();
            move || acquire(&d, "k", "activate", Duration::from_secs(4))
        });
        std::thread::sleep(Duration::from_millis(400));
        gone_tx.send(()).unwrap();
        holder.join().unwrap();
        assert!(answered.load(std::sync::atomic::Ordering::SeqCst) > 0, "it asked the quitting one first");
        assert!(matches!(later.join().unwrap(), Start::First(_)), "the later start runs itself");
    }

    #[test]
    fn an_instance_that_never_answers_doesnt_block_start() {
        let dir = tempfile::tempdir().unwrap();
        // Holds the lock but never serves (hung while starting).
        let Start::First(_hung) = acquire(dir.path(), "k", "activate", WAIT) else { panic!() };
        let t = Instant::now();
        assert!(matches!(acquire(dir.path(), "k", "activate", Duration::from_millis(300)), Start::Alone));
        assert!(t.elapsed() < Duration::from_secs(5));
        // The lock goes with the instance.
        drop(_hung);
        assert!(matches!(acquire(dir.path(), "k", "activate", WAIT), Start::First(_)));
    }

    #[test]
    fn keys_and_modes() {
        let a = key(Path::new("/home/u/.local/share/lodestar"));
        assert_eq!(a, key(Path::new("/home/u/.local/share/lodestar")));
        assert_ne!(a, key(Path::new("/tmp/lodestar-test/data")));
        assert!(a.starts_with("lodestar-") && a.len() == "lodestar-".len() + 16);
        // A fixed value: the key must not change between versions (old and new copies must meet).
        assert_eq!(key(Path::new("")), "lodestar-cbf29ce484222325");
        assert_eq!(requested_mode("activate"), None);
        assert_eq!(requested_mode("activate desktop"), Some(Mode::Desktop));
        assert_eq!(requested_mode("activate bogus"), None);
    }
}
