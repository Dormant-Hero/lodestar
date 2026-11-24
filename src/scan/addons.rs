//! Library add-ons scanner: `list` of every enabled library add-on (one thread each, each bounded
//! by its own timeout) → `Source::Addon` games with `source_id` = `<addon id>:<game id>`.
//!
//! - Launch: the entry's `launch` (`{"program","args","cwd","env"}` or `{"url"}`); without one, the
//!   add-on itself run as the game (`<exec> launch <id>`, see `AddonManager::direct_launch`).
//! - Art: local paths (absolute, `file://`, or relative to the add-on folder) that exist. http(s)
//!   URLs are not downloaded here (a scan never touches the network): an add-on that wants its
//!   art shown downloads it into `LODESTAR_ADDON_DATA` and returns those paths; otherwise the
//!   regular art look-up (Steam / SteamGridDB) fills missing art by name.
//! - One failing add-on is logged and kept as its `last_error`; the Addon source fails (keeping
//!   its previous games in the DB) only when every enabled library add-on failed.

use super::ScanContext;
use crate::addons::metadata::{art_from, i64_field, meta_from, str_field, u64_field};
use crate::addons::{AddonInfo, AddonKind, AddonManager};
use crate::model::*;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;

/// Most games taken from one add-on.
const MAX_GAMES: usize = 100_000;

pub fn scan(ctx: &ScanContext) -> anyhow::Result<Vec<ScannedGame>> {
    let mut mgr = AddonManager::new(ctx.data_dir.join("addons"));
    mgr.refresh();
    let libs: Vec<&AddonInfo> = mgr.enabled_of_kind(AddonKind::Library, &ctx.addons);
    if libs.is_empty() {
        return Ok(vec![]);
    }
    let request = serde_json::json!({ "include_uninstalled": ctx.config.include_uninstalled });
    let results: Vec<(&AddonInfo, anyhow::Result<Value>)> = std::thread::scope(|sc| {
        let handles: Vec<_> = libs
            .iter()
            .map(|&info| {
                let (mgr, request) = (&mgr, request.clone());
                sc.spawn(move || (info, mgr.call(&info.manifest.id, "list", request, &ctx.addons)))
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });

    let mut games = vec![];
    let mut errors = vec![];
    for (info, res) in results {
        match res {
            Ok(reply) => games.extend(map_reply(&mgr, info, &reply, ctx)),
            Err(e) => errors.push(format!("{e:#}")),
        }
    }
    if !errors.is_empty() && errors.len() == libs.len() {
        anyhow::bail!("{}", errors.join("; "));
    }
    Ok(games)
}

/// Map one add-on's `list` reply. Entries without an id or name are skipped; duplicate ids keep
/// the first.
pub fn map_reply(mgr: &AddonManager, info: &AddonInfo, reply: &Value, ctx: &ScanContext) -> Vec<ScannedGame> {
    let addon = &info.manifest.id;
    let Some(list) = reply.get("games").and_then(|g| g.as_array()) else {
        log::warn!("addon {addon}: list reply has no \"games\" array");
        return vec![];
    };
    let mut seen = HashSet::new();
    let mut out = vec![];
    for g in list.iter().take(MAX_GAMES) {
        let Some(gid) = game_id(g) else { continue };
        let Some(name) = str_field(g, "name", 300) else {
            continue;
        };
        if !seen.insert(gid.clone()) {
            continue;
        }
        let mut sg = ScannedGame::new(Source::Addon, format!("{addon}:{gid}"), name);
        sg.installed = g.get("installed").and_then(|v| v.as_bool()).unwrap_or(true);
        sg.install_dir = str_field(g, "install_dir", 4096).filter(|p| p.starts_with('/')).map(PathBuf::from);
        sg.launch = g
            .get("launch")
            .and_then(|l| crate::addons::parse_launch(l, &info.dir))
            .or_else(|| mgr.direct_launch(info, &gid, &ctx.addons));
        sg.art = art_from(g, &info.dir).local;
        sg.meta = meta_from(g);
        sg.playtime_secs = u64_field(g, "playtime_secs");
        sg.last_played = i64_field(g, "last_played").filter(|t| *t > 0);
        sg.size_bytes = u64_field(g, "size_bytes");
        sg.platform = str_field(g, "platform", 40).map(|p| p.to_lowercase());
        out.push(sg);
    }
    out
}

fn game_id(g: &Value) -> Option<String> {
    let id = match g.get("id")? {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!id.is_empty() && id.len() <= 256 && !id.chars().any(char::is_control)).then_some(id)
}
