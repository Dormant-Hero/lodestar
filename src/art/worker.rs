//! The artwork engine: source lookups with session caches, thumbnails, applying images and bulk
//! runs. Shared by the art threads (`ArtService`); every request goes through the polite
//! process-wide `Throttle`. No method here touches the UI.

use super::net::*;
use super::sources::*;
use super::*;
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

mod more;
pub(crate) use more::{Query, candidates};

/// Why a lookup produced nothing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Fail {
    Cancelled,
    Transient(String),
    /// The source rejected the user's key / credentials (401/403).
    BadKey(SourceKind),
    /// The source's API limit is reached (or nearly, in bulk runs): why and when it continues.
    Limit(String),
    /// The source can't settle this game now, though it keeps answering others (libretro
    /// without its name list): asked again another time.
    Later(String),
}

impl From<NetErr> for Fail {
    fn from(e: NetErr) -> Fail {
        match e {
            NetErr::Cancelled => Fail::Cancelled,
            NetErr::Transient(s) => Fail::Transient(s),
            NetErr::Limit(s) => Fail::Limit(s),
        }
    }
}

impl std::fmt::Display for Fail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fail::Cancelled => write!(f, "cancelled"),
            Fail::Transient(e) | Fail::Limit(e) | Fail::Later(e) => write!(f, "{e}"),
            Fail::BadKey(_) => write!(f, "the API key was rejected (check Settings → Metadata)"),
        }
    }
}

/// What `Engine::check_sgdb_key` searches for: any name SteamGridDB surely knows.
const KEY_CHECK_TERM: &str = "Portal";

/// Request context: bulk runs are slower (≤ 2 req/s) and cancellable.
#[derive(Clone, Copy)]
pub(crate) struct Req<'a> {
    pub bulk: bool,
    pub cancel: Option<&'a AtomicBool>,
    /// May wait for an API limit to reset (bulk downloads; not the quiet automatic fill).
    pub patient: bool,
}

impl Req<'_> {
    pub const UI: Req<'static> = Req { bulk: false, cancel: None, patient: false };
    fn cancelled(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed))
    }
}

#[derive(Default)]
pub(crate) struct Caches {
    exists: HashMap<String, bool>,
    store_search: HashMap<String, Vec<StoreHit>>,
    get_items: HashMap<u32, Option<SteamAssets>>,
    appdetails_header: HashMap<u32, Option<String>>,
    sgdb_search: HashMap<String, Vec<SgdbGame>>,
    /// None = SteamGridDB doesn't know the target.
    sgdb_assets: HashMap<(String, ArtSlot), Option<Vec<SgdbAsset>>>,
    /// Answers of the other sources' APIs by request (URL, or URL + POST body); None = a
    /// definitive "no such thing" (4xx). Bounded (`API_CACHE_MAX`).
    pub(crate) api: HashMap<String, Option<Vec<u8>>>,
    /// libretro-thumbnails box art names per system.
    pub(crate) lr_index: HashMap<String, std::sync::Arc<LrIndex>>,
}

/// Result of fetching a full image.
enum Fetched {
    Image(Vec<u8>, &'static str),
    /// Definitive 4xx.
    Missing(u16),
    /// 200, but not an image Lodestar can show.
    NotImage(String),
}

/// What to search for.
#[derive(Clone, Debug)]
pub(crate) struct SearchJob {
    pub game: GameId,
    pub slot: ArtSlot,
    pub name: String,
    pub appid: Option<u32>,
    /// GOG product id (GOG games; `gog:<id>` typed as the query).
    pub gog_id: Option<u64>,
    /// The game's platform as the library knows it (console sources use it).
    pub platform: Option<String>,
    /// Emulated games: the ROM's No-Intro / Redump name and file name (libretro's exact names).
    pub rom_names: Vec<String>,
    /// Emulated games: the ROM header's serial (GameTDB's game ID).
    pub serial: Option<String>,
    /// The source list of Settings → Metadata the game uses.
    pub list: ArtList,
}

impl SearchJob {
    /// A query of only digits (or `steam:<id>`) means a Steam appid, `gog:<id>` a GOG product;
    /// any other query a name.
    pub fn new(g: &Game, slot: ArtSlot, query: Option<&str>) -> SearchJob {
        let q = query.map(str::trim).filter(|q| !q.is_empty());
        let mut gog_id = None;
        let (name, appid) = match q {
            Some(q) => {
                let id = q.strip_prefix("steam:").unwrap_or(q);
                if let Some(gid) = q.strip_prefix("gog:").and_then(|s| s.trim().parse::<u64>().ok()) {
                    gog_id = Some(gid);
                    (g.display_name().to_string(), None)
                } else {
                    match id.parse::<u32>().ok().and_then(store_appid) {
                        Some(a) if id.bytes().all(|b| b.is_ascii_digit()) => (g.display_name().to_string(), Some(a)),
                        _ => (q.to_string(), None),
                    }
                }
            }
            None => {
                gog_id = gog_product_id(g);
                (g.display_name().to_string(), steam_appid(g))
            }
        };
        SearchJob {
            game: g.id,
            slot,
            name,
            appid,
            gog_id,
            platform: sources::art_platform(g),
            // A typed name is what the user wants found.
            rom_names: if q.is_none() { sources::rom_names(g) } else { vec![] },
            serial: if q.is_none() { sources::rom_serial(g) } else { None },
            list: sources::art_list(g),
        }
    }
}

/// One game of a bulk run.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BulkGame {
    pub id: GameId,
    pub name: String,
    pub appid: Option<u32>,
    pub gog_id: Option<u64>,
    pub platform: Option<String>,
    /// See `SearchJob::rom_names`.
    pub rom_names: Vec<String>,
    /// See `SearchJob::serial`.
    pub serial: Option<String>,
    pub need: Vec<ArtSlot>,
    /// Folder the images go to (`<dir>/<slot>.<ext>`); None = the game's user art folder.
    pub dest: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct BulkReport {
    pub changed: usize,
    pub failed: usize,
    pub not_found: Vec<String>,
    pub cancelled: bool,
    pub error: Option<String>,
}

pub(crate) struct Engine {
    pub net: Box<dyn Net>,
    pub throttle: Throttle,
    pub art_dir: PathBuf,
    pub thumbs_dir: PathBuf,
    pub cfg: Mutex<MetadataConfig>,
    /// Sources that rejected the user's credentials this session (until they change).
    pub rejected: Mutex<HashSet<SourceKind>>,
    pub(crate) cache: Mutex<Caches>,
    /// The APIs' quotas and pauses (saved in `<data>/api-limits.json`).
    pub limits: super::limits::Limits,
}

impl Engine {
    pub fn new(
        net: Box<dyn Net>,
        throttle: Throttle,
        art_dir: PathBuf,
        thumbs_dir: PathBuf,
        cfg: &MetadataConfig,
    ) -> Engine {
        let limits = super::limits::Limits::new(Some(limits_file(&art_dir)));
        Engine {
            net,
            throttle,
            art_dir,
            thumbs_dir,
            cfg: Mutex::new(cfg.clone()),
            rejected: Mutex::new(HashSet::new()),
            cache: Mutex::new(Caches::default()),
            limits,
        }
    }

    pub fn set_config(&self, cfg: &MetadataConfig) {
        let mut cur = self.cfg.lock();
        let mut rej = self.rejected.lock();
        let mut c = self.cache.lock();
        if cur.steamgriddb_key != cfg.steamgriddb_key {
            rej.remove(&SourceKind::SteamGridDb);
            c.sgdb_search.clear();
            c.sgdb_assets.clear();
        }
        let changed = |a: &MetadataConfig, b: &MetadataConfig, k: SourceKind| match k {
            SourceKind::Igdb => {
                (&a.igdb_client_id, &a.igdb_client_secret) != (&b.igdb_client_id, &b.igdb_client_secret)
            }
            SourceKind::TheGamesDb => a.thegamesdb_key != b.thegamesdb_key,
            _ => false,
        };
        for k in [SourceKind::Igdb, SourceKind::TheGamesDb] {
            if changed(&cur, cfg, k) {
                rej.remove(&k);
                // Answers that depend on the account (TheGamesDB 403).
                c.api.clear();
            }
        }
        *cur = cfg.clone();
    }

    pub fn is_rejected(&self, k: SourceKind) -> bool {
        self.rejected.lock().contains(&k)
    }

    pub(crate) fn reject(&self, k: SourceKind) {
        log::warn!("{} rejected the credentials; skipping it this session", k.name());
        self.rejected.lock().insert(k);
    }

    /// The SteamGridDB key, unless missing or rejected this session.
    pub fn sgdb_key(&self) -> Option<String> {
        if self.is_rejected(SourceKind::SteamGridDb) {
            return None;
        }
        crate::config::secret(&self.cfg.lock().steamgriddb_key).map(str::to_string)
    }

    /// Every source of the PC list (see `sources_for`).
    pub fn sources(&self) -> Vec<SourceInfo> {
        self.sources_for(ArtList::Pc)
    }

    /// Every source of `list` in the user's order: available (switched on, configured, not
    /// rejected) or why not. Switched-off sources are listed last.
    pub fn sources_for(&self, list: ArtList) -> Vec<SourceInfo> {
        let cfg = self.cfg.lock().clone();
        let mut on = vec![];
        let mut off = vec![];
        for (k, enabled) in ordered_sources_for(&cfg, list) {
            let reason = if !enabled {
                Some("Switched off in Settings → Metadata".to_string())
            } else if !k.configured(&cfg) {
                Some(k.needs().to_string())
            } else if self.is_rejected(k) {
                Some(format!("{} rejected the key (check Settings → Metadata)", k.name()))
            } else {
                None
            };
            let info = SourceInfo { name: k.name().into(), available: reason.is_none(), reason, kind: k, enabled };
            if enabled { on.push(info) } else { off.push(info) }
        }
        on.extend(off);
        on
    }

    /// Available sources of `list`, in priority order.
    pub fn available_sources(&self, list: ArtList) -> Vec<SourceKind> {
        self.sources_for(list).into_iter().filter(|s| s.available).map(|s| s.kind).collect()
    }

    // ----- HTTP -----

    /// Keep to `url`'s API limits before sending: Err when a limit stops it (a patient bulk
    /// run waits instead, saying why in the status bar).
    pub(crate) fn gate(&self, url: &str, r: Req) -> Result<(), NetErr> {
        use super::limits::{Verdict, sleep_cancellable, unix_now};
        let host = Host::of(url);
        loop {
            match self.limits.check(host, r.bulk, unix_now()) {
                Verdict::Go => break,
                Verdict::Wait(secs, why) if r.patient => {
                    self.limits.set_waiting(Some(why));
                    let ok = sleep_cancellable(std::time::Duration::from_secs(secs.clamp(1, 30) as u64), r.cancel);
                    if !ok {
                        self.limits.set_waiting(None);
                        return Err(NetErr::Cancelled);
                    }
                }
                Verdict::Wait(_, why) | Verdict::Stop(why) => {
                    self.limits.set_waiting(None);
                    return Err(NetErr::Limit(why));
                }
            }
        }
        self.limits.set_waiting(None);
        Ok(())
    }

    /// GET; Ok only for 200 or a definitive 4xx ("no such thing").
    fn get(&self, url: &str, bearer: Option<&str>, limit: u64, r: Req) -> Result<HttpResponse, NetErr> {
        self.gate(url, r)?;
        if !self.throttle.wait(url, r.bulk, r.cancel) {
            return Err(NetErr::Cancelled);
        }
        let real = self.with_credentials(url);
        let resp = self.net.get(&real, bearer, limit).map_err(|e| NetErr::Transient(self.redact(&e)))?;
        self.limits.observe(Host::of(url), resp.status, &resp.headers, &resp.body, super::limits::unix_now());
        match resp.status {
            200 => Ok(resp),
            s if is_definitive(s) || (300..400).contains(&s) => Ok(resp),
            s => Err(NetErr::Transient(format!("HTTP {s}"))),
        }
    }

    /// Does `url` exist? (HEAD, cached for the session.)
    pub fn exists(&self, url: &str, r: Req) -> Result<bool, NetErr> {
        if let Some(&e) = self.cache.lock().exists.get(url) {
            return Ok(e);
        }
        self.gate(url, r)?;
        if !self.throttle.wait(url, r.bulk, r.cancel) {
            return Err(NetErr::Cancelled);
        }
        let status = self.net.head(&self.with_credentials(url)).map_err(|e| NetErr::Transient(self.redact(&e)))?;
        let e = match status {
            200..=299 => true,
            // Server doesn't do HEAD: assume it's there (the thumbnail GET decides).
            405 | 501 => true,
            s if is_definitive(s) || (300..400).contains(&s) => false,
            s => return Err(NetErr::Transient(format!("HTTP {s}"))),
        };
        self.cache.lock().exists.insert(url.to_string(), e);
        Ok(e)
    }

    // ----- Steam -----

    pub fn store_search(&self, term: &str, r: Req) -> Result<Vec<StoreHit>, Fail> {
        let key = term.trim().to_lowercase();
        if key.is_empty() {
            return Ok(vec![]);
        }
        if let Some(h) = self.cache.lock().store_search.get(&key) {
            return Ok(h.clone());
        }
        let resp = self.get(&storesearch_url(term), None, MAX_JSON_BYTES, r)?;
        let hits = if resp.status == 200 {
            parse_storesearch(&resp.body).ok_or_else(|| Fail::Transient("Steam search: unexpected response".into()))?
        } else {
            vec![]
        };
        self.cache.lock().store_search.insert(key, hits.clone());
        Ok(hits)
    }

    /// Asset lists for `appids` (GetItems, batched, cached). Unknown apps map to None.
    pub fn steam_assets(&self, appids: &[u32], r: Req) -> Result<HashMap<u32, Option<SteamAssets>>, Fail> {
        let missing: Vec<u32> = {
            let c = self.cache.lock();
            let mut seen = HashSet::new();
            appids.iter().copied().filter(|a| !c.get_items.contains_key(a) && seen.insert(*a)).collect()
        };
        for chunk in missing.chunks(GET_ITEMS_CHUNK) {
            let resp = self.get(&get_items_url(chunk), None, MAX_JSON_BYTES, r)?;
            let parsed = if resp.status == 200 { parse_get_items(&resp.body) } else { Some(HashMap::new()) };
            let Some(mut parsed) = parsed else {
                return Err(Fail::Transient("Steam GetItems: unexpected response".into()));
            };
            let mut c = self.cache.lock();
            for a in chunk {
                c.get_items.insert(*a, parsed.remove(a).flatten());
            }
        }
        let c = self.cache.lock();
        Ok(appids.iter().map(|a| (*a, c.get_items.get(a).cloned().flatten())).collect())
    }

    /// Assets of one app; network trouble = None (the classic CDN paths are tried instead).
    fn assets_or_none(&self, appid: u32, r: Req) -> Result<Option<SteamAssets>, Fail> {
        match self.steam_assets(&[appid], r) {
            Ok(mut m) => Ok(m.remove(&appid).flatten()),
            Err(Fail::Cancelled) => Err(Fail::Cancelled),
            Err(e) => {
                log::debug!("art: GetItems {appid}: {e}");
                Ok(None)
            }
        }
    }

    pub fn appdetails_header(&self, appid: u32, r: Req) -> Result<Option<String>, Fail> {
        if let Some(h) = self.cache.lock().appdetails_header.get(&appid) {
            return Ok(h.clone());
        }
        let resp = self.get(&appdetails_url(appid), None, MAX_JSON_BYTES, r)?;
        let h = if resp.status == 200 {
            parse_appdetails_header(&resp.body, appid)
                .ok_or_else(|| Fail::Transient("Steam appdetails: unexpected response".into()))?
        } else {
            None
        };
        self.cache.lock().appdetails_header.insert(appid, h.clone());
        Ok(h)
    }

    /// The existing Steam picture for (appid, slot), as a candidate.
    pub fn steam_candidate(&self, appid: u32, name: &str, slot: ArtSlot, r: Req) -> Result<Option<Candidate>, Fail> {
        let assets = self.assets_or_none(appid, r)?;
        let label_name = assets.as_ref().map(|a| a.name.as_str()).filter(|n| !n.is_empty()).unwrap_or(name);
        let mk = |o: &UrlOpt| Candidate {
            source: SRC_STEAM.into(),
            slot,
            url: o.url.clone(),
            thumb_url: o.thumb.clone(),
            width: o.width,
            height: o.height,
            label: format!("{SRC_STEAM} · {label_name}"),
        };
        for o in steam_urls(appid, assets.as_ref(), slot) {
            if self.exists(&o.url, r)? {
                let mut c = mk(&o);
                if let Some(t) = &o.thumb
                    && !self.exists(t, r)?
                {
                    c.thumb_url = None;
                }
                return Ok(Some(c));
            }
        }
        if slot == ArtSlot::Banner
            && let Some(u) = self.appdetails_header(appid, r)?
            && self.exists(&u, r)?
        {
            return Ok(Some(mk(&UrlOpt { url: u, thumb: None, width: 460, height: 215 })));
        }
        Ok(None)
    }

    /// Steam candidates: by appid, or the top name-search hits (exact name match first).
    fn steam_search(&self, job: &SearchJob, r: Req) -> Result<Vec<Candidate>, Fail> {
        if let Some(appid) = job.appid {
            return Ok(self.steam_candidate(appid, &job.name, job.slot, r)?.into_iter().collect());
        }
        let mut hits = self.store_search(&job.name, r)?;
        hits.truncate(TOP_HITS);
        hits.sort_by_key(|h| !same_name(&job.name, &h.name));
        let ids: Vec<u32> = hits.iter().map(|h| h.appid).collect();
        if let Err(Fail::Cancelled) = self.steam_assets(&ids, r) {
            return Err(Fail::Cancelled);
        }
        let mut out = vec![];
        for h in &hits {
            if let Some(c) = self.steam_candidate(h.appid, &h.name, job.slot, r)? {
                out.push(c);
            }
        }
        Ok(out)
    }

    // ----- SteamGridDB -----

    fn sgdb_get(&self, url: &str, key: &str, r: Req) -> Result<HttpResponse, Fail> {
        let resp = self.get(url, Some(key), MAX_JSON_BYTES, r)?;
        if matches!(resp.status, 401 | 403) {
            self.reject(SourceKind::SteamGridDb);
            return Err(Fail::BadKey(SourceKind::SteamGridDb));
        }
        Ok(resp)
    }

    /// Does SteamGridDB accept `key`? One small search with it, before the key is saved (a
    /// rejection here doesn't mark the saved key as rejected).
    pub fn check_sgdb_key(&self, key: &str) -> KeyCheck {
        match self.get(&sgdb_search_url(KEY_CHECK_TERM), Some(key.trim()), MAX_JSON_BYTES, Req::UI) {
            Ok(r) if r.status == 200 && parse_sgdb_games(&r.body).is_some() => KeyCheck::Works,
            Ok(r) if matches!(r.status, 401 | 403) => KeyCheck::Rejected,
            Ok(r) => KeyCheck::Unreachable(format!("SteamGridDB answered HTTP {}", r.status)),
            Err(e) => KeyCheck::Unreachable(e.to_string()),
        }
    }

    pub fn sgdb_games(&self, term: &str, key: &str, r: Req) -> Result<Vec<SgdbGame>, Fail> {
        let term = search_name_for(term);
        let ck = term.to_lowercase();
        if ck.is_empty() {
            return Ok(vec![]);
        }
        if let Some(g) = self.cache.lock().sgdb_search.get(&ck) {
            return Ok(g.clone());
        }
        let resp = self.sgdb_get(&sgdb_search_url(&term), key, r)?;
        let games = if resp.status == 200 {
            parse_sgdb_games(&resp.body)
                .ok_or_else(|| Fail::Transient("SteamGridDB search: unexpected response".into()))?
        } else {
            vec![]
        };
        self.cache.lock().sgdb_search.insert(ck, games.clone());
        Ok(games)
    }

    /// Images of `target` (`game/<id>` / `steam/<appid>`) for `slot`; None = unknown target.
    pub fn sgdb_assets(&self, target: &str, slot: ArtSlot, key: &str, r: Req) -> Result<Option<Vec<SgdbAsset>>, Fail> {
        let ck = (target.to_string(), slot);
        if let Some(a) = self.cache.lock().sgdb_assets.get(&ck) {
            return Ok(a.clone());
        }
        let resp = self.sgdb_get(&sgdb_assets_url(target, slot), key, r)?;
        let assets = if resp.status == 200 {
            Some(
                parse_sgdb_assets(&resp.body)
                    .ok_or_else(|| Fail::Transient("SteamGridDB: unexpected response".into()))?,
            )
        } else {
            None
        };
        self.cache.lock().sgdb_assets.insert(ck, assets.clone());
        Ok(assets)
    }

    fn sgdb_search(&self, job: &SearchJob, key: &str, r: Req) -> Result<Vec<Candidate>, Fail> {
        let mut targets: Vec<(String, String)> = vec![];
        let mut by_appid = None;
        if let Some(appid) = job.appid {
            let t = format!("steam/{appid}");
            if let Some(a) = self.sgdb_assets(&t, job.slot, key, r)? {
                by_appid = Some((t.clone(), a));
                targets.push((t, job.name.clone()));
            }
        }
        if targets.is_empty() {
            let mut games = self.sgdb_games(&job.name, key, r)?;
            games.truncate(TOP_HITS);
            games.sort_by_key(|g| !same_name(&job.name, &g.name));
            targets = games.into_iter().map(|g| (format!("game/{}", g.id), g.name)).collect();
        }
        let mut out = vec![];
        for (t, gname) in targets {
            let assets = match &by_appid {
                Some((bt, a)) if *bt == t => a.clone(),
                _ => self.sgdb_assets(&t, job.slot, key, r)?.unwrap_or_default(),
            };
            for a in assets.into_iter().take(SGDB_PER_GAME) {
                if out.len() >= SGDB_PER_SEARCH {
                    break;
                }
                let label = if a.style.is_empty() {
                    format!("{SRC_SGDB} · {gname}")
                } else {
                    format!("{SRC_SGDB} · {gname} · {}", a.style)
                };
                out.push(Candidate {
                    source: SRC_SGDB.into(),
                    slot: job.slot,
                    url: a.url,
                    thumb_url: a.thumb,
                    width: a.width,
                    height: a.height,
                    label,
                });
            }
        }
        Ok(out)
    }

    /// Run a search; `emit(items, done, errors)` returns false when the search was superseded.
    pub fn search(&self, job: &SearchJob, mut emit: impl FnMut(Vec<Candidate>, bool, Vec<String>) -> bool) {
        let r = Req::UI;
        let q = Query::of_search(job);
        let sources: Vec<SourceKind> =
            self.available_sources(job.list).into_iter().filter(|k| k.has_slot(job.slot) && q.searchable(*k)).collect();
        if sources.is_empty() {
            emit(vec![], true, vec![]);
            return;
        }
        for (i, k) in sources.iter().enumerate() {
            let res = match k {
                SourceKind::Steam => self.steam_search(job, r),
                SourceKind::SteamGridDb => match self.sgdb_key() {
                    Some(key) => self.sgdb_search(job, &key, r),
                    None => Ok(vec![]),
                },
                k => self
                    .find(*k, &q, job.slot, false, r)
                    .map(|found| candidates(*k, job.slot, found.unwrap_or_default())),
            };
            let (items, errors) = match res {
                Ok(c) => (c, vec![]),
                Err(Fail::Cancelled) => (vec![], vec![]),
                // Already names the source and says when it continues.
                Err(Fail::Limit(why)) => (vec![], vec![why]),
                Err(e) => (vec![], vec![format!("{}: {e}", k.name())]),
            };
            if !emit(items, i + 1 == sources.len(), errors) {
                return;
            }
        }
    }

    // ----- images -----

    /// Thumbnail file of `url` in the thumbs cache (any extension), when downloaded before.
    pub fn cached_thumb(&self, url: &str) -> Option<PathBuf> {
        let stem = thumb_stem(url);
        THUMB_EXTS
            .iter()
            .map(|e| self.thumbs_dir.join(format!("{stem}.{e}")))
            .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0))
    }

    /// Download a candidate thumbnail into the cache.
    pub fn thumb(&self, url: &str) -> Result<PathBuf, String> {
        if let Some(p) = self.cached_thumb(url) {
            return Ok(p);
        }
        let mut resp = self.get(url, None, MAX_IMAGE_BYTES, Req::UI).map_err(|e| e.to_string())?;
        if resp.status == 200
            && let Some(target) = libretro_symlink(url, &resp.body)
        {
            resp = self.get(&target, None, MAX_IMAGE_BYTES, Req::UI).map_err(|e| e.to_string())?;
        }
        if resp.status != 200 {
            return Err(format!("HTTP {}", resp.status));
        }
        let ext = sniff_ext(&resp.body).ok_or("not an image")?;
        let dest = self.thumbs_dir.join(format!("{}.{ext}", thumb_stem(url)));
        write_atomic(&dest, &resp.body).map_err(|e| e.to_string())?;
        Ok(dest)
    }

    /// Fetch a full image (reusing a cached thumbnail of the same URL) and check it decodes.
    fn fetch_image(&self, url: &str, r: Req) -> Result<Fetched, Fail> {
        if let Some(p) = self.cached_thumb(url)
            && let Ok(bytes) = std::fs::read(&p)
            && let Ok(ext) = validate_image(&bytes)
        {
            return Ok(Fetched::Image(bytes, ext));
        }
        let mut resp = self.get(url, None, MAX_IMAGE_BYTES, r)?;
        if resp.status == 200
            && let Some(target) = libretro_symlink(url, &resp.body)
        {
            resp = self.get(&target, None, MAX_IMAGE_BYTES, r)?;
        }
        if resp.status != 200 {
            return Ok(Fetched::Missing(resp.status));
        }
        match validate_image(&resp.body) {
            Ok(ext) => Ok(Fetched::Image(resp.body, ext)),
            Err(e) => {
                log::debug!("art: {url}: {e}");
                Ok(Fetched::NotImage(e))
            }
        }
    }

    /// Download `url` (or read a local path / file:// URL) into the game's user art.
    pub fn apply(&self, game: GameId, slot: ArtSlot, url: &str) -> Result<PathBuf, String> {
        let url = url.trim();
        if !(url.starts_with("https://") || url.starts_with("http://")) {
            let path = url.strip_prefix("file://").map(percent_decode).unwrap_or_else(|| url.to_string());
            let path = expand_tilde(&path);
            return import_local(&self.art_dir, game, slot, &path).map_err(|e| format!("{e:#}"));
        }
        match self.fetch_image(url, Req::UI) {
            Ok(Fetched::Image(bytes, ext)) => {
                write_user_art(&self.art_dir, game, slot, ext, &bytes).map_err(|e| e.to_string())
            }
            Ok(Fetched::Missing(s)) => Err(format!("nothing found at that address (HTTP {s})")),
            Ok(Fetched::NotImage(e)) => Err(format!("that address isn't an image ({e})")),
            Err(e) => Err(format!("download failed: {e}")),
        }
    }

    /// Download the first of `urls` that is a valid image into the slot.
    fn bulk_download(&self, g: &BulkGame, slot: ArtSlot, urls: &[String], r: Req) -> Result<Option<PathBuf>, Fail> {
        for u in urls {
            if let Fetched::Image(bytes, ext) = self.fetch_image(u, r)? {
                let p = match &g.dest {
                    Some(dir) => {
                        let p = dir.join(format!("{}.{ext}", slot.key()));
                        write_atomic(&p, &bytes).map(|_| p)
                    }
                    None => write_user_art(&self.art_dir, g.id, slot, ext, &bytes),
                }
                .map_err(|e| Fail::Transient(e.to_string()))?;
                return Ok(Some(p));
            }
        }
        Ok(None)
    }

    // ----- bulk -----

    fn bulk_steam(
        &self,
        g: &BulkGame,
        need: &mut Vec<ArtSlot>,
        r: Req,
        on_applied: &mut dyn FnMut(ArtSlot, PathBuf),
    ) -> Result<bool, Fail> {
        let appid = match g.appid {
            Some(a) => a,
            None => match self.confident_store_match(&g.name, r)? {
                Some(a) => a,
                None => return Ok(false),
            },
        };
        let assets = self.assets_or_none(appid, r)?;
        for slot in need.clone() {
            let urls: Vec<String> = steam_urls(appid, assets.as_ref(), slot).into_iter().map(|o| o.url).collect();
            let mut got = self.bulk_download(g, slot, &urls, r)?;
            if got.is_none()
                && slot == ArtSlot::Banner
                && assets.is_none()
                && let Some(u) = self.appdetails_header(appid, r)?
                && !urls.contains(&u)
            {
                got = self.bulk_download(g, slot, &[u], r)?;
            }
            if let Some(p) = got {
                on_applied(slot, p);
                need.retain(|s| *s != slot);
            }
        }
        Ok(true)
    }

    /// The Steam app named exactly `name`. The store search finds nothing for some exact names
    /// ("Nioh 2 – The Complete Edition": the dash) that it finds by their plain words, so
    /// those are searched too when the name has any punctuation.
    pub fn confident_store_match(&self, name: &str, r: Req) -> Result<Option<u32>, Fail> {
        if let Some(a) = confident_steam(name, &self.store_search(name, r)?) {
            return Ok(Some(a));
        }
        let plain = norm_name(name);
        if plain.is_empty() || plain == name.trim().to_lowercase() {
            return Ok(None);
        }
        Ok(confident_steam(name, &self.store_search(&plain, r)?))
    }

    fn bulk_sgdb(
        &self,
        g: &BulkGame,
        key: &str,
        need: &mut Vec<ArtSlot>,
        r: Req,
        on_applied: &mut dyn FnMut(ArtSlot, PathBuf),
    ) -> Result<bool, Fail> {
        let mut target = None;
        if let Some(appid) = g.appid {
            let t = format!("steam/{appid}");
            if let Some(&slot) = need.first()
                && self.sgdb_assets(&t, slot, key, r)?.is_some()
            {
                target = Some(t);
            }
        }
        if target.is_none() {
            target = confident_sgdb(&g.name, &self.sgdb_games(&g.name, key, r)?).map(|id| format!("game/{id}"));
        }
        let Some(target) = target else {
            return Ok(false);
        };
        for slot in need.clone() {
            let assets = self.sgdb_assets(&target, slot, key, r)?.unwrap_or_default();
            let urls: Vec<String> = assets.into_iter().take(3).map(|a| a.url).collect();
            if let Some(p) = self.bulk_download(g, slot, &urls, r)? {
                on_applied(slot, p);
                need.retain(|s| *s != slot);
            }
        }
        Ok(true)
    }

    /// One game: fill `need` from `sources` in order. Ok((slots applied, matched any source));
    /// the first failure when no source answered at all.
    #[cfg(test)]
    pub fn bulk_game(
        &self,
        g: &BulkGame,
        sources: &[SourceKind],
        r: Req,
        on_applied: &mut dyn FnMut(ArtSlot, PathBuf),
    ) -> Result<(usize, bool), Fail> {
        let run = self.bulk_game_run(g, sources, r, on_applied)?;
        if run.answered.is_empty()
            && let Some((_, e)) = run.failed.into_iter().next()
        {
            return Err(e);
        }
        Ok((run.applied, run.matched))
    }

    /// One game, source by source: a source that fails (network, rejected key) doesn't stop
    /// the others. Err only when cancelled.
    pub(crate) fn bulk_game_run(
        &self,
        g: &BulkGame,
        sources: &[SourceKind],
        r: Req,
        on_applied: &mut dyn FnMut(ArtSlot, PathBuf),
    ) -> Result<GameRun, Fail> {
        let mut need = g.need.clone();
        let mut run = GameRun::default();
        let q = Query::of_bulk(g);
        for src in sources {
            if r.cancelled() {
                break;
            }
            // Nothing (more) to ask it: counts as answered.
            if need.is_empty() || !q.fits(*src) || !need.iter().any(|s| src.has_slot(*s)) {
                run.answered.push(*src);
                continue;
            }
            let res = match src {
                SourceKind::Steam => self.bulk_steam(g, &mut need, r, on_applied),
                SourceKind::SteamGridDb => match self.sgdb_key() {
                    Some(key) => self.bulk_sgdb(g, &key, &mut need, r, on_applied),
                    None => Ok(false),
                },
                k => self.bulk_other(*k, g, &q, &mut need, r, on_applied),
            };
            match res {
                Ok(m) => {
                    run.matched |= m;
                    run.answered.push(*src);
                }
                Err(Fail::Cancelled) => return Err(Fail::Cancelled),
                Err(e) => {
                    log::info!("art: {}: {}: {e}", g.name, src.name());
                    run.failed.push((*src, e));
                }
            }
        }
        if r.cancelled() {
            return Err(Fail::Cancelled);
        }
        run.applied = g.need.len() - need.len();
        Ok(run)
    }

    /// A whole bulk run. `progress(done, total, current)`.
    pub fn bulk(
        &self,
        games: &[Game],
        opts: &BulkOptions,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(usize, usize, &str),
        on_applied: &mut dyn FnMut(GameId, ArtSlot, PathBuf),
    ) -> BulkReport {
        let r = Req { bulk: true, cancel: Some(cancel), patient: true };
        // Each game uses the source list of Settings → Metadata that matches it.
        let mut lists: HashMap<ArtList, Vec<SourceKind>> =
            ArtList::ALL.into_iter().map(|l| (l, bulk_sources(&opts.sources, &self.sources_for(l)))).collect();
        let work: Vec<(BulkGame, ArtList)> = games
            .iter()
            .filter_map(|g| {
                let need = slots_to_fill(g, opts);
                (!need.is_empty()).then(|| {
                    let game = BulkGame {
                        id: g.id,
                        name: g.display_name().to_string(),
                        appid: steam_appid(g),
                        gog_id: gog_product_id(g),
                        platform: sources::art_platform(g),
                        rom_names: sources::rom_names(g),
                        serial: sources::rom_serial(g),
                        need,
                        dest: None,
                    };
                    (game, sources::art_list(g))
                })
            })
            .collect();
        let total = work.len();
        let mut rep = BulkReport::default();
        progress(0, total, "");
        let used: HashSet<ArtList> = work.iter().map(|(_, l)| *l).collect();
        if total > 0 && used.iter().all(|l| lists[l].is_empty()) {
            rep.error = Some("no artwork source is available".into());
            return rep;
        }
        // One GetItems request per 30 Steam games instead of one each.
        if lists.values().any(|s| s.contains(&SourceKind::Steam)) {
            let ids: Vec<u32> = work.iter().filter_map(|(g, _)| g.appid).collect();
            if let Err(e) = self.steam_assets(&ids, r) {
                log::info!("art bulk: GetItems prefetch: {e}");
            }
        }
        let mut transient_run = 0;
        for (i, (g, list)) in work.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                rep.cancelled = true;
                break;
            }
            progress(i, total, &g.name);
            let sources = lists[list].clone();
            if sources.is_empty() {
                // Not asked (nothing to ask for this kind of game, or its sources hit a limit /
                // rejected the key earlier in the run, which `rep.error` already says): not
                // "not found".
                rep.failed += 1;
                if rep.error.is_none() {
                    rep.error = Some(format!(
                        "no artwork source is available for {} (Settings → Metadata)",
                        list.label().to_lowercase()
                    ));
                }
                progress(i + 1, total, &g.name);
                continue;
            }
            match self.bulk_game_run(g, &sources, r, &mut |slot, p| on_applied(g.id, slot, p)) {
                Ok(run) => {
                    for (src, e) in &run.failed {
                        match e {
                            Fail::BadKey(k) => {
                                lists.values_mut().for_each(|s| s.retain(|s| s != k));
                                rep.error = Some(format!("{} rejected the API key", k.name()));
                            }
                            // Not asked again this run; the reason says when it continues.
                            Fail::Limit(why) => {
                                log::info!("art bulk: {} paused: {why}", src.name());
                                lists.values_mut().for_each(|s| s.retain(|s| s != src));
                                rep.error = Some(why.clone());
                            }
                            _ => {}
                        }
                    }
                    let net_err = run.failed.iter().find_map(|(_, e)| match e {
                        Fail::Transient(t) => Some(t.clone()),
                        _ => None,
                    });
                    if run.answered.is_empty() && !run.failed.is_empty() {
                        // No source answered: offline, or every key rejected.
                        rep.failed += 1;
                        if let Some(e) = net_err {
                            transient_run += 1;
                            if transient_run >= 3 {
                                rep.error = Some(format!("stopped after repeated network errors ({e})"));
                                break;
                            }
                        }
                    } else {
                        transient_run = 0;
                        if run.applied > 0 {
                            rep.changed += 1;
                        } else if net_err.is_some() {
                            // A source that might have known it didn't answer.
                            rep.failed += 1;
                        } else {
                            if run.matched {
                                log::info!("art bulk: {}: matched, but no images", g.name);
                            }
                            rep.not_found.push(g.name.clone());
                        }
                    }
                    if used.iter().all(|l| lists[l].is_empty()) {
                        break;
                    }
                }
                Err(_) => {
                    rep.cancelled = true;
                    break;
                }
            }
            progress(i + 1, total, &g.name);
        }
        self.limits.set_waiting(None);
        self.limits.flush();
        rep
    }
}

impl Engine {
    // ----- automatic fill after a scan -----

    /// Look up `reqs` one by one (confident matches only) into `<art_dir>/auto/<game>/`.
    /// `on_game` gets every game whose lookup finished, with the art found (possibly none)
    /// and the sources that gave a definitive answer (a source that failed is left out, so it is
    /// asked again another time; the others still ran). A game no source answered for (network
    /// error) is only reported when it got art. Stops when cancelled or after three games in a
    /// row that no source answered (offline).
    pub fn auto_fill(
        &self,
        reqs: &[AutoRequest],
        cancel: &AtomicBool,
        on_game: &mut dyn FnMut(GameId, Art, Vec<SourceKind>),
    ) -> AutoReport {
        let r = Req { bulk: true, cancel: Some(cancel), patient: false };
        let mut rep = AutoReport::default();
        let mut transient_run = 0;
        // Sources whose API limit is reached: asked again another time.
        let mut limited: HashSet<SourceKind> = HashSet::new();
        for req in reqs {
            if cancel.load(Ordering::Relaxed) {
                rep.cancelled = true;
                break;
            }
            let mut sources = req.sources.clone();
            // Keyed sources without (working) credentials are asked another time.
            let cfg = self.cfg.lock().clone();
            sources.retain(|s| s.configured(&cfg) && !self.is_rejected(*s) && !limited.contains(s));
            if sources.is_empty() || req.need.is_empty() {
                continue;
            }
            let g = BulkGame {
                id: req.id,
                name: req.name.clone(),
                appid: req.appid,
                gog_id: req.gog_id,
                platform: req.platform.clone(),
                rom_names: req.rom_names.clone(),
                serial: req.serial.clone(),
                need: req.need.clone(),
                dest: Some(auto_art_dir(&self.art_dir, req.id)),
            };
            let mut art = Art::default();
            let res = self.bulk_game_run(&g, &sources, r, &mut |slot, p| slot.set(&mut art, Some(p)));
            let got = ArtSlot::ALL.iter().any(|s| s.get(&art).is_some());
            let run = match res {
                Ok(run) => run,
                Err(_) => {
                    if got {
                        on_game(req.id, art, vec![]);
                    }
                    rep.cancelled = true;
                    break;
                }
            };
            for (src, e) in &run.failed {
                match e {
                    // Asked again another time (with the new key).
                    Fail::BadKey(k) => rep.error = Some(format!("{} rejected the API key", k.name())),
                    Fail::Limit(why) => {
                        limited.insert(*src);
                        rep.error = Some(why.clone());
                    }
                    _ => {}
                }
            }
            let net_err = run.failed.iter().find_map(|(_, e)| match e {
                Fail::Transient(t) => Some(t.clone()),
                _ => None,
            });
            if run.answered.is_empty() && net_err.is_some() {
                // Offline (or every source failed): the whole game is asked again next time.
                let e = net_err.unwrap_or_default();
                log::info!("art auto: {}: {e}", req.name);
                if got {
                    on_game(req.id, art, vec![]);
                }
                transient_run += 1;
                if transient_run >= 3 {
                    rep.error = Some(format!("stopped after repeated network errors ({e})"));
                    break;
                }
                continue;
            }
            if !run.answered.is_empty() {
                transient_run = 0;
            }
            if got {
                rep.changed += 1;
            }
            // Only the sources that answered count as asked; failed ones are asked again.
            on_game(req.id, art, run.answered);
        }
        self.limits.flush();
        rep
    }
}

/// How one game's lookup went, source by source.
#[derive(Debug, Default)]
pub(crate) struct GameRun {
    /// Slots that got an image.
    pub applied: usize,
    /// Some source knew the game for sure.
    pub matched: bool,
    /// Sources that answered for sure (or had nothing to look for).
    pub answered: Vec<SourceKind>,
    /// Sources that failed (network error, rejected key); the others still ran.
    pub failed: Vec<(SourceKind, Fail)>,
}

/// One game of an automatic fill.
#[derive(Clone, Debug, PartialEq)]
pub struct AutoRequest {
    pub id: GameId,
    pub name: String,
    /// Steam store appid when known (else the name is searched).
    pub appid: Option<u32>,
    /// GOG product id (GOG games).
    pub gog_id: Option<u64>,
    /// The library's platform (console sources).
    pub platform: Option<String>,
    /// Emulated games: the ROM's No-Intro / Redump name and file name (libretro's exact names).
    pub rom_names: Vec<String>,
    /// Emulated games: the ROM header's serial (GameTDB's game ID).
    pub serial: Option<String>,
    /// Slots to fill.
    pub need: Vec<ArtSlot>,
    /// Sources to ask, in order (keyed ones are skipped without working credentials).
    pub sources: Vec<SourceKind>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AutoReport {
    /// Games that got at least one image.
    pub changed: usize,
    pub cancelled: bool,
    pub error: Option<String>,
}

/// `<data>/api-limits.json`, next to the art folder.
pub(crate) fn limits_file(art_dir: &Path) -> PathBuf {
    art_dir.with_file_name("api-limits.json")
}

/// `<art_dir>/auto/<game>`: art found by the automatic fill (stored as fetched art).
pub fn auto_art_dir(art_dir: &Path, game: GameId) -> PathBuf {
    art_dir.join("auto").join(game.to_string())
}

// ---------------------------------------------------------------------------------------------
// Bulk option logic (pure)

/// The slots of `g` a bulk run should fill.
pub(crate) fn slots_to_fill(g: &Game, opts: &BulkOptions) -> Vec<ArtSlot> {
    let mut out: Vec<ArtSlot> = vec![];
    for s in &opts.slots {
        if out.contains(s) {
            continue;
        }
        let missing = s.get(&g.art).is_none_or(|p| !p.is_file());
        if !opts.only_missing || missing {
            out.push(*s);
        }
    }
    out
}

/// Sources a bulk run uses, in priority order: the requested ones that are available
/// (all available ones when none are requested).
pub(crate) fn bulk_sources(requested: &[String], available: &[SourceInfo]) -> Vec<SourceKind> {
    let ok = |k: &SourceKind| available.iter().any(|s| s.available && s.name == k.name());
    let mut out: Vec<SourceKind> = if requested.is_empty() {
        available.iter().map(|s| s.kind).collect()
    } else {
        requested.iter().filter_map(|n| SourceKind::from_name(n)).collect()
    };
    let mut seen = HashSet::new();
    out.retain(|k| ok(k) && seen.insert(*k));
    out
}

/// The GOG product id of a GOG game.
pub(crate) fn gog_product_id(g: &Game) -> Option<u64> {
    (g.source == Source::Gog).then(|| g.source_id.trim().parse().ok()).flatten().filter(|id| *id > 0)
}

fn search_name_for(name: &str) -> String {
    crate::metadata::parse::search_name(name)
}

pub(crate) const THUMB_EXTS: [&str; 8] = ["jpg", "png", "webp", "gif", "bmp", "ico", "qoi", "svg"];

pub(crate) fn thumb_stem(url: &str) -> String {
    format!("{:016x}", crate::db::fnv1a(url.as_bytes()))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("zz"), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn expand_tilde(p: &str) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().map(|h| h.join(rest)).unwrap_or_else(|| PathBuf::from(p)),
        None => PathBuf::from(p),
    }
}
