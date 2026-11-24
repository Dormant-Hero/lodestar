//! Running an add-on: one process per call, JSON in on stdin, JSON out on stdout, stderr to the log,
//! bounded output and a hard timeout that kills the whole process group.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// Largest reply read from stdout.
pub const MAX_STDOUT: usize = 8 << 20;
/// Most stderr text passed to the log per call (the rest is read and dropped).
const MAX_STDERR_LOG: usize = 64 << 10;
/// After the add-on exits, how long to wait for its stdout to close (a background child it left
/// behind may hold the pipe open).
const DRAIN_GRACE: Duration = Duration::from_millis(500);

pub struct Invocation<'a> {
    pub id: &'a str,
    pub exec: &'a Path,
    pub cwd: &'a Path,
    pub command: &'a str,
    pub stdin: Vec<u8>,
    pub env: Vec<(String, String)>,
    pub timeout: Duration,
}

/// Start `cmd`, trying again for a moment while its program is "busy" (ETXTBSY): an add-on that
/// was just installed or updated can't be run while another thread's freshly forked child still
/// holds the file open for writing (it closes when that child execs, within milliseconds).
fn spawn_retrying(cmd: &mut Command) -> std::io::Result<std::process::Child> {
    let mut wait = Duration::from_millis(5);
    let give_up = Instant::now() + Duration::from_secs(2);
    loop {
        match cmd.spawn() {
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) && Instant::now() < give_up => {
                std::thread::sleep(wait);
                wait = (wait * 2).min(Duration::from_millis(100));
            }
            r => return r,
        }
    }
}

/// Run the add-on once and return its parsed JSON reply.
pub fn invoke(inv: Invocation<'_>) -> anyhow::Result<serde_json::Value> {
    use std::os::unix::process::CommandExt;
    let mut cmd = Command::new(inv.exec);
    cmd.arg(inv.command)
        .current_dir(inv.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Own process group, so a timeout can kill everything the add-on started.
        .process_group(0);
    for (k, v) in &inv.env {
        cmd.env(k, v);
    }
    // If Lodestar dies mid-call (crash, kill), the add-on goes too. This thread waits for the
    // child below, so the "parent thread exited" signal can't come early.
    // SAFETY: prctl is async-signal-safe; nothing else runs between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
    let mut child =
        spawn_retrying(&mut cmd).map_err(|e| anyhow::anyhow!("could not start {}: {e}", inv.exec.display()))?;
    let pid = child.id() as libc::pid_t;
    let _running = Running::new(pid);
    let started = Instant::now();

    // stdin: written from a thread so a child that doesn't read it can't deadlock us.
    if let Some(mut stdin) = child.stdin.take() {
        let data = inv.stdin;
        std::thread::spawn(move || {
            let _ = stdin.write_all(&data);
        });
    }

    let overflow = Arc::new(AtomicBool::new(false));
    let (out_tx, out_rx) = crossbeam_channel::bounded::<Vec<u8>>(1);
    if let Some(mut out) = child.stdout.take() {
        let overflow = overflow.clone();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 64 * 1024];
            loop {
                match out.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        if buf.len() + n > MAX_STDOUT {
                            overflow.store(true, Ordering::SeqCst);
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            let _ = out_tx.send(buf);
        });
    }
    let (err_tx, err_rx) = crossbeam_channel::bounded::<String>(1);
    if let Some(err) = child.stderr.take() {
        let id = inv.id.to_string();
        std::thread::spawn(move || {
            let _ = err_tx.send(log_stderr(&id, err));
        });
    }

    // Only this thread reaps the child, so the process group still exists when it is killed.
    let deadline = started + inv.timeout;
    let mut nap = Duration::from_millis(1);
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Ok(st),
            Ok(None) => {}
            Err(e) => break Err(anyhow::anyhow!("waiting for the add-on: {e}")),
        }
        let over = overflow.load(Ordering::SeqCst);
        if over || Instant::now() >= deadline {
            kill_group(pid);
            let _ = child.wait();
            if over {
                break Err(anyhow::anyhow!("reply is larger than {} MB", MAX_STDOUT >> 20));
            }
            break Err(anyhow::anyhow!("timed out after {} s", inv.timeout.as_secs_f32()));
        }
        std::thread::sleep(nap.min(deadline.saturating_duration_since(Instant::now())));
        nap = (nap * 2).min(Duration::from_millis(20));
    };
    let status = status?;
    if overflow.load(Ordering::SeqCst) {
        kill_group(pid);
        anyhow::bail!("reply is larger than {} MB", MAX_STDOUT >> 20);
    }
    let stdout = match out_rx.recv_timeout(DRAIN_GRACE) {
        Ok(b) => b,
        Err(_) => {
            // Something it started still holds stdout open: end it, take what was sent.
            kill_group(pid);
            out_rx.recv_timeout(DRAIN_GRACE).unwrap_or_default()
        }
    };
    if overflow.load(Ordering::SeqCst) {
        anyhow::bail!("reply is larger than {} MB", MAX_STDOUT >> 20);
    }
    let stderr_tail = err_rx.recv_timeout(Duration::from_millis(100)).unwrap_or_default();
    log::debug!("addon {} {}: {:?} in {:?}", inv.id, inv.command, status, started.elapsed());

    let text = String::from_utf8_lossy(&stdout);
    let text = text.trim();
    if !status.success() {
        // An add-on may still explain itself with {"error": …} on stdout.
        if let Some(msg) = reply_error(text) {
            anyhow::bail!("{msg}");
        }
        let code = status.code().map(|c| format!("exit code {c}")).unwrap_or_else(|| "killed by a signal".into());
        if stderr_tail.is_empty() {
            anyhow::bail!("failed ({code})");
        }
        anyhow::bail!("failed ({code}): {stderr_tail}");
    }
    if text.is_empty() {
        anyhow::bail!("no reply on stdout");
    }
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| anyhow::anyhow!("reply is not valid JSON: {e}"))?;
    if !value.is_object() {
        anyhow::bail!("reply is not a JSON object");
    }
    if let Some(msg) = value.get("error").and_then(|e| e.as_str()) {
        anyhow::bail!("{}", clip(msg, 300));
    }
    Ok(value)
}

fn reply_error(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    Some(clip(v.get("error")?.as_str()?, 300))
}

/// Pass stderr lines to the log (bounded); return the last non-empty line for error messages.
fn log_stderr(id: &str, mut err: impl Read) -> String {
    let mut logged = 0usize;
    let mut last = String::new();
    let mut pending = Vec::new();
    let mut chunk = [0u8; 8192];
    let emit = |line: &[u8], logged: &mut usize, last: &mut String| {
        let s = String::from_utf8_lossy(line);
        let s = s.trim_end();
        if s.trim().is_empty() {
            return;
        }
        if *logged < MAX_STDERR_LOG {
            log::info!("addon {id}: {s}");
            *logged += s.len();
        }
        *last = clip(s.trim(), 300);
    };
    loop {
        match err.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                pending.extend_from_slice(&chunk[..n]);
                while let Some(i) = pending.iter().position(|&b| b == b'\n') {
                    let line: Vec<u8> = pending.drain(..=i).collect();
                    emit(&line, &mut logged, &mut last);
                }
                if pending.len() > 16 * 1024 {
                    let line = std::mem::take(&mut pending);
                    emit(&line, &mut logged, &mut last);
                }
            }
        }
    }
    if !pending.is_empty() {
        emit(&pending, &mut logged, &mut last);
    }
    last
}

/// Process groups of add-on calls in flight (ended by [`kill_running`] when Lodestar quits).
static RUNNING: parking_lot::Mutex<Vec<libc::pid_t>> = parking_lot::Mutex::new(Vec::new());

struct Running(libc::pid_t);

impl Running {
    fn new(pid: libc::pid_t) -> Running {
        RUNNING.lock().push(pid);
        Running(pid)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.lock().retain(|p| *p != self.0);
    }
}

/// Kill every add-on call still running (and what it started). Called when Lodestar exits, so
/// a hanging add-on doesn't outlive it.
pub fn kill_running() {
    for pid in RUNNING.lock().drain(..) {
        kill_group(pid);
    }
}

fn kill_group(pid: libc::pid_t) {
    if pid > 0 {
        // SAFETY: plain syscall; the group is the child's own (process_group(0)), not ours.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
}

pub fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Where an add-on may keep its own files (`LODESTAR_ADDON_DATA`): next to the add-ons folder,
/// so removing or updating the add-on keeps its data.
pub fn data_dir_for(addons_dir: &Path, id: &str) -> PathBuf {
    match addons_dir.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.join("addon-data").join(id),
        _ => addons_dir.join(".data").join(id),
    }
}
