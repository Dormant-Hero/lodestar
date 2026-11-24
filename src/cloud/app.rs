//! AppState side of cloud games: adding one (Add game → Cloud game…).

use super::{Entry, PLATFORM, Service, is_game_link};
use crate::app::AppState;
use crate::model::*;

/// What the Add dialog hands over.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewCloudGame {
    pub name: String,
    pub entry: Entry,
    /// The game's own page on the service (None: the service's home page).
    pub link: Option<String>,
}

impl NewCloudGame {
    /// The page the game opens (its link, else the service's home page).
    pub fn url(&self) -> Option<String> {
        self.link
            .as_deref()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .or_else(|| self.entry.service.home().map(str::to_string))
    }
}

impl AppState {
    /// Add a cloud game: a hand-added game (removable, editable like one) on platform "cloud",
    /// its service and how it opens in its source id, the game's link among its links. Then
    /// artwork / metadata are looked up by name like after a scan. Returns its id.
    pub fn add_cloud_game(&mut self, g: NewCloudGame) -> Option<GameId> {
        let name = g.name.trim().to_string();
        if name.is_empty() {
            self.notify_error("A game needs a name");
            return None;
        }
        let Some(url) = g.url().filter(|u| super::is_web(u)) else {
            self.notify_error("A cloud game needs a web link (https://…)");
            return None;
        };
        let nonce = format!("{}-{:x}", now_unix(), crate::db::fnv1a(name.as_bytes()) & 0xffff_ffff);
        let sid = g.entry.source_id(&nonce);
        let id = match self.db.add_manual_as(&sid, &name, LaunchSpec::Url(url.clone()), Art::default()) {
            Ok(id) => id,
            Err(e) => {
                self.notify_error(format!("Couldn't add {name}: {e:#}"));
                return None;
            }
        };
        let mut e = UserEdits { platform: Some(PLATFORM.into()), ..Default::default() };
        let service = g.entry.service;
        if is_game_link(service, &url) {
            let label = match service {
                Service::Other => "Play online".to_string(),
                s => format!("Play on {}", s.name()),
            };
            e.links.push(GameLink { name: label, url });
        }
        if let Err(err) = self.db.set_edits(id, &e) {
            self.notify_error(format!("{name} was added, but its details weren't saved: {err:#}"));
        }
        self.reload_library();
        // Artwork and details by name, as for games a scan finds.
        self.queue_metadata();
        self.auto_art_after_scan();
        Some(id)
    }

    /// Change how a cloud game opens (Edit → Installation): its service stays, "Opens with" and
    /// Full screen are stored in its source id like when it was added.
    pub fn set_cloud_entry(&mut self, id: GameId, entry: &Entry) {
        let Some(g) = self.library.get(id) else { return };
        let (Some(old), Some(nonce)) = (Entry::of(g), Entry::nonce(&g.source_id)) else { return };
        if old == *entry {
            return;
        }
        let entry = Entry { service: old.service, ..entry.clone() };
        let name = g.display_name().to_string();
        match self.db.set_manual_source_id(id, &entry.source_id(nonce)) {
            Ok(()) => self.reload_library(),
            Err(e) => self.notify_error(format!("Couldn't change how {name} opens: {e:#}")),
        }
    }

    /// Cloud games already in the library on `service` named like `name` (duplicate check).
    pub fn cloud_duplicates(&self, service: Service, name: &str) -> Vec<GameId> {
        let n = crate::addgame::norm_name(name);
        if n.is_empty() {
            return vec![];
        }
        self.library
            .games
            .iter()
            .filter(|g| Entry::of(g).is_some_and(|e| e.service == service))
            .filter(|g| crate::addgame::norm_name(g.display_name()) == n)
            .map(|g| g.id)
            .collect()
    }
}
