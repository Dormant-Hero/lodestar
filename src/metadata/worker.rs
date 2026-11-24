//! The metadata worker: HTTP (mocked in tests), polite per-host throttling, and processing one
//! job through the sources (Steam store and CDN, SteamGridDB, IGDB, Wikipedia, ProtonDB,
//! metadata add-ons).

use super::*;

// ---------------------------------------------------------------------------------------------
// HTTP

/// A response with any status code.
pub(crate) struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// Minimal HTTP GET used by the worker (mocked in tests).
pub(crate) trait Http {
    /// Err = network-level failure (DNS, connect, TLS, timeout, body too large).
    fn get(&mut self, url: &str, bearer: Option<&str>, limit: u64) -> Result<HttpResponse, String>;
    /// POST with extra headers (IGDB, Twitch). Body only read for 200.
    fn post(&mut self, url: &str, headers: &[(&str, String)], body: &str, limit: u64) -> Result<HttpResponse, String> {
        let _ = (url, headers, body, limit);
        Err("POST is not supported".into())
    }
}

pub(super) struct UreqHttp {
    agent: ureq::Agent,
}

impl UreqHttp {
    pub(super) fn new() -> UreqHttp {
        let cfg = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(5)))
            .timeout_global(Some(Duration::from_secs(20)))
            .user_agent(USER_AGENT)
            .http_status_as_error(false)
            .max_redirects(5)
            .build();
        UreqHttp { agent: ureq::Agent::new_with_config(cfg) }
    }
}

impl Http for UreqHttp {
    fn get(&mut self, url: &str, bearer: Option<&str>, limit: u64) -> Result<HttpResponse, String> {
        crate::art::net::check_allowed(url)?;
        let mut req = self.agent.get(url);
        if let Some(k) = bearer {
            req = req.header("Authorization", format!("Bearer {k}"));
        }
        let mut resp = req.call().map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let body = if status == 200 {
            resp.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        Ok(HttpResponse { status, body })
    }

    fn post(&mut self, url: &str, headers: &[(&str, String)], body: &str, limit: u64) -> Result<HttpResponse, String> {
        crate::art::net::check_allowed(url)?;
        let mut req = self.agent.post(url).header("Accept", "application/json");
        for (k, v) in headers {
            req = req.header(*k, v.as_str());
        }
        let mut resp = req.send(body).map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let body = if status == 200 {
            resp.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| e.to_string())?
        } else {
            Vec::new()
        };
        Ok(HttpResponse { status, body })
    }
}

/// Minimum spacing between requests per host class.
pub(crate) struct Throttle {
    pub(super) store: Duration,
    cdn: Duration,
    sgdb: Duration,
    protondb: Duration,
    igdb: Duration,
    wikipedia: Duration,
    last: [Option<Instant>; 6],
}

#[derive(Clone, Copy)]
pub(super) enum Host {
    Store = 0,
    Cdn = 1,
    Sgdb = 2,
    ProtonDb = 3,
    /// api.igdb.com and id.twitch.tv (4 requests / s, through `igdb::wait_turn`).
    Igdb = 4,
    /// en.wikipedia.org (one request at a time, as Wikimedia asks).
    Wikipedia = 5,
}

impl Throttle {
    pub(super) fn polite() -> Throttle {
        Throttle {
            // The store's appdetails API allows about 200 requests per 5 minutes per IP;
            // 1.1 s spacing (~270 per 5 min) gets a big library rate limited (429) part way.
            store: Duration::from_millis(STORE_GAP_MS),
            cdn: Duration::from_millis(150),
            sgdb: Duration::from_millis(300),
            // No published limit; one request a second keeps a big library's first pass gentle.
            protondb: Duration::from_millis(1000),
            igdb: Duration::from_millis(500),
            // No hard limit for serial requests; a second apart keeps a big first pass gentle.
            wikipedia: Duration::from_millis(1000),
            last: [None; 6],
        }
    }
    #[cfg(test)]
    pub(crate) fn none() -> Throttle {
        Throttle {
            store: Duration::ZERO,
            cdn: Duration::ZERO,
            sgdb: Duration::ZERO,
            protondb: Duration::ZERO,
            igdb: Duration::ZERO,
            wikipedia: Duration::ZERO,
            last: [None; 6],
        }
    }
    pub(super) fn wait(&mut self, h: Host) {
        let gap = match h {
            Host::Store => self.store,
            Host::Cdn => self.cdn,
            Host::Sgdb => self.sgdb,
            Host::ProtonDb => self.protondb,
            Host::Igdb => self.igdb,
            Host::Wikipedia => self.wikipedia,
        };
        if matches!(h, Host::Igdb) && !gap.is_zero() {
            // One window for the whole process, shared with the artwork engine.
            crate::igdb::wait_turn(None);
            self.last[h as usize] = Some(Instant::now());
            return;
        }
        if let Some(t) = self.last[h as usize] {
            let since = t.elapsed();
            if since < gap {
                std::thread::sleep(gap - since);
            }
        }
        self.last[h as usize] = Some(Instant::now());
    }
}

// ---------------------------------------------------------------------------------------------
// Job processing

/// Network failure / server trouble / rate limit: retry next session.
#[derive(Debug)]
pub(crate) struct Transient(pub String);

pub(super) type R<T> = Result<T, Transient>;

/// SteamGridDB said the key is invalid.
pub(super) enum SgdbOutcome {
    Done,
    BadKey,
}

pub(crate) struct Worker<H: Http> {
    pub http: H,
    pub art_dir: PathBuf,
    pub throttle: Throttle,
    /// Set after a 401/403 from SteamGridDB: don't use it again this session.
    pub sgdb_disabled: bool,
    /// ProtonDB answers (shared with `MetadataFetcher`); saved to `<art_dir>/protondb.json`.
    pub proton: Arc<Mutex<protondb::Cache>>,
    /// After a ProtonDB network error / 5xx / 429: skip it until then (other lookups go on).
    pub protondb_paused_until: Option<Instant>,
}

impl<H: Http> Worker<H> {
    pub(crate) fn process(&mut self, job: &Job, cfg: &MetadataConfig) -> R<FetchResult> {
        let mut res = FetchResult::empty(job);
        if let Some(known) = &job.backfill {
            // IGDB only, and only into what the game lacks (the DB merge lets new values win).
            if let Some(m) = self.igdb(job, cfg)? {
                res.meta.fill_from(&m);
                res.meta.keep_only_missing(known);
            }
            return Ok(res);
        }
        if job.wiki.only {
            if let Some((text, url)) = self.wikipedia(job)? {
                res.meta.description = Some(text);
                res.meta.wikipedia = Some(url);
            }
            return Ok(res);
        }
        let mut want = job.want;
        if let (Some(appid), true) = (job.steam_appid, cfg.steam_store) {
            self.steam(appid, &mut want, &mut res)?;
        }
        let key = sgdb_key(cfg).filter(|_| !self.sgdb_disabled).map(str::to_string);
        if let Some(key) = key
            && (want.cover || want.hero || want.logo)
            && let SgdbOutcome::BadKey = self.sgdb(job, &key, &mut want, &mut res)?
        {
            log::warn!("SteamGridDB rejected the API key; skipping it this session");
            self.sgdb_disabled = true;
            if job.steam_appid.is_none() || !cfg.steam_store {
                // Nothing definitive learned about this game.
                return Err(Transient("SteamGridDB key rejected".into()));
            }
        }
        let mut igdb_matched = false;
        if igdb_wanted(job, cfg) {
            match self.igdb_with(job, cfg, &mut res.candidates) {
                Ok(Some(m)) => {
                    igdb_matched = true;
                    res.meta.fill_from(&m);
                }
                Ok(None) => {}
                // Nothing else was asked: try the whole job again another time.
                Err(e) if !(job.steam_appid.is_some() && cfg.steam_store) => return Err(e),
                Err(Transient(e)) => log::info!("IGDB: {} ({}): {e}", job.name, job.id),
            }
        }
        if wikipedia_wanted(job, cfg) && (cfg.prefer_wikipedia || res.meta.description.is_none()) {
            // Ambiguous IGDB answers are offered first; Wikipedia's only when IGDB had none
            // and found no game either (a game IGDB settled isn't asked about again).
            let mut wiki_candidates = vec![];
            // A lookup the user asked for: the year of the game found (Steam / IGDB) tells
            // same-named articles apart (the user's own year comes first).
            let year = job.wiki.year.or(job.lookup.as_ref().and(res.meta.release_year));
            let found = self.wikipedia_with(job, year, &mut wiki_candidates);
            if res.candidates.is_empty() && !igdb_matched {
                res.candidates = wiki_candidates;
            }
            match found {
                Ok(found) => {
                    if let Some((text, url)) = found {
                        res.meta.description = Some(text);
                        res.meta.wikipedia = Some(url);
                    }
                    // Wikipedia answered: the game needs no background lookup later.
                    res.wiki_backfill = true;
                }
                // Nothing else was asked: try the whole job again another time.
                Err(e) if !(job.steam_appid.is_some() && cfg.steam_store) && !igdb_wanted(job, cfg) => {
                    return Err(e);
                }
                Err(Transient(e)) => log::info!("Wikipedia: {} ({}): {e}", job.name, job.id),
            }
        }
        if let (Some(appid), true, true) = (job.steam_appid, cfg.steam_store, want.protondb) {
            res.meta.protondb = self.protondb(appid, job.protondb_max_age);
        }
        Ok(res)
    }

    /// One IGDB request (POST); Ok(None) for a definitive "no" (4xx) or rejected credentials.
    pub(super) fn igdb_post(&mut self, url: &str, body: &str, cfg: &MetadataConfig) -> R<Option<Vec<u8>>> {
        let Some((id, secret)) = cfg.igdb_credentials() else {
            return Ok(None);
        };
        let secrets = cfg.secrets();
        let token = crate::igdb::token(id, secret, |u| {
            self.throttle.wait(Host::Igdb);
            self.http.post(u, &[], "", MAX_JSON_BYTES).map(|r| (r.status, r.body))
        });
        let token = match token {
            Ok(t) => t,
            Err(crate::igdb::TokenErr::BadCredentials) => {
                log::warn!("Twitch rejected the IGDB client id / secret");
                return Ok(None);
            }
            Err(crate::igdb::TokenErr::Transient(e)) => {
                return Err(Transient(crate::config::redact(&e, &secrets)));
            }
        };
        self.throttle.wait(Host::Igdb);
        let r = self
            .http
            .post(url, &crate::igdb::headers(id, &token), body, MAX_JSON_BYTES)
            .map_err(|e| Transient(crate::config::redact(&e, &secrets)))?;
        match r.status {
            200 => Ok(Some(r.body)),
            401 => {
                // Token expired early: a new one next time.
                crate::igdb::forget_token(id, secret);
                Err(Transient("IGDB: token refused".into()))
            }
            403 => {
                log::warn!("IGDB rejected the client id");
                crate::igdb::mark_rejected(id, secret);
                Ok(None)
            }
            s if is_definitive(s) => Ok(None),
            s => Err(Transient(format!("IGDB: HTTP {s}"))),
        }
    }

    /// IGDB's metadata of the game: by Steam appid (IGDB's external games) or, failing that, by
    /// exact name. None = no confident match.
    pub(super) fn igdb(&mut self, job: &Job, cfg: &MetadataConfig) -> R<Option<Metadata>> {
        self.igdb_with(job, cfg, &mut vec![])
    }

    /// `igdb`; for a lookup the user asked for, the game they picked, and an ambiguous name
    /// search fills `candidates` instead of guessing.
    pub(super) fn igdb_with(
        &mut self,
        job: &Job,
        cfg: &MetadataConfig,
        candidates: &mut Vec<Candidate>,
    ) -> R<Option<Metadata>> {
        use crate::igdb::*;
        let bad = || Transient("IGDB: unexpected response".into());
        let lookup = job.lookup.as_ref();
        if let Some(Pick::Igdb(id)) = lookup.and_then(|l| l.pick.as_ref()) {
            let Some(b) = self.igdb_post(&games_url(), &by_ids_body(&[*id]), cfg)? else {
                return Ok(None);
            };
            return Ok(parse_games(&b).ok_or_else(bad)?.first().map(to_metadata));
        }
        if let Some(appid) = job.steam_appid {
            let ids = match self.igdb_post(&external_games_url(), &steam_body(appid), cfg)? {
                Some(b) => parse_external(&b).ok_or_else(bad)?,
                None => vec![],
            };
            if !ids.is_empty()
                && let Some(b) = self.igdb_post(&games_url(), &by_ids_body(&ids), cfg)?
                && let Some(g) = parse_games(&b).ok_or_else(bad)?.first()
            {
                return Ok(Some(to_metadata(g)));
            }
        }
        let name = parse::search_name(&job.name);
        if name.is_empty() {
            return Ok(None);
        }
        let Some(b) = self.igdb_post(&games_url(), &search_body_on(&name, job.igdb_platform), cfg)? else {
            return Ok(None);
        };
        let games = parse_games(&b).ok_or_else(bad)?;
        if lookup.is_none_or(|l| l.pick.is_some()) {
            return Ok(confident(&job.name, &games).map(to_metadata));
        }
        // The user asked: one game of that name is the answer; several (or only other names)
        // are offered to pick from.
        let mut same: Vec<&IgdbGame> =
            games.iter().filter(|g| crate::art::sources::same_name(&job.name, &g.name)).collect();
        if same.len() > 1
            && let Some(y) = job.wiki.year
        {
            let of_year: Vec<&IgdbGame> = same.iter().copied().filter(|g| igdb_year(g) == Some(y)).collect();
            if of_year.len() == 1 {
                same = of_year;
            }
        }
        if same.len() == 1 {
            return Ok(Some(to_metadata(same[0])));
        }
        let offer: Vec<&IgdbGame> = if same.is_empty() { games.iter().collect() } else { same };
        candidates.extend(offer.into_iter().take(MAX_CANDIDATES).map(|g| Candidate {
            pick: Pick::Igdb(g.id),
            name: g.name.clone(),
            year: igdb_year(g),
            platforms: g.platforms.join(", "),
            source: "IGDB",
        }));
        Ok(None)
    }

    /// The lead section of the game's English Wikipedia article and the article's URL. None =
    /// no confident match.
    pub(super) fn wikipedia(&mut self, job: &Job) -> R<Option<(String, String)>> {
        self.wikipedia_with(job, job.wiki.year, &mut vec![])
    }

    /// `wikipedia`; for a lookup the user asked for, the article they picked, and same-named
    /// articles it can't tell apart (or only other games' articles) fill `candidates`.
    /// `year` tells same-named articles apart.
    pub(super) fn wikipedia_with(
        &mut self,
        job: &Job,
        year: Option<i32>,
        candidates: &mut Vec<Candidate>,
    ) -> R<Option<(String, String)>> {
        let name = match job.lookup.as_ref().and_then(|l| l.pick.as_ref()) {
            Some(Pick::Wikipedia(title)) => title.clone(),
            _ => parse::search_name(&job.name),
        };
        if name.is_empty() {
            return Ok(None);
        }
        let r = self.get(&wikipedia::search_url(&name), Host::Wikipedia, None, MAX_JSON_BYTES)?;
        match r.status {
            200 => {}
            s if is_definitive(s) => return Ok(None),
            s => return Err(Transient(format!("Wikipedia: HTTP {s}"))),
        }
        let want = wikipedia::Want { name: &job.name, year, platform: job.wiki.platform.as_deref() };
        let Some(l) = job.lookup.as_ref() else {
            return wikipedia::description_from(&r.body, &want)
                .ok_or_else(|| Transient("Wikipedia: unexpected response".into()));
        };
        let pages =
            wikipedia::parse_search(&r.body).ok_or_else(|| Transient("Wikipedia: unexpected response".into()))?;
        let page = match &l.pick {
            Some(Pick::Wikipedia(title)) => pages.iter().find(|p| p.title == *title),
            Some(Pick::Igdb(_)) => wikipedia::pick(&pages, &want),
            None => {
                let page = wikipedia::pick(&pages, &want);
                if page.is_none() {
                    candidates.extend(wikipedia::candidates(&pages, &want).into_iter().take(MAX_CANDIDATES).map(|p| {
                        Candidate {
                            pick: Pick::Wikipedia(p.title.clone()),
                            name: p.title.clone(),
                            year: wikipedia::page_year(p),
                            platforms: String::new(),
                            source: "Wikipedia",
                        }
                    }));
                }
                page
            }
        };
        Ok(page.and_then(wikipedia::text_of))
    }

    /// Fill what the built-in sources left missing from the enabled metadata add-ons. Their
    /// failures never fail the job (logged and kept as the add-on's last error).
    pub(super) fn ask_addons(&mut self, job: &Job, a: &AddonSources, res: &mut FetchResult) {
        let w = job.want;
        let need_meta = w.meta && res.meta.description.is_none();
        let need = |want: bool, have: &Option<PathBuf>| want && have.is_none();
        if !need_meta && !need(w.cover, &res.art.cover) && !need(w.hero, &res.art.hero) && !need(w.logo, &res.art.logo)
        {
            return;
        }
        let Some(found) = crate::addons::metadata::lookup(&a.mgr, &a.cfg, &job.name, None) else {
            return;
        };
        log::info!("metadata: {} ({}) from add-on {}", job.name, job.id, found.addon);
        if w.meta {
            res.meta.fill_from(&found.meta);
        }
        res.art.fill_from(&found.art.local);
        let dir = self.art_dir.join("addon").join(job.id.to_string());
        let urls = &found.art.urls;
        for (want, slot, url, stem) in [
            (w.cover, &mut res.art.cover, &urls.cover, "cover"),
            (w.hero, &mut res.art.hero, &urls.hero, "hero"),
            (w.logo, &mut res.art.logo, &urls.logo, "logo"),
            (w.banner, &mut res.art.banner, &urls.banner, "banner"),
        ] {
            if !want || slot.is_some() {
                continue;
            }
            let Some(url) = url.as_deref().filter(|u| u.starts_with("https://") || u.starts_with("http://")) else {
                continue;
            };
            match self.download(url, &dir, stem, Host::Cdn, None) {
                Ok(p) => *slot = p,
                Err(Transient(e)) => log::info!("metadata add-on art for {}: {e}", job.name),
            }
        }
    }

    /// The ProtonDB tier: from the cache while fresh, else asked online. Failures never fail
    /// the job (the art / store answers already gathered still count): they pause ProtonDB
    /// lookups for a while and leave the cache alone, so the next pass asks again.
    pub(super) fn protondb(&mut self, appid: u32, max_age: i64) -> Option<String> {
        let now = now_unix();
        {
            let c = self.proton.lock();
            if c.is_fresh(appid, max_age, now) {
                return c.get(appid).and_then(|e| e.tier.clone());
            }
        }
        if self.protondb_paused_until.is_some_and(|t| Instant::now() < t) {
            return None;
        }
        let fail = |w: &mut Self, why: String| {
            log::info!("ProtonDB {appid}: {why}; pausing ProtonDB lookups");
            w.protondb_paused_until = Some(Instant::now() + OFFLINE_BACKOFF);
            None
        };
        let r = match self.get(&protondb::url(appid), Host::ProtonDb, None, MAX_JSON_BYTES) {
            Ok(r) => r,
            Err(Transient(e)) => return fail(self, e),
        };
        let tier = match r.status {
            200 => match protondb::parse_summary(&r.body) {
                Some(t) => t,
                None => return fail(self, "unexpected response".into()),
            },
            // 404: no reports for this game yet.
            s if is_definitive(s) => None,
            s => return fail(self, format!("HTTP {s}")),
        };
        self.proton.lock().record(appid, tier.clone(), now);
        tier
    }

    /// Write the ProtonDB cache (atomic) when it changed.
    pub(crate) fn save_protondb(&mut self) {
        let bytes = {
            let mut c = self.proton.lock();
            if c.dirty == 0 {
                return;
            }
            c.dirty = 0;
            c.to_json()
        };
        let path = protondb::Cache::path(&self.art_dir);
        if let Err(e) = write_atomic(&path, &bytes) {
            log::warn!("{}: {e}", path.display());
        }
    }

    pub(super) fn steam(&mut self, appid: u32, want: &mut Wanted, res: &mut FetchResult) -> R<()> {
        let dir = self.art_dir.join("steam").join(appid.to_string());
        let cdn = |f: &str| format!("{STEAM_CDN}/{appid}/{f}");
        if want.cover {
            let mut got = existing(&dir, "library_600x900_2x").or_else(|| existing(&dir, "library_600x900"));
            if got.is_none() {
                got = self.download(&cdn("library_600x900_2x.jpg"), &dir, "library_600x900_2x", Host::Cdn, None)?;
            }
            if got.is_none() {
                got = self.download(&cdn("library_600x900.jpg"), &dir, "library_600x900", Host::Cdn, None)?;
            }
            if got.is_some() {
                want.cover = false;
            }
            res.art.cover = got;
        }
        if want.hero {
            res.art.hero = self.download(&cdn("library_hero.jpg"), &dir, "library_hero", Host::Cdn, None)?;
            want.hero &= res.art.hero.is_none();
        }
        if want.logo {
            res.art.logo = self.download(&cdn("logo.png"), &dir, "logo", Host::Cdn, None)?;
            want.logo &= res.art.logo.is_none();
        }
        if want.banner {
            res.art.banner = self.download(&cdn("header.jpg"), &dir, "header", Host::Cdn, None)?;
            want.banner &= res.art.banner.is_none();
        }
        if want.meta {
            let url = format!("{STEAM_STORE}?appids={appid}&l=english");
            let r = self.get(&url, Host::Store, None, MAX_JSON_BYTES)?;
            match r.status {
                200 => match parse::parse_appdetails(&r.body, appid) {
                    AppDetails::Found(m) => res.meta = *m,
                    AppDetails::NotFound => {}
                    AppDetails::Invalid => {
                        return Err(Transient("appdetails: unexpected response".into()));
                    }
                },
                s if is_definitive(s) => {}
                s => return Err(Transient(format!("appdetails: HTTP {s}"))),
            }
            want.meta = false;
        }
        Ok(())
    }

    pub(super) fn sgdb(&mut self, job: &Job, key: &str, want: &mut Wanted, res: &mut FetchResult) -> R<SgdbOutcome> {
        // Steam games are addressed by appid; others are searched by name.
        let target = match job.steam_appid {
            Some(appid) => format!("steam/{appid}"),
            None => {
                let name = parse::search_name(&job.name);
                if name.is_empty() {
                    return Ok(SgdbOutcome::Done);
                }
                let url = format!("{SGDB_API}/search/autocomplete/{}", parse::encode_path_segment(&name));
                let r = self.get(&url, Host::Sgdb, Some(key), MAX_JSON_BYTES)?;
                match r.status {
                    200 => match parse::parse_sgdb_search(&r.body) {
                        Some(Some(id)) => format!("game/{id}"),
                        Some(None) => return Ok(SgdbOutcome::Done),
                        None => {
                            return Err(Transient("SteamGridDB search: unexpected response".into()));
                        }
                    },
                    401 | 403 => return Ok(SgdbOutcome::BadKey),
                    s if is_definitive(s) => return Ok(SgdbOutcome::Done),
                    s => return Err(Transient(format!("SteamGridDB search: HTTP {s}"))),
                }
            }
        };
        let dir = self.art_dir.join("sgdb").join(job.id.to_string());
        let kinds: [(bool, &str, &str, &str); 3] = [
            (want.cover, "grids", "?dimensions=600x900&types=static", "cover"),
            (want.hero, "heroes", "?types=static", "hero"),
            (want.logo, "logos", "?types=static", "logo"),
        ];
        for (wanted, kind, query, stem) in kinds {
            if !wanted {
                continue;
            }
            // Reuse a file from an earlier, interrupted run without asking the API again.
            let path = match existing(&dir, stem) {
                Some(p) => Some(p),
                None => {
                    let url = format!("{SGDB_API}/{kind}/{target}{query}");
                    let r = self.get(&url, Host::Sgdb, Some(key), MAX_JSON_BYTES)?;
                    let asset = match r.status {
                        200 => parse::parse_sgdb_assets(&r.body)
                            .ok_or_else(|| Transient(format!("SteamGridDB {kind}: unexpected response")))?,
                        401 | 403 => return Ok(SgdbOutcome::BadKey),
                        s if is_definitive(s) => None,
                        s => return Err(Transient(format!("SteamGridDB {kind}: HTTP {s}"))),
                    };
                    match asset {
                        Some(u) => self.download(&u, &dir, stem, Host::Cdn, None)?,
                        None => None,
                    }
                }
            };
            match stem {
                "cover" => res.art.cover = res.art.cover.take().or(path),
                "hero" => res.art.hero = res.art.hero.take().or(path),
                _ => res.art.logo = res.art.logo.take().or(path),
            }
        }
        want.cover &= res.art.cover.is_none();
        want.hero &= res.art.hero.is_none();
        want.logo &= res.art.logo.is_none();
        Ok(SgdbOutcome::Done)
    }

    pub(super) fn get(&mut self, url: &str, host: Host, bearer: Option<&str>, limit: u64) -> R<HttpResponse> {
        self.throttle.wait(host);
        self.http.get(url, bearer, limit).map_err(Transient)
    }

    /// Download an image to `<dir>/<stem>.<ext>` (atomic). Ok(None) when the server definitively
    /// has no such file (4xx) or returned something that isn't an image.
    pub(super) fn download(
        &mut self,
        url: &str,
        dir: &Path,
        stem: &str,
        host: Host,
        bearer: Option<&str>,
    ) -> R<Option<PathBuf>> {
        if let Some(p) = existing(dir, stem) {
            return Ok(Some(p));
        }
        let r = self.get(url, host, bearer, MAX_IMAGE_BYTES)?;
        match r.status {
            200 => {}
            s if is_definitive(s) => return Ok(None),
            s => return Err(Transient(format!("{url}: HTTP {s}"))),
        }
        let Some(ext) = parse::image_ext(&r.body) else {
            log::debug!("metadata: {url} is not an image");
            return Ok(None);
        };
        let dest = dir.join(format!("{stem}.{ext}"));
        match write_atomic(&dest, &r.body) {
            Ok(()) => Ok(Some(dest)),
            Err(e) => {
                // A local disk problem: not the server's fault, but don't record "no art" either.
                Err(Transient(format!("writing {}: {e}", dest.display())))
            }
        }
    }
}

/// The release year of an IGDB game.
pub(super) fn igdb_year(g: &crate::igdb::IgdbGame) -> Option<i32> {
    g.first_release.map(|t| crate::igdb::ymd(t).0)
}

/// 4xx (except 408/429) is a definitive "no"; 5xx / 408 / 429 / anything else is transient.
pub(super) fn is_definitive(status: u16) -> bool {
    (400..500).contains(&status) && status != 408 && status != 429
}

/// A previously downloaded `<dir>/<stem>.{jpg,png,webp,gif}`.
pub(super) fn existing(dir: &Path, stem: &str) -> Option<PathBuf> {
    ["jpg", "png", "webp", "gif"]
        .iter()
        .map(|e| dir.join(format!("{stem}.{e}")))
        .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.len() > 0))
}
