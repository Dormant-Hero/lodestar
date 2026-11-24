//! Performance probes for screenshot runs (testing only, never in a normal start):
//!
//! - `LODESTAR_IDLE_PROBE=SECS`: once the UI has settled (before the screenshot), stop forcing
//!   frames and count how many frames the app still draws by itself over SECS seconds, with
//!   their repaint causes. An idle window should draw (almost) none.
//! - Every screenshot run prints how long `ui()` took per frame (count, mean, 95th percentile,
//!   worst) when `LODESTAR_FRAME_STATS=1`, so big-library runs can be compared before / after.
//! - `LODESTAR_LATENCY_PROBE=1`: for every injected input (`--inject`), how long its frame
//!   took and how long until every picture that frame asked for was on screen; per screen and
//!   input a summary line, plus every frame over 16 ms / 33 ms with what was happening
//!   (`=all`: every input on its own line too).
//!   `LODESTAR_INJECT_MS=N` sends the injected inputs N ms apart (a held direction repeats
//!   about every 50–100 ms) instead of one every 4th frame.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Idle-frame counter (see the module docs).
pub(crate) struct IdleProbe {
    pub secs: f64,
    started: Option<(Instant, u64)>,
    /// CPU ticks per thread (name → utime + stime) when the probe started.
    cpu0: BTreeMap<String, u64>,
    causes: BTreeMap<String, u32>,
    pub done: bool,
}

impl IdleProbe {
    pub fn from_env() -> Option<IdleProbe> {
        let secs: f64 = std::env::var("LODESTAR_IDLE_PROBE").ok()?.parse().ok()?;
        Some(IdleProbe {
            secs: secs.max(0.5),
            started: None,
            cpu0: BTreeMap::new(),
            causes: BTreeMap::new(),
            done: false,
        })
    }

    /// Called once the UI settled. Returns true while the probe still runs (the caller then
    /// neither forces frames nor takes the screenshot).
    pub fn running(&mut self, ctx: &egui::Context, frames: u64) -> bool {
        if self.done {
            return false;
        }
        let now = Instant::now();
        let Some((t0, f0)) = self.started else {
            self.started = Some((now, frames));
            self.cpu0 = thread_ticks();
            // One wake-up at the end, so the probe can finish even when nothing repaints.
            let (ctx, secs) = (ctx.clone(), self.secs);
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs_f64(secs + 0.05));
                ctx.request_repaint();
            });
            return true;
        };
        for c in ctx.repaint_causes() {
            *self.causes.entry(format!("{c}")).or_default() += 1;
        }
        if now.duration_since(t0).as_secs_f64() < self.secs {
            return true;
        }
        // The last frame is the probe's own wake-up.
        let n = frames.saturating_sub(f0).saturating_sub(1);
        eprintln!("idle probe: {n} frames in {:.1} s ({:.2} fps)", self.secs, n as f64 / self.secs);
        let mut top: Vec<_> = self.causes.iter().collect();
        top.sort_by_key(|t| std::cmp::Reverse(*t.1));
        for (cause, count) in top.into_iter().take(8) {
            eprintln!("idle probe:   {count:>5} × {cause}");
        }
        // CPU used meanwhile, per thread (clock ticks, usually 1/100 s): background threads
        // that poll too often show up here even when no frame is drawn.
        let cpu1 = thread_ticks();
        let mut used: Vec<(String, u64)> = cpu1
            .iter()
            .map(|(k, v)| (k.clone(), v.saturating_sub(self.cpu0.get(k).copied().unwrap_or(0))))
            .filter(|(_, d)| *d > 0)
            .collect();
        used.sort_by_key(|u| std::cmp::Reverse(u.1));
        let total: u64 = used.iter().map(|u| u.1).sum();
        eprintln!(
            "idle probe: CPU {:.2} s in {:.1} s ({:.1} %)",
            total as f64 / 100.0,
            self.secs,
            total as f64 / self.secs
        );
        for (name, d) in used.into_iter().take(8) {
            eprintln!("idle probe:   {:>6.2} s  {name}", d as f64 / 100.0);
        }
        self.done = true;
        false
    }
}

/// `LODESTAR_FRAME_STATS=bump`: bump the library generation on every 4th frame from frame 40
/// to 120 (see `App::ui_inner`).
pub(crate) fn bump_wanted(frame: u64) -> bool {
    (40..120).contains(&frame)
        && frame.is_multiple_of(4)
        && std::env::var_os("LODESTAR_FRAME_STATS").is_some_and(|v| v == "bump")
}

/// Seconds since this process started (/proc: start time in clock ticks since boot, 1/100 s).
fn process_age() -> Option<f64> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let rest = stat.rsplit_once(')')?.1;
    let start: f64 = rest.split_whitespace().nth(19)?.parse().ok()?;
    let up: f64 = std::fs::read_to_string("/proc/uptime").ok()?.split_whitespace().next()?.parse().ok()?;
    Some(up - start / 100.0)
}

/// utime + stime of every thread of this process, keyed by "name/tid".
fn thread_ticks() -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    let Ok(dir) = std::fs::read_dir("/proc/self/task") else { return out };
    for e in dir.flatten() {
        let Ok(stat) = std::fs::read_to_string(e.path().join("stat")) else { continue };
        // "tid (comm) state …": fields after the closing parenthesis; utime/stime are 14/15.
        let Some((head, rest)) = stat.rsplit_once(')') else { continue };
        let name = head.split_once('(').map_or("?", |x| x.1);
        let f: Vec<&str> = rest.split_whitespace().collect();
        let ticks = f.get(11).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0)
            + f.get(12).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        *out.entry(format!("{name}/{}", e.file_name().to_string_lossy())).or_default() += ticks;
    }
    out
}

/// Per-frame `ui()` durations (see the module docs).
#[derive(Default)]
pub(crate) struct FrameStats {
    times: Vec<f32>,
    /// eframe's whole previous frame (ui + tessellation + rendering, which is software
    /// rendering in headless runs), ms.
    whole: Vec<f32>,
}

impl FrameStats {
    pub fn wanted() -> bool {
        std::env::var_os("LODESTAR_FRAME_STATS").is_some()
    }

    pub fn push_whole(&mut self, secs: Option<f32>) {
        if let Some(s) = secs {
            self.whole.push(s * 1000.0);
        }
    }

    pub fn push(&mut self, d: Duration) {
        if self.times.is_empty()
            && let Some(age) = process_age()
        {
            eprintln!("first frame: {:.0} ms after the process started", age * 1000.0);
        }
        let ms = d.as_secs_f32() * 1000.0;
        self.times.push(ms);
        if ms >= 8.0 {
            eprintln!("slow frame {}: ui {ms:.1} ms", self.times.len());
        }
    }

    /// One line: frames, mean, p95 and worst (with its frame number), skipping the first
    /// `skip` frames (start-up: fonts, first texture uploads).
    pub fn summary(&self, skip: usize) -> String {
        let t: Vec<f32> = self.times.iter().skip(skip).copied().collect();
        if t.is_empty() {
            return "frame stats: no frames".into();
        }
        let (worst_i, worst) =
            t.iter().copied().enumerate().fold((0, 0.0f32), |a, (i, v)| if v > a.1 { (i, v) } else { a });
        let mut s = t.clone();
        s.sort_by(f32::total_cmp);
        let p95 = s[((s.len() as f32 * 0.95) as usize).min(s.len() - 1)];
        let mean = t.iter().sum::<f32>() / t.len() as f32;
        let mut w: Vec<f32> = self.whole.iter().skip(skip).copied().collect();
        w.sort_by(f32::total_cmp);
        let whole = if w.is_empty() {
            String::new()
        } else {
            format!(
                "; whole frame p50 {:.1} ms, p95 {:.1} ms, worst {:.1} ms",
                w[w.len() / 2],
                w[((w.len() as f32 * 0.95) as usize).min(w.len() - 1)],
                w[w.len() - 1]
            )
        };
        format!(
            "frame stats: {} frames, ui mean {mean:.2} ms, p95 {p95:.2} ms, worst {worst:.2} ms (frame {}); first frame {:.1} ms{whole}",
            t.len(),
            worst_i + skip + 1,
            self.times.first().copied().unwrap_or(0.0)
        )
    }
}

/// Milliseconds between injected inputs (`LODESTAR_INJECT_MS`), None: one every 4th frame.
pub(crate) fn inject_every() -> Option<Duration> {
    let ms: u64 = std::env::var("LODESTAR_INJECT_MS").ok()?.parse().ok()?;
    Some(Duration::from_millis(ms.clamp(1, 5000)))
}

/// Resident memory of this process in MB (/proc/self/status VmRSS).
pub(crate) fn rss_mb() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let kb: f64 = s.lines().find(|l| l.starts_with("VmRSS:"))?.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

/// One injected input and what followed it.
struct NavRecord {
    what: String,
    /// Injection → end of the frame that handled it (that frame's own work), ms.
    frame_ms: f32,
    /// Injection → first frame end with every picture asked for on screen, ms (None: the next
    /// input came first, or the run ended).
    art_ms: Option<f32>,
}

/// Input-latency probe (see the module docs).
#[derive(Default)]
pub(crate) struct LatencyProbe {
    /// Inputs injected this frame: (description, when).
    fresh: Vec<(String, Instant)>,
    /// The latest input whose pictures are still loading: (index into `records`, when).
    art_wait: Option<(usize, Instant)>,
    records: Vec<NavRecord>,
    /// (ui ms, gap since the previous frame's start ms, description) of slow frames.
    slow: Vec<(f32, f32, String)>,
    frames: u32,
    last_start: Option<Instant>,
    peak_rss: f64,
}

impl LatencyProbe {
    pub fn wanted() -> bool {
        std::env::var_os("LODESTAR_LATENCY_PROBE").is_some()
    }

    /// An input was injected (before the frame handles it). `what`: screen + input.
    pub fn injected(&mut self, what: String) {
        self.fresh.push((what, Instant::now()));
    }

    /// Start of a frame (gaps between frames show stalls outside `ui()`).
    pub fn frame_start(&mut self) -> f32 {
        let now = Instant::now();
        let gap = self.last_start.map_or(0.0, |t| now.duration_since(t).as_secs_f32() * 1000.0);
        self.last_start = Some(now);
        gap
    }

    /// End of a frame: `ui` its work, `gap` from `frame_start`, `waiting` pictures still loading
    /// that this frame drew, `where_` what is on screen.
    pub fn frame_end(&mut self, ui: Duration, gap: f32, waiting: usize, where_: &str) {
        let now = Instant::now();
        self.frames += 1;
        let ui_ms = ui.as_secs_f32() * 1000.0;
        if self.frames > 20 && (ui_ms > 16.0 || gap > 33.0) {
            self.slow.push((ui_ms, gap, where_.to_string()));
        }
        if self.frames.is_multiple_of(16) {
            self.peak_rss = self.peak_rss.max(rss_mb().unwrap_or(0.0));
        }
        for (what, t) in std::mem::take(&mut self.fresh) {
            self.records.push(NavRecord { what, frame_ms: now.duration_since(t).as_secs_f32() * 1000.0, art_ms: None });
            // A newer input replaces the one still waiting for its pictures.
            self.art_wait = Some((self.records.len() - 1, t));
        }
        if waiting == 0
            && let Some((i, t)) = self.art_wait.take()
        {
            self.records[i].art_ms = Some(now.duration_since(t).as_secs_f32() * 1000.0);
        }
    }

    /// The report, one line each: per (screen, input) group, then the slow frames.
    pub fn summary(&self) -> Vec<String> {
        let mut groups: Vec<(&str, Vec<&NavRecord>)> = vec![];
        for r in &self.records {
            match groups.iter_mut().find(|g| g.0 == r.what) {
                Some(g) => g.1.push(r),
                None => groups.push((&r.what, vec![r])),
            }
        }
        let pct = |v: &mut Vec<f32>, q: f32| -> f32 {
            if v.is_empty() {
                return 0.0;
            }
            v.sort_by(f32::total_cmp);
            v[((v.len() as f32 * q) as usize).min(v.len() - 1)]
        };
        let mut out = vec![];
        for (what, rs) in groups {
            let mut fr: Vec<f32> = rs.iter().map(|r| r.frame_ms).collect();
            let mut art: Vec<f32> = rs.iter().filter_map(|r| r.art_ms).collect();
            let late = rs.len() - art.len();
            out.push(format!(
                "latency: {what:<28} ×{:<3} frame p50 {:.1} ms max {:.1} ms; pictures ready p50 {:.0} ms max {:.0} ms{}",
                rs.len(),
                pct(&mut fr, 0.5),
                pct(&mut fr, 1.0),
                pct(&mut art, 0.5),
                pct(&mut art, 1.0),
                if late > 0 { format!(", {late} not ready before the next input") } else { String::new() }
            ));
        }
        if std::env::var_os("LODESTAR_LATENCY_PROBE").is_some_and(|v| v == "all") {
            for (i, r) in self.records.iter().enumerate() {
                out.push(format!(
                    "latency:   #{i} {}: frame {:.1} ms, pictures {}",
                    r.what,
                    r.frame_ms,
                    r.art_ms.map_or("not ready before the next input".into(), |a| format!("{a:.0} ms"))
                ));
            }
        }
        let over16 = self.slow.iter().filter(|s| s.0 > 16.0).count();
        let over33 = self.slow.iter().filter(|s| s.0 > 33.0 || s.1 > 50.0).count();
        out.push(format!(
            "latency: {} frames; ui > 16 ms: {over16}; ui > 33 ms or a gap > 50 ms: {over33}; peak memory {:.0} MB",
            self.frames,
            self.peak_rss.max(rss_mb().unwrap_or(0.0))
        ));
        for (ui, gap, w) in self.slow.iter().take(12) {
            out.push(format!("latency:   slow frame: ui {ui:.1} ms, {gap:.1} ms since the previous frame, on {w}"));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latency_probe_times_inputs_and_their_pictures() {
        let mut p = LatencyProbe::default();
        p.injected("Games Right".into());
        p.frame_end(Duration::from_millis(1), 1.0, 2, "Games");
        p.frame_end(Duration::from_millis(1), 1.0, 0, "Games");
        p.injected("Games Right".into());
        p.frame_end(Duration::from_millis(1), 1.0, 1, "Games");
        p.injected("Games Right".into());
        p.frame_end(Duration::from_millis(1), 1.0, 1, "Games");
        let s = p.summary();
        assert!(s[0].contains("Games Right") && s[0].contains("×3"), "{s:?}");
        assert!(s[0].contains("2 not ready before the next input"), "{s:?}");
    }

    #[test]
    fn frame_stats_summary_skips_startup_and_finds_the_worst_frame() {
        let mut f = FrameStats::default();
        for ms in [50.0, 1.0, 2.0, 9.0, 1.0] {
            f.push(Duration::from_secs_f32(ms / 1000.0));
        }
        let s = f.summary(1);
        assert!(s.contains("4 frames"), "{s}");
        assert!(s.contains("worst 9.00 ms (frame 4)"), "{s}");
        assert!(s.contains("first frame 50.0 ms"), "{s}");
        assert_eq!(FrameStats::default().summary(0), "frame stats: no frames");
    }
}
