//! `lodestar --launch NAME [--direct] [--options "..."] [--stop-after SECS] [--timeout SECS] [--dry-run]`
//!
//! Launches one game through exactly the same code path as the UI (resolve → Launcher → tracking),
//! prints the resolved command and every launch event with timestamps, optionally stops the game
//! after a while, and exits. Nothing is written to the library (playtime is not recorded).

use crate::config::Config;
use crate::db::Db;
use crate::launch::{self, LaunchEvent, Launcher};
use crate::model::*;
use crate::scan::ScanContext;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Opts {
    pub query: String,
    /// Bypass the source launcher (Steam / Faugus) with its direct command.
    pub direct: bool,
    /// Extra Steam-style launch options for this run.
    pub options: Option<String>,
    pub stop_after: Option<u64>,
    pub timeout: u64,
    pub dry_run: bool,
}

pub fn run(db: &Db, config: &Config, o: &Opts) -> anyhow::Result<()> {
    let games = db.load_all()?;
    let game = pick(&games, &o.query)?;
    println!("game:     {} [{} {}]", game.display_name(), game.source.key(), game.source_id);
    let mut game = game.clone();
    let mut ov = game.launch_override.clone().unwrap_or_default();
    if o.direct {
        let ctx = ScanContext::from_config(config);
        let (cmd, cwd) = match game.source {
            Source::Steam => {
                let d = crate::scan::steam::direct_launch_for_source_id(&ctx, &game.source_id)?;
                (d.command_line(), Some(d.cwd.clone()))
            }
            Source::Faugus => {
                let d = crate::scan::faugus::direct_launch(&ctx, &game.source_id)?;
                (d.command_line(), d.cwd.clone())
            }
            s => anyhow::bail!("--direct only applies to Steam and Faugus games (this is {})", s.display()),
        };
        ov.command = Some(cmd);
        ov.cwd = cwd;
    }
    if let Some(opts) = &o.options {
        ov.options = opts.clone();
    }
    game.launch_override = Some(ov).filter(|o| !o.is_empty());
    if let Some(src) = &game.source_launch_options {
        println!("source options: {src}");
    }
    // The library window's game options (Settings → Games, per library, detected tools), so
    // this prints and runs what the UI would.
    launch::play::publish_headless(&crate::config::Paths::detect(), config);
    // "Choose what to play": the remembered line, else the first (what Play does without a UI).
    let entries = crate::emu_app::play_entries_now(&game);
    if entries.len() > 1 {
        let picked = crate::emulation::roms::remembered(&game, &entries);
        let names: Vec<&str> = entries.iter().map(|e| e.label.as_str()).collect();
        println!(
            "choices:  {} ({})",
            names.join(" | "),
            if picked.is_some() { "remembered one used" } else { "the first used" }
        );
    }
    crate::emu_app::apply_play_choice(&mut game, None);
    let r = launch::resolve(&game)?;
    println!("kind:     {:?}", r.kind);
    println!("will run: {}", r.display);
    for n in &r.notes {
        println!("note:     {n}");
    }
    if o.dry_run {
        return Ok(());
    }

    let ctx = egui::Context::default();
    let mut l = Launcher::new(&ctx);
    let t0 = Instant::now();
    let stamp = |t: Instant| format!("[{:>6.1}s]", t.elapsed().as_secs_f32());
    l.launch(&game)?;
    println!("{} launched", stamp(t0));
    let mut started_at: Option<Instant> = None;
    let mut stop_sent = false;
    let deadline = t0 + Duration::from_secs(o.timeout.max(5));
    loop {
        for ev in l.poll() {
            match ev {
                LaunchEvent::Started { .. } => {
                    started_at = Some(Instant::now());
                    println!("{} STARTED (game process detected)", stamp(t0));
                }
                LaunchEvent::Exited { secs, .. } => {
                    println!("{} EXITED after {secs}s of tracked play", stamp(t0));
                    return Ok(());
                }
                LaunchEvent::Failed { error, .. } => {
                    println!("{} FAILED: {error}", stamp(t0));
                    return Ok(());
                }
            }
        }
        if let (Some(s), Some(after), false) = (started_at, o.stop_after, stop_sent)
            && s.elapsed() >= Duration::from_secs(after)
        {
            println!("{} stopping the game (--stop-after {after})", stamp(t0));
            l.stop();
            stop_sent = true;
        }
        if Instant::now() > deadline {
            println!("{} timeout: running={:?} started={:?}; stopping", stamp(t0), l.running(), l.started());
            l.stop();
            std::thread::sleep(Duration::from_secs(5));
            for ev in l.poll() {
                println!("{} {ev:?}", stamp(t0));
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Exact (case-insensitive) name, else source id, else best fuzzy match.
fn pick<'a>(games: &'a [Game], q: &str) -> anyhow::Result<&'a Game> {
    let ql = q.to_lowercase();
    if let Some(g) = games.iter().find(|g| g.display_name().to_lowercase() == ql) {
        return Ok(g);
    }
    if let Some(g) = games.iter().find(|g| g.source_id == q || g.id.to_string() == q) {
        return Ok(g);
    }
    games
        .iter()
        .filter_map(|g| crate::search::score(q, g.display_name()).map(|s| (s, g)))
        .max_by_key(|(s, _)| *s)
        .map(|(_, g)| g)
        .ok_or_else(|| anyhow::anyhow!("no game matches {q:?}"))
}
