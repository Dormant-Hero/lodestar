//! The engine side of the extra artwork sources (GOG, Lutris.net, PCGamingWiki,
//! libretro-thumbnails, TheGamesDB, IGDB): credentials added only when a request
//! is sent (never in cached / logged URLs), answers cached per session, confident matching for
//! bulk runs and the automatic fill.

use super::*;
use crate::igdb;
use std::sync::Arc;

/// Most API answers kept per session (then the cache starts over).
const API_CACHE_MAX: usize = 400;
/// The libretro name lists are big (a few MB for the large systems).
const MAX_INDEX_BYTES: u64 = 24 * 1024 * 1024;
/// libretro name lists on disk are fresh this long.
const LR_INDEX_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(14 * 24 * 3600);
/// Games of a name search shown per source.
const HITS: usize = 3;

/// What a source looks a game up by.
#[derive(Clone, Debug)]
pub(crate) struct Query<'a> {
    pub name: &'a str,
    pub appid: Option<u32>,
    pub gog_id: Option<u64>,
    /// The console platform (libretro / TheGamesDB / IGDB ids), if it is one.
    pub console: Option<&'static Platform>,
    /// Emulated games: the ROM's No-Intro / Redump name and file name (libretro's exact names).
    pub rom_names: &'a [String],
    /// Emulated games: the serial the ROM header carries (GameTDB's game ID).
    pub serial: Option<&'a str>,
}

impl<'a> Query<'a> {
    pub fn of_search(j: &'a SearchJob) -> Query<'a> {
        Query {
            name: &j.name,
            appid: j.appid,
            gog_id: j.gog_id,
            console: console_platform(j.platform.as_deref()),
            rom_names: &j.rom_names,
            serial: j.serial.as_deref(),
        }
    }
    pub fn of_bulk(g: &'a BulkGame) -> Query<'a> {
        Query {
            name: &g.name,
            appid: g.appid,
            gog_id: g.gog_id,
            console: console_platform(g.platform.as_deref()),
            rom_names: &g.rom_names,
            serial: g.serial.as_deref(),
        }
    }
    /// The libretro-thumbnails system (consoles that have a repo; not Switch).
    pub fn libretro(&self) -> Option<&'static str> {
        self.console.and_then(|p| p.libretro)
    }
    /// GameTDB's (system, game ID): Wii / GameCube / Wii U / DS / 3DS games with a header serial.
    pub fn gametdb(&self) -> Option<(&'static str, String)> {
        gametdb_key(self.console.map(|p| p.id), self.serial)
    }
    /// Worth asking `k` in a search (libretro needs a console platform with a repo, GameTDB a
    /// game ID).
    pub fn searchable(&self, k: SourceKind) -> bool {
        match k {
            SourceKind::Libretro => self.libretro().is_some(),
            SourceKind::GameTdb => self.gametdb().is_some(),
            _ => true,
        }
    }
    /// Worth asking `k` for a confident match (bulk / automatic): console games aren't on the
    /// PC stores, and libretro only has console games.
    pub fn fits(&self, k: SourceKind) -> bool {
        match k {
            SourceKind::Libretro => self.libretro().is_some(),
            SourceKind::GameTdb => self.gametdb().is_some(),
            SourceKind::Steam | SourceKind::Gog | SourceKind::PcGamingWiki => self.console.is_none(),
            _ => true,
        }
    }
}

/// The user's region for GameTDB (from the locale; none in tests, so they don't depend on it).
fn gametdb_user_region() -> Option<&'static str> {
    if cfg!(test) { None } else { user_region() }
}

/// Pictures found: (the game's name at the source, picture).
type Found = Vec<(String, Pic)>;

/// Pictures of `kind` as editor candidates.
pub(crate) fn candidates(kind: SourceKind, slot: ArtSlot, found: Found) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter(|(_, p)| seen.insert(p.url.clone()))
        .map(|(game, p)| {
            let mut label = format!("{} · {game}", kind.name());
            if !p.note.is_empty() {
                label.push_str(" · ");
                label.push_str(&p.note);
            }
            Candidate {
                source: kind.name().into(),
                slot,
                url: p.url,
                thumb_url: p.thumb,
                width: p.width,
                height: p.height,
                label,
            }
        })
        .collect()
}

fn tag(name: &str, pics: Vec<Pic>) -> Found {
    pics.into_iter().map(|p| (name.to_string(), p)).collect()
}

impl Engine {
    // ----- credentials -----

    /// `url` with the user's credentials for the services that take them in the query
    /// (TheGamesDB). The result is sent only, never stored or logged.
    pub(crate) fn with_credentials(&self, url: &str) -> String {
        if is_tgdb_api(url)
            && let Some(k) = crate::config::secret(&self.cfg.lock().thegamesdb_key)
        {
            return format!("{url}&apikey={}", encode(k));
        }
        url.to_string()
    }

    /// `text` without any of the user's secrets in it.
    pub(crate) fn redact(&self, text: &str) -> String {
        let cfg = self.cfg.lock();
        let mut secrets = cfg.secrets();
        // Ids are not secret, but the encoded forms of the secrets may appear in URLs.
        secrets.extend(secrets.clone().iter().map(|s| encode(s)));
        crate::config::redact(text, &secrets)
    }

    // ----- cached API requests -----

    fn api_cached(&self, key: &str) -> Option<Option<Vec<u8>>> {
        self.cache.lock().api.get(key).cloned()
    }

    fn api_store(&self, key: String, v: Option<Vec<u8>>) {
        let mut c = self.cache.lock();
        if c.api.len() >= API_CACHE_MAX {
            c.api.clear();
        }
        c.api.insert(key, v);
    }

    /// GET a JSON API: Some(body) for 200, None for a definitive 4xx (cached for the session).
    /// `reject` maps statuses that mean "bad credentials" to that source.
    fn api_get(
        &self,
        url: &str,
        limit: u64,
        reject: Option<(SourceKind, &[u16])>,
        r: Req,
    ) -> Result<Option<Vec<u8>>, Fail> {
        if let Some(v) = self.api_cached(url) {
            return Ok(v);
        }
        let resp = self.get(url, None, limit, r)?;
        if let Some((k, codes)) = reject
            && codes.contains(&resp.status)
        {
            self.reject(k);
            return Err(Fail::BadKey(k));
        }
        let v = match resp.status {
            200 => Some(resp.body),
            _ => None,
        };
        self.api_store(url.to_string(), v.clone());
        Ok(v)
    }

    /// An IGDB query (POST with the app token; a token that stopped working is renewed once).
    fn igdb_post(&self, url: &str, body: &str, r: Req) -> Result<Option<Vec<u8>>, Fail> {
        let key = format!("{url}\n{body}");
        if let Some(v) = self.api_cached(&key) {
            return Ok(v);
        }
        let Some((id, secret)) = self.cfg.lock().igdb_credentials().map(|(a, b)| (a.to_string(), b.to_string())) else {
            return Err(Fail::BadKey(SourceKind::Igdb));
        };
        for attempt in 0..2 {
            let token = igdb::token(&id, &secret, |u| {
                if !self.throttle.wait(u, r.bulk, r.cancel) {
                    return Err("cancelled".into());
                }
                self.net.post(u, &[], "", MAX_JSON_BYTES).map(|x| (x.status, x.body)).map_err(|e| self.redact(&e))
            });
            let token = match token {
                Ok(t) => t,
                Err(igdb::TokenErr::BadCredentials) => {
                    self.reject(SourceKind::Igdb);
                    return Err(Fail::BadKey(SourceKind::Igdb));
                }
                Err(igdb::TokenErr::Transient(_)) if r.cancelled() => return Err(Fail::Cancelled),
                Err(igdb::TokenErr::Transient(e)) => return Err(Fail::Transient(e)),
            };
            self.gate(url, r)?;
            if !self.throttle.wait(url, r.bulk, r.cancel) {
                return Err(Fail::Cancelled);
            }
            let resp = self
                .net
                .post(url, &igdb::headers(&id, &token), body, MAX_JSON_BYTES)
                .map_err(|e| Fail::Transient(self.redact(&e)))?;
            self.limits.observe(
                Host::of(url),
                resp.status,
                &resp.headers,
                &resp.body,
                super::super::limits::unix_now(),
            );
            match resp.status {
                200 => {
                    self.api_store(key, Some(resp.body.clone()));
                    return Ok(Some(resp.body));
                }
                401 if attempt == 0 => igdb::forget_token(&id, &secret),
                401 | 403 => {
                    self.reject(SourceKind::Igdb);
                    return Err(Fail::BadKey(SourceKind::Igdb));
                }
                s if is_definitive(s) => {
                    self.api_store(key, None);
                    return Ok(None);
                }
                s => return Err(Fail::Transient(format!("HTTP {s}"))),
            }
        }
        Err(Fail::Transient("IGDB: the token was refused".into()))
    }

    fn parsed<T>(
        &self,
        body: Option<Vec<u8>>,
        what: &str,
        parse: impl FnOnce(&[u8]) -> Option<T>,
    ) -> Result<Option<T>, Fail> {
        match body {
            None => Ok(None),
            Some(b) => parse(&b).map(Some).ok_or_else(|| Fail::Transient(format!("{what}: unexpected response"))),
        }
    }

    // ----- one lookup per source -----

    /// Pictures of `kind` for `slot`. `confident`: only the game itself (bulk / automatic;
    /// Ok(None) = no confident match); else the top hits of a search (Ok(Some), maybe empty).
    pub(crate) fn find(
        &self,
        kind: SourceKind,
        q: &Query,
        slot: ArtSlot,
        confident: bool,
        r: Req,
    ) -> Result<Option<Found>, Fail> {
        if !kind.has_slot(slot) {
            return Ok(Some(vec![]));
        }
        match kind {
            SourceKind::Lutris => self.find_lutris(q, slot, confident, r),
            SourceKind::Gog => self.find_gog(q, slot, confident, r),
            SourceKind::PcGamingWiki => self.find_pcgw(q, confident, r),
            SourceKind::Libretro => self.find_libretro(q, slot, confident, r),
            SourceKind::TheGamesDb => self.find_tgdb(q, slot, confident, r),
            SourceKind::Igdb => self.find_igdb(q, slot, confident, r),
            SourceKind::GameTdb => self.find_gametdb(q, slot, confident, r),
            SourceKind::Steam | SourceKind::SteamGridDb => Ok(Some(vec![])),
        }
    }

    fn find_lutris(&self, q: &Query, slot: ArtSlot, confident: bool, r: Req) -> Result<Option<Found>, Fail> {
        let term = search_name_for(q.name);
        if term.is_empty() {
            return Ok(None);
        }
        let body = self.api_get(&lutris_search_url(&term), MAX_JSON_BYTES, None, r)?;
        let games = self.parsed(body, "Lutris", parse_lutris)?.unwrap_or_default();
        if confident {
            return Ok(confident_lutris(q.name, q.appid, &games).map(|g| tag(&g.name, lutris_pics(g, slot))));
        }
        let mut hits: Vec<&LutrisGame> = games.iter().collect();
        hits.sort_by_key(|g| !(same_name(q.name, &g.name) || q.appid.is_some_and(|a| g.steam.contains(&a))));
        Ok(Some(hits.into_iter().take(HITS * 2).flat_map(|g| tag(&g.name, lutris_pics(g, slot))).collect()))
    }

    fn gog_game(&self, id: u64, r: Req) -> Result<Option<GogProduct>, Fail> {
        let body = self.api_get(&gog_game_url(id), MAX_JSON_BYTES, None, r)?;
        self.parsed(body, "GOG", parse_gog_game)
    }

    fn gog_catalog(&self, name: &str, r: Req) -> Result<Vec<GogProduct>, Fail> {
        let term = search_name_for(name);
        if term.is_empty() {
            return Ok(vec![]);
        }
        let body = self.api_get(&gog_catalog_url(&term), MAX_JSON_BYTES, None, r)?;
        Ok(self.parsed(body, "GOG catalog", parse_gog_catalog)?.unwrap_or_default())
    }

    fn find_gog(&self, q: &Query, slot: ArtSlot, confident: bool, r: Req) -> Result<Option<Found>, Fail> {
        // The game's own product (GOG games), from the catalog for the wide capsule.
        let own = |p: GogProduct, this: &Self| -> Result<GogProduct, Fail> {
            let mut p = p;
            if slot == ArtSlot::Banner && p.horizontal.is_none() {
                let name = if p.title.is_empty() { q.name } else { p.title.as_str() };
                if let Some(c) = this.gog_catalog(name, r)?.into_iter().find(|c| c.id == p.id) {
                    p.horizontal = c.horizontal;
                }
            }
            Ok(p)
        };
        if let Some(id) = q.gog_id
            && let Some(mut p) = self.gog_game(id, r)?
        {
            p.id = id;
            let p = own(p, self)?;
            let name = if p.title.is_empty() { q.name.to_string() } else { p.title.clone() };
            let mut found = tag(&name, gog_pics(&p, slot));
            if !confident && found.is_empty() {
                found = self.gog_search_hits(q, slot, r)?;
            }
            return Ok(Some(found));
        }
        if confident {
            let products = self.gog_catalog(q.name, r)?;
            let Some(p) = confident_gog(q.name, &products).cloned() else {
                return Ok(None);
            };
            let mut p = p;
            if slot == ArtSlot::Icon
                && let Some(full) = self.gog_game(p.id, r)?
            {
                p.icon = full.icon;
            }
            return Ok(Some(tag(&p.title, gog_pics(&p, slot))));
        }
        self.gog_search_hits(q, slot, r).map(Some)
    }

    fn gog_search_hits(&self, q: &Query, slot: ArtSlot, r: Req) -> Result<Found, Fail> {
        let mut products = self.gog_catalog(q.name, r)?;
        products.sort_by_key(|p| !same_name(q.name, &p.title));
        products.truncate(HITS);
        let mut out = vec![];
        for mut p in products {
            if slot == ArtSlot::Icon
                && let Some(full) = self.gog_game(p.id, r)?
            {
                p.icon = full.icon;
            }
            out.extend(tag(&p.title, gog_pics(&p, slot)));
        }
        Ok(out)
    }

    fn pcgw_page(&self, title: &str, r: Req) -> Result<Option<PcgwPage>, Fail> {
        let body = self.api_get(&pcgw_page_url(title), MAX_JSON_BYTES, None, r)?;
        Ok(self.parsed(body, "PCGamingWiki", parse_pcgw_page)?.flatten())
    }

    fn pcgw_cover(&self, page: &PcgwPage, r: Req) -> Result<Option<Pic>, Fail> {
        let Some(file) = &page.cover else {
            return Ok(None);
        };
        let body = self.api_get(&pcgw_file_url(file), MAX_JSON_BYTES, None, r)?;
        Ok(self.parsed(body, "PCGamingWiki", parse_pcgw_file)?.flatten())
    }

    fn find_pcgw(&self, q: &Query, confident: bool, r: Req) -> Result<Option<Found>, Fail> {
        let title = search_name_for(q.name);
        if title.is_empty() {
            return Ok(None);
        }
        let direct = self.pcgw_page(&title, r)?;
        if confident {
            let Some(p) = direct.filter(|p| pcgw_confident(q.name, q.appid, p)) else {
                return Ok(None);
            };
            return Ok(Some(self.pcgw_cover(&p, r)?.into_iter().map(|c| (p.title.clone(), c)).collect()));
        }
        let mut pages: Vec<PcgwPage> = direct.into_iter().collect();
        let body = self.api_get(&pcgw_search_url(&title), MAX_JSON_BYTES, None, r)?;
        let titles = self.parsed(body, "PCGamingWiki search", parse_pcgw_search)?.unwrap_or_default();
        for t in titles.into_iter().take(HITS) {
            if pages.iter().any(|p| p.title == t) {
                continue;
            }
            if let Some(p) = self.pcgw_page(&t, r)? {
                pages.push(p);
            }
        }
        let mut out = vec![];
        for p in pages.iter().take(HITS + 1) {
            if let Some(c) = self.pcgw_cover(p, r)? {
                out.push((p.title.clone(), c));
            }
        }
        Ok(Some(out))
    }

    /// The box art names of a libretro system (memory, then disk, then GitHub).
    fn libretro_index(&self, system: &str, r: Req) -> Result<Arc<LrIndex>, Fail> {
        if let Some(v) = self.cache.lock().lr_index.get(system) {
            return Ok(v.clone());
        }
        let file = libretro_index_dir(&self.thumbs_dir).join(format!("{}.txt", libretro_repo(system)));
        let fresh = std::fs::metadata(&file)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age < LR_INDEX_MAX_AGE);
        let mut names: Option<Vec<String>> = None;
        if fresh && let Ok(text) = std::fs::read_to_string(&file) {
            names = Some(text.lines().filter(|l| !l.is_empty()).map(str::to_string).collect());
        }
        let names = match names {
            Some(n) => n,
            None => {
                let url = libretro_index_url(system);
                let mut resp = self.get(&url, None, MAX_INDEX_BYTES, r)?;
                // GitHub's hourly limit hit after all: a patient bulk run waits for the reset.
                if matches!(resp.status, 403 | 429) && r.patient {
                    resp = self.get(&url, None, MAX_INDEX_BYTES, r)?;
                }
                match resp.status {
                    200 => {
                        let n = parse_libretro_tree(&resp.body)
                            .ok_or_else(|| Fail::Transient("libretro: unexpected response".into()))?;
                        if let Err(e) = write_atomic(&file, n.join("\n").as_bytes()) {
                            log::debug!("libretro index {}: {e}", file.display());
                        }
                        n
                    }
                    // GitHub's hourly limit (403/429 with a message): try again later.
                    403 | 429 => {
                        use super::super::limits::{Verdict, unix_now};
                        return Err(match self.limits.check(Host::GitHubApi, false, unix_now()) {
                            Verdict::Stop(why) | Verdict::Wait(_, why) => Fail::Limit(why),
                            Verdict::Go => Fail::Transient("GitHub rate limit; try again in an hour".into()),
                        });
                    }
                    _ => vec![],
                }
            }
        };
        let names = Arc::new(LrIndex::new(names));
        self.cache.lock().lr_index.insert(system.to_string(), names.clone());
        Ok(names)
    }

    fn find_libretro(&self, q: &Query, slot: ArtSlot, confident: bool, r: Req) -> Result<Option<Found>, Fail> {
        let Some(system) = q.libretro() else {
            return Ok(if confident { None } else { Some(vec![]) });
        };
        let names = match self.libretro_index(system, r) {
            Ok(n) => n,
            Err(Fail::Limit(why)) if confident => return self.libretro_exact(q, system, slot, why, r),
            Err(e) => return Err(e),
        };
        let stems: Vec<&str> = if confident {
            names.matches_rom(q.rom_names, q.name).into_iter().take(1).collect()
        } else {
            // The ROM's own image first, then what the name finds.
            let mut s: Vec<&str> = names.matches_rom(q.rom_names, q.name).into_iter().take(1).collect();
            for hit in names.search(q.name, 8) {
                if !s.contains(&hit) && s.len() < 8 {
                    s.push(hit);
                }
            }
            s
        };
        if stems.is_empty() {
            return Ok(if confident { None } else { Some(vec![]) });
        }
        let mut out = vec![];
        for s in stems {
            for pic in libretro_pics(system, s, slot) {
                // Not every name has a snap / title screen: only offer the ones that exist.
                if !confident && slot != ArtSlot::Cover && !self.exists(&pic.url, r)? {
                    continue;
                }
                out.push((nointro_title(s), pic));
            }
        }
        Ok(Some(out))
    }

    /// libretro while its name list is out of reach (the list comes from GitHub's API, which
    /// allows 60 requests an hour per address; the images come from a host without that limit):
    /// the ROM's own No-Intro / Redump names, exactly as the image files are named. A game none
    /// of them fits is asked again once the list can be had (`Fail::Later`), as its title may
    /// still match an image of another region.
    fn libretro_exact(
        &self,
        q: &Query,
        system: &str,
        slot: ArtSlot,
        why: String,
        r: Req,
    ) -> Result<Option<Found>, Fail> {
        for n in q.rom_names {
            let stem = libretro_file_name(n.trim());
            let cover = libretro_image_url(system, LR_BOXART, &stem);
            if !stem.is_empty() && self.exists(&cover, r)? {
                return Ok(Some(tag(&nointro_title(&stem), libretro_pics(system, &stem, slot))));
            }
        }
        Err(Fail::Later(why))
    }

    /// GameTDB by the game ID: the first region (see `regions`) that has the slot's main
    /// picture; a search also offers that region's other kinds (3D box, full box).
    fn find_gametdb(&self, q: &Query, slot: ArtSlot, confident: bool, r: Req) -> Result<Option<Found>, Fail> {
        let none = || if confident { None } else { Some(vec![]) };
        let Some((system, id)) = q.gametdb() else {
            return Ok(none());
        };
        let kinds = gametdb_kinds(system, slot);
        let Some(main) = kinds.first() else {
            return Ok(none());
        };
        let mut region = None;
        for reg in regions(&id, gametdb_user_region()) {
            if self.exists(&gametdb_pic(system, main, reg, &id).url, r)? {
                region = Some(reg);
                break;
            }
        }
        let Some(reg) = region else {
            return Ok(none());
        };
        let mut out = vec![(q.name.to_string(), gametdb_pic(system, main, reg, &id))];
        if !confident {
            for k in &kinds[1..] {
                let pic = gametdb_pic(system, k, reg, &id);
                if self.exists(&pic.url, r)? {
                    out.push((q.name.to_string(), pic));
                }
            }
        }
        Ok(Some(out))
    }

    fn tgdb_reject() -> Option<(SourceKind, &'static [u16])> {
        Some((SourceKind::TheGamesDb, &[401, 403]))
    }

    fn tgdb_pics(&self, g: &TgdbGame, slot: ArtSlot, r: Req) -> Result<Vec<Pic>, Fail> {
        if slot == ArtSlot::Cover {
            return Ok(g.boxart.iter().cloned().collect());
        }
        let Some(kind) = tgdb_kind(slot) else {
            return Ok(vec![]);
        };
        let body = self.api_get(&tgdb_images_url(g.id), MAX_JSON_BYTES, Self::tgdb_reject(), r)?;
        let imgs = self.parsed(body, "TheGamesDB", |b| parse_tgdb_images(b, g.id))?.unwrap_or_default();
        Ok(imgs.into_iter().filter(|(k, _)| k == kind).map(|(_, p)| p).collect())
    }

    fn find_tgdb(&self, q: &Query, slot: ArtSlot, confident: bool, r: Req) -> Result<Option<Found>, Fail> {
        let term = search_name_for(q.name);
        if term.is_empty() {
            return Ok(None);
        }
        let url = tgdb_search_url(&term, q.console.and_then(|p| p.tgdb));
        let body = self.api_get(&url, MAX_JSON_BYTES, Self::tgdb_reject(), r)?;
        let games = self.parsed(body, "TheGamesDB", parse_tgdb_games)?.unwrap_or_default();
        if confident {
            let Some(g) = confident_tgdb(q.name, &games, q.console.is_none()) else {
                return Ok(None);
            };
            return Ok(Some(tag(&g.title, self.tgdb_pics(g, slot, r)?)));
        }
        let mut hits: Vec<&TgdbGame> = games.iter().collect();
        hits.sort_by_key(|g| !same_name(q.name, &g.title));
        let mut out = vec![];
        for g in hits.into_iter().take(HITS) {
            out.extend(tag(&g.title, self.tgdb_pics(g, slot, r)?));
        }
        Ok(Some(out))
    }

    /// IGDB games for the query: by Steam appid (external games) when known, else by name.
    pub(crate) fn igdb_games(&self, q: &Query, r: Req) -> Result<(Vec<igdb::IgdbGame>, bool), Fail> {
        if let Some(appid) = q.appid {
            let ids = self
                .igdb_post(&igdb::external_games_url(), &igdb::steam_body(appid), r)?
                .and_then(|b| igdb::parse_external(&b))
                .unwrap_or_default();
            if !ids.is_empty() {
                let body = self.igdb_post(&igdb::games_url(), &igdb::by_ids_body(&ids), r)?;
                let games = self.parsed(body, "IGDB", igdb::parse_games)?.unwrap_or_default();
                if !games.is_empty() {
                    return Ok((games, true));
                }
            }
        }
        let term = search_name_for(q.name);
        if term.is_empty() {
            return Ok((vec![], false));
        }
        let body =
            self.igdb_post(&igdb::games_url(), &igdb::search_body_on(&term, q.console.and_then(|p| p.igdb)), r)?;
        Ok((self.parsed(body, "IGDB", igdb::parse_games)?.unwrap_or_default(), false))
    }

    fn find_igdb(&self, q: &Query, slot: ArtSlot, confident: bool, r: Req) -> Result<Option<Found>, Fail> {
        let (games, by_id) = self.igdb_games(q, r)?;
        let pics = |g: &igdb::IgdbGame| -> Vec<Pic> {
            match slot {
                ArtSlot::Cover => g
                    .cover
                    .iter()
                    .map(|c| Pic {
                        url: c.url("cover_big_2x"),
                        thumb: Some(c.url("cover_big")),
                        width: 528,
                        height: 748,
                        note: String::new(),
                    })
                    .collect(),
                ArtSlot::Hero => g
                    .artworks
                    .iter()
                    .map(|a| (a, "artwork"))
                    .chain(g.screenshots.iter().map(|s| (s, "screenshot")))
                    .map(|(a, note)| Pic {
                        url: a.url("1080p"),
                        thumb: Some(a.url("screenshot_med")),
                        width: 1920,
                        height: 1080,
                        note: note.into(),
                    })
                    .collect(),
                _ => vec![],
            }
        };
        if confident {
            let g = if by_id { games.first() } else { igdb::confident(q.name, &games) };
            return Ok(g.map(|g| tag(&g.name, pics(g))));
        }
        let mut hits: Vec<&igdb::IgdbGame> = games.iter().collect();
        hits.sort_by_key(|g| !(by_id || same_name(q.name, &g.name)));
        Ok(Some(hits.into_iter().take(HITS).flat_map(|g| tag(&g.name, pics(g))).collect()))
    }

    // ----- bulk -----

    /// Fill `need` from one of the 4f sources (confident match only). Ok(matched).
    pub(crate) fn bulk_other(
        &self,
        kind: SourceKind,
        g: &BulkGame,
        q: &Query,
        need: &mut Vec<ArtSlot>,
        r: Req,
        on_applied: &mut dyn FnMut(ArtSlot, PathBuf),
    ) -> Result<bool, Fail> {
        let mut matched = false;
        for slot in need.clone() {
            if !kind.has_slot(slot) {
                continue;
            }
            let Some(found) = self.find(kind, q, slot, true, r)? else {
                // No such game at this source: the other slots won't find it either.
                return Ok(matched);
            };
            matched = true;
            let urls: Vec<String> = found.into_iter().take(3).map(|(_, p)| p.url).collect();
            if let Some(p) = self.bulk_download(g, slot, &urls, r)? {
                on_applied(slot, p);
                need.retain(|s| *s != slot);
            }
        }
        Ok(matched)
    }
}

/// `<cache>/libretro-index` next to the thumbnail folder.
pub(crate) fn libretro_index_dir(thumbs_dir: &Path) -> PathBuf {
    thumbs_dir.with_file_name("libretro-index")
}
