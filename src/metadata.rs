//! Optional online metadata/art (Steam store + CDN without a key; SteamGridDB with a user key).
//!
//! CONTRACT:
//!   MetadataFetcher::new(ctx, art_dir, &MetadataConfig) — one background worker thread, queue,
//!     polite rate limit, short timeouts; never blocks the UI; offline = silently nothing.
//!   set_config(&mut self, &MetadataConfig)
//!   request(&self, &Game) — queue a lookup (dedup by id). Only fills what is missing.
//!   lookup(&self, &Game, LookupReq) — a lookup the user asked for (Download metadata…): asks
//!     every source again with the name typed in the dialog, answers once (`FetchResult::lookup`),
//!     offers candidates for an ambiguous name; the app stores it with `Db::replace_fetched`.
//!   poll(&mut self) -> Vec<FetchResult>
//!   pending(&self) -> usize
//!
//! Behaviour:
//! - Steam games (when `steam_store`): missing art slots from the Steam CDN into
//!   `<art_dir>/steam/<appid>/`, metadata from the store's `appdetails` API.
//! - With a SteamGridDB key: non-Steam games are searched by name; Steam games whose art the CDN
//!   lacks are looked up by appid. Files go to `<art_dir>/sgdb/<gameid>/`.
//! - A `FetchResult` is sent for every definitive answer (success, 404, no match), possibly
//!   empty, so the app records `fetched_at` and doesn't ask again. Network errors, 5xx and rate
//!   limiting send nothing (next session retries) and pause the worker for a while.
//! - Files already downloaded are reused without a request (partially completed jobs resume).
//! - ProtonDB tiers for Steam games (keyless, same `steam_store` switch): asked along with a
//!   normal lookup, and by `AppState::queue_protondb` as a silent background refresh at most
//!   once per ~2 weeks per game (`protondb::REFRESH_SECS`, timestamps in `<art_dir>/protondb.json`).
//!   Background jobs wait behind every user-visible job and don't count in `pending()`.
//! - IGDB (with the user's Twitch client id + secret and `igdb_metadata` on): games missing a
//!   description, companies, genres or year, or every IGDB-only fact (scores, series, themes,
//!   modes), are matched by Steam appid (IGDB external games) or exact name, and IGDB's answer
//!   only fills what the Steam store left empty (`Metadata::fill_from`; user edits always win).
//! - Metadata add-ons (`set_addons`): after the built-in sources, games still missing a
//!   description or art are looked up through the enabled metadata add-ons
//!   (`addons::metadata::lookup`). Their fields only fill gaps; their art URLs are downloaded
//!   into `<art_dir>/addon/<gameid>/`.
//! - Wikipedia (no key, `wikipedia` on): after the Steam store and IGDB, a game still without a
//!   description gets the lead section of its English Wikipedia article (confident matches only,
//!   see `wikipedia`); with `prefer_wikipedia` the article's text replaces the store's. The
//!   article's URL travels with the text (`Metadata::wikipedia`) so the game page can credit it.
//!   Games looked up before get one quiet Wikipedia-only background lookup each
//!   (`MetadataFetcher::request_wikipedia_backfill`).

pub(crate) mod fixture;
mod lookup_app;
pub mod parse;
pub mod protondb;
pub mod wikipedia;
mod worker;
pub use lookup_app::Lookups;
use worker::*;

use crate::config::MetadataConfig;
use crate::fsutil::write_atomic;
use crate::model::*;
use crossbeam_channel::{Receiver, Sender, unbounded};
use parking_lot::{Condvar, Mutex};
use parse::AppDetails;
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct FetchResult {
    pub id: GameId,
    pub art: Art,
    pub meta: Metadata,
    /// The answer of an IGDB-only background lookup (`MetadataFetcher::request_igdb_backfill`).
    pub backfill: bool,
    /// Wikipedia was asked for this game (a Wikipedia-only background lookup,
    /// `request_wikipedia_backfill`, or a full lookup that reached it): it isn't asked again.
    pub wiki_backfill: bool,
    /// The answer to a lookup the user asked for (`MetadataFetcher::lookup`): its token. Such
    /// answers go back to Edit Game Details, not to the DB.
    pub lookup: Option<u64>,
    /// A lookup the user asked for whose name search was ambiguous: the games it could be
    /// (`meta` then holds only what the other sources found).
    pub candidates: Vec<Candidate>,
    /// A lookup the user asked for that failed (network trouble …), in plain words.
    pub error: Option<String>,
}

impl FetchResult {
    fn empty(job: &Job) -> FetchResult {
        FetchResult {
            id: job.id,
            art: Art::default(),
            meta: Metadata::default(),
            backfill: job.backfill.is_some(),
            wiki_backfill: job.wiki.only,
            lookup: job.lookup.as_ref().map(|l| l.token),
            candidates: vec![],
            error: None,
        }
    }
}

/// A lookup the user asked for (Edit Game Details → Download metadata…): every source is asked
/// again whatever the game already has, with the name and platform typed in the dialog; the
/// answer goes back to the dialog (shown before Save), and an ambiguous name search comes back
/// as candidates to pick from instead of a guess.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LookupReq {
    pub token: u64,
    /// The game the user picked from the candidates of an earlier answer.
    pub pick: Option<Pick>,
    /// Also fill empty art slots (a game already in the library).
    pub with_art: bool,
    /// The scanner has its own description (Wikipedia then only replaces it when preferred).
    pub scan_description: bool,
}

/// Which candidate the user picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pick {
    /// An IGDB game id.
    Igdb(u64),
    /// A Wikipedia article title.
    Wikipedia(String),
}

/// One game an ambiguous name search found.
#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub pick: Pick,
    pub name: String,
    pub year: Option<i32>,
    /// "PlayStation 3, PC" (may be empty).
    pub platforms: String,
    /// Where it was found ("IGDB", "Wikipedia").
    pub source: &'static str,
}

/// At most this many candidates are offered.
const MAX_CANDIDATES: usize = 8;

/// Descriptive, as Wikimedia's API etiquette asks (no personal data).
pub const USER_AGENT: &str = concat!("Lodestar/", env!("CARGO_PKG_VERSION"), " (game launcher)");
const STEAM_CDN: &str = "https://shared.steamstatic.com/store_item_assets/steam/apps";
const STEAM_STORE: &str = "https://store.steampowered.com/api/appdetails";
const SGDB_API: &str = "https://www.steamgriddb.com/api/v2";
const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;
const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;
/// Minimum spacing of Steam store API requests: 300 s / 200 requests.
const STORE_GAP_MS: u64 = 1500;
/// After a network error / 5xx / 429, drop jobs for this long (they are retried next session).
const OFFLINE_BACKOFF: Duration = Duration::from_secs(120);

/// Which art slots a job should fill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Wanted {
    pub cover: bool,
    pub hero: bool,
    pub logo: bool,
    pub banner: bool,
    pub meta: bool,
    /// ProtonDB tier (Steam games only; the worker skips it while the cache entry is fresh).
    pub protondb: bool,
    /// Facts only IGDB has (scores, series, themes, modes) are missing.
    pub extra: bool,
}

impl Wanted {
    pub fn for_game(g: &Game) -> Wanted {
        let m = &g.meta;
        Wanted {
            cover: g.art.cover.is_none(),
            hero: g.art.hero.is_none(),
            logo: g.art.logo.is_none(),
            banner: g.art.banner.is_none(),
            meta: m.description.is_none() || m.developer.is_none() || m.genres.is_empty() || m.release_year.is_none(),
            protondb: false,
            extra: m.critic_score.is_none() && m.series.is_none() && m.themes.is_empty() && m.features.is_empty(),
        }
    }
    fn any_art(&self) -> bool {
        self.cover || self.hero || self.logo || self.banner
    }
    /// Only the ProtonDB tier (a background refresh).
    fn protondb_only(&self) -> bool {
        self.protondb && !self.any_art() && !self.meta
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Job {
    pub id: GameId,
    pub name: String,
    pub steam_appid: Option<u32>,
    pub want: Wanted,
    /// Ask ProtonDB again only when the last answer is older than this (seconds).
    pub protondb_max_age: i64,
    /// The tier the game already shows: a background refresh that learns nothing new sends
    /// no result (no DB write).
    pub known_protondb: Option<String>,
    /// An IGDB-only background lookup (IGDB set up after the game was looked up): only IGDB is
    /// asked, and only what this (the game's metadata at queue time) lacks is sent back.
    pub backfill: Option<Box<Metadata>>,
    /// IGDB platform id of a console game (emulated or a console platform): the IGDB name search
    /// only looks at games on it.
    pub igdb_platform: Option<u32>,
    /// What a Wikipedia lookup needs (default: Wikipedia isn't asked).
    pub wiki: WikiJob,
    /// A lookup the user asked for (see `LookupReq`).
    pub lookup: Option<LookupReq>,
}

/// The Wikipedia side of a job.
#[derive(Clone, Debug, Default)]
pub(crate) struct WikiJob {
    /// Wikipedia may supply the description: the user didn't write their own and it isn't
    /// Wikipedia's already.
    pub allowed: bool,
    /// The game has no description at all.
    pub missing: bool,
    /// Release year and platform (tell same-named games apart).
    pub year: Option<i32>,
    pub platform: Option<String>,
    /// A background lookup: only Wikipedia is asked.
    pub only: bool,
}

impl WikiJob {
    fn of(g: &Game) -> WikiJob {
        WikiJob {
            allowed: g.edits.description.is_none() && g.meta.wikipedia.is_none(),
            missing: g.meta.description.is_none(),
            year: g.meta.release_year,
            platform: crate::art::sources::art_platform(g).or_else(|| g.platform.clone()),
            only: false,
        }
    }
}

impl Job {
    pub fn from_game(g: &Game) -> Job {
        let steam_appid = match &g.launch {
            Some(LaunchSpec::Steam { appid }) => Some(*appid),
            _ if g.source == Source::Steam => g.source_id.parse().ok(),
            _ => None,
        };
        let mut want = Wanted::for_game(g);
        want.protondb = steam_appid.is_some();
        Job {
            id: g.id,
            name: g.display_name().to_string(),
            steam_appid,
            want,
            protondb_max_age: protondb::MANUAL_SECS,
            known_protondb: g.meta.protondb.clone(),
            backfill: None,
            igdb_platform: crate::igdb::platform_of(g),
            wiki: WikiJob::of(g),
            lookup: None,
        }
    }

    /// A lookup the user asked for: `g` is the game as typed in Edit Game Details (name,
    /// platform and hand edits). Every metadata source is asked again, whatever the game has
    /// (its answer replaces what was fetched before); art only for empty slots.
    pub fn for_lookup(g: &Game, req: LookupReq) -> Job {
        let mut j = Job::from_game(g);
        j.want.meta = true;
        j.want.extra = true;
        if !req.with_art {
            j.want.cover = false;
            j.want.hero = false;
            j.want.logo = false;
            j.want.banner = false;
        }
        let own_description = g.edits.description.as_deref().is_some_and(|d| !d.trim().is_empty());
        j.wiki = WikiJob {
            allowed: !own_description,
            missing: !req.scan_description,
            // Only the user's own year tells same-named articles apart: a year fetched before
            // may belong to the wrong game this lookup is meant to replace.
            year: g.edits.release_year.filter(|y| *y != CLEARED_YEAR),
            only: false,
            ..j.wiki
        };
        j.lookup = Some(req);
        j
    }

    /// Wikipedia-only background lookup of a game looked up before (None when Wikipedia has
    /// nothing to add: it has a description and Wikipedia isn't preferred, or the user wrote
    /// their own).
    pub fn wikipedia_backfill(g: &Game, prefer: bool) -> Option<Job> {
        let mut j = Job::from_game(g);
        if !j.wiki.allowed || !(j.wiki.missing || prefer) {
            return None;
        }
        j.want = Wanted::default();
        j.wiki.only = true;
        Some(j)
    }

    /// IGDB-only background lookup of a game already looked up before IGDB was set up (None
    /// when it misses nothing IGDB could fill).
    pub fn igdb_backfill(g: &Game) -> Option<Job> {
        let mut j = Job::from_game(g);
        let w = Wanted::for_game(g);
        if !(w.meta || w.extra) {
            return None;
        }
        j.want = Wanted { meta: w.meta, extra: w.extra, ..Wanted::default() };
        j.backfill = Some(Box::new(g.meta.clone()));
        Some(j)
    }

    /// Background ProtonDB refresh for one game (None when it has no Steam appid).
    pub fn protondb_refresh(g: &Game) -> Option<Job> {
        let mut j = Job::from_game(g);
        j.steam_appid?;
        j.want = Wanted { protondb: true, ..Wanted::default() };
        j.protondb_max_age = protondb::REFRESH_SECS;
        Some(j)
    }
}

struct Queue {
    jobs: VecDeque<Job>,
    /// Silent background jobs (ProtonDB refresh): run only when `jobs` is empty.
    background: VecDeque<Job>,
    /// Queued or in flight.
    ids: HashSet<GameId>,
}

struct Shared {
    q: Mutex<Queue>,
    cv: Condvar,
    cfg: Mutex<MetadataConfig>,
    /// Foreground jobs in flight (background ones don't count in `pending`).
    in_flight: AtomicUsize,
    shutdown: AtomicBool,
    /// ProtonDB answers by appid; loaded by the worker at start.
    proton: Arc<Mutex<protondb::Cache>>,
    /// Metadata add-ons to ask after the built-in sources (None / none enabled: skipped).
    addons: Mutex<Option<AddonSources>>,
}

/// The add-on manager and settings the worker asks metadata add-ons with.
#[derive(Clone)]
struct AddonSources {
    mgr: crate::addons::AddonManager,
    cfg: crate::config::AddonsConfig,
}

impl AddonSources {
    fn any(&self) -> bool {
        !self.mgr.enabled_of_kind(crate::addons::AddonKind::Metadata, &self.cfg).is_empty()
    }
}

pub struct MetadataFetcher {
    shared: Arc<Shared>,
    rx: Receiver<FetchResult>,
}

impl MetadataFetcher {
    pub fn new(ctx: &egui::Context, art_dir: PathBuf, cfg: &MetadataConfig) -> MetadataFetcher {
        let shared = Arc::new(Shared {
            q: Mutex::new(Queue { jobs: VecDeque::new(), background: VecDeque::new(), ids: HashSet::new() }),
            cv: Condvar::new(),
            cfg: Mutex::new(cfg.clone()),
            in_flight: AtomicUsize::new(0),
            shutdown: AtomicBool::new(false),
            proton: Arc::new(Mutex::new(protondb::Cache::default())),
            addons: Mutex::new(None),
        });
        let (tx, rx) = unbounded();
        let (sh, ctx) = (shared.clone(), ctx.clone());
        // Screenshot runs may answer from local fixture files (never the network).
        let fixtures = fixture::from_env();
        let spawned = std::thread::Builder::new().name("metadata".into()).spawn(move || {
            // Load off the UI thread; swapped in whole so `request` never waits on disk.
            let loaded = protondb::Cache::load(&protondb::Cache::path(&art_dir));
            *sh.proton.lock() = loaded;
            fn run<H: Http>(http: H, art_dir: PathBuf, sh: &Shared, tx: &Sender<FetchResult>, ctx: &egui::Context) {
                let mut w = Worker {
                    http,
                    art_dir,
                    throttle: Throttle::polite(),
                    sgdb_disabled: false,
                    proton: sh.proton.clone(),
                    protondb_paused_until: None,
                };
                run_worker(sh, tx, ctx, &mut w);
                w.save_protondb();
            }
            match fixtures {
                Some(f) => run(f, art_dir, &sh, &tx, &ctx),
                None => run(UreqHttp::new(), art_dir, &sh, &tx, &ctx),
            }
        });
        if let Err(e) = spawned {
            log::error!("metadata worker spawn failed: {e}");
        }
        MetadataFetcher { shared, rx }
    }

    pub fn set_config(&mut self, cfg: &MetadataConfig) {
        *self.shared.cfg.lock() = cfg.clone();
    }

    /// The add-ons to ask for metadata (a snapshot: call again after add-ons or their settings
    /// change). Cheap.
    pub fn set_addons(&mut self, mgr: &crate::addons::AddonManager, cfg: &crate::config::AddonsConfig) {
        *self.shared.addons.lock() = Some(AddonSources { mgr: mgr.clone(), cfg: cfg.clone() });
    }

    /// Any metadata add-on switched on (the app queues non-Steam games for them too).
    pub fn has_addon_sources(&self) -> bool {
        self.shared.addons.lock().as_ref().is_some_and(AddonSources::any)
    }

    /// Queue a lookup for `game` (ignored when already queued, when nothing is missing, or when
    /// no enabled provider applies to it).
    pub fn request(&self, game: &Game) {
        if crate::art::net::local_only() {
            return; // screenshot runs never go online (nor show "Fetching metadata")
        }
        let cfg = self.shared.cfg.lock().clone();
        let mut job = Job::from_game(game);
        self.drop_fresh_protondb(&mut job);
        if !eligible(&job, &cfg) && !(addon_eligible(&job) && self.has_addon_sources()) {
            return;
        }
        let mut q = self.shared.q.lock();
        if q.ids.insert(job.id) {
            q.jobs.push_back(job);
            self.shared.cv.notify_one();
        } else if let Some(i) = q.background.iter().position(|j| j.id == job.id) {
            // Waiting as a background refresh: the user asked, so move it to the front queue.
            q.background.remove(i);
            q.jobs.push_back(job);
            self.shared.cv.notify_one();
        }
    }

    /// Start a lookup the user asked for (see `LookupReq`); its answer comes back from `poll`
    /// with `lookup == Some(req.token)` — always exactly one answer. Err (in plain words) when
    /// no source could look this game up.
    pub fn lookup(&self, game: &Game, req: LookupReq) -> Result<(), String> {
        if crate::art::net::local_only() && !fixture::active() {
            return Err("Online lookups are off in this test run".into());
        }
        let cfg = self.shared.cfg.lock().clone();
        let mut job = Job::for_lookup(game, req);
        if game.display_name().trim().is_empty() && job.steam_appid.is_none() {
            return Err("Type the game's name first".into());
        }
        self.drop_fresh_protondb(&mut job);
        if !eligible(&job, &cfg) && !(addon_eligible(&job) && self.has_addon_sources()) {
            return Err(no_source_text(&cfg));
        }
        // Not deduplicated by game (a background job of the same game may be queued): the user
        // waits for this one, so it goes first.
        self.shared.q.lock().jobs.push_front(job);
        self.shared.cv.notify_one();
        Ok(())
    }

    /// Queue a silent IGDB-only background lookup for a game looked up before IGDB was set up
    /// (no-op when IGDB isn't set up, nothing is missing, or the game is already queued).
    pub fn request_igdb_backfill(&self, game: &Game) {
        if crate::art::net::local_only() {
            return; // screenshot runs never go online (nor show "Fetching metadata")
        }
        let cfg = self.shared.cfg.lock().clone();
        let Some(job) = Job::igdb_backfill(game).filter(|j| igdb_wanted(j, &cfg)) else {
            return;
        };
        let mut q = self.shared.q.lock();
        if q.ids.insert(job.id) {
            q.background.push_back(job);
            self.shared.cv.notify_one();
        }
    }

    /// Queue a silent Wikipedia-only background lookup for a game looked up before (no-op when
    /// Wikipedia is off, has nothing to add, or the game is already queued).
    pub fn request_wikipedia_backfill(&self, game: &Game) {
        if crate::art::net::local_only() {
            return; // screenshot runs never go online (nor show "Fetching metadata")
        }
        let cfg = self.shared.cfg.lock().clone();
        let Some(job) = Job::wikipedia_backfill(game, cfg.prefer_wikipedia).filter(|j| wikipedia_wanted(j, &cfg))
        else {
            return;
        };
        let mut q = self.shared.q.lock();
        if q.ids.insert(job.id) {
            q.background.push_back(job);
            self.shared.cv.notify_one();
        }
    }

    /// Queue a silent background ProtonDB refresh for a Steam game whose tier wasn't asked for
    /// in ~2 weeks (no-op otherwise, when disabled, or when the game is already queued).
    pub fn request_protondb(&self, game: &Game) {
        if crate::art::net::local_only() {
            return; // screenshot runs never go online (nor show "Fetching metadata")
        }
        if !self.shared.cfg.lock().steam_store {
            return;
        }
        let Some(mut job) = Job::protondb_refresh(game) else {
            return;
        };
        self.drop_fresh_protondb(&mut job);
        if !job.want.protondb {
            return;
        }
        let mut q = self.shared.q.lock();
        if q.ids.insert(job.id) {
            q.background.push_back(job);
            self.shared.cv.notify_one();
        }
    }

    fn drop_fresh_protondb(&self, job: &mut Job) {
        if let (true, Some(appid)) = (job.want.protondb, job.steam_appid) {
            let fresh = self.shared.proton.lock().is_fresh(appid, job.protondb_max_age, now_unix());
            if fresh {
                job.want.protondb = false;
            }
        }
    }

    pub fn poll(&mut self) -> Vec<FetchResult> {
        self.rx.try_iter().collect()
    }

    /// Jobs queued or in flight.
    pub fn pending(&self) -> usize {
        self.shared.q.lock().jobs.len() + self.shared.in_flight.load(Ordering::Relaxed)
    }

    /// The cached ProtonDB answer for a Steam appid (tier, unix time asked), if any.
    #[cfg(test)]
    pub fn protondb_cached(&self, appid: u32) -> Option<(Option<String>, i64)> {
        self.shared.proton.lock().get(appid).map(|e| (e.tier.clone(), e.at))
    }
}

impl Drop for MetadataFetcher {
    fn drop(&mut self) {
        self.shared.shutdown.store(true, Ordering::Relaxed);
        let mut q = self.shared.q.lock();
        q.jobs.clear();
        q.background.clear();
        drop(q);
        self.shared.cv.notify_all();
    }
}

impl crate::app::AppState {
    /// Silent background ProtonDB refresh for the library's Steam games (each asked at most once
    /// per ~2 weeks; nothing happens offline or with Steam store lookups switched off). Cheap:
    /// meant to run after every scan.
    pub fn queue_protondb(&self) {
        if !self.config.metadata.steam_store {
            return;
        }
        for g in &self.library.games {
            self.metadata.request_protondb(g);
        }
    }
}

fn sgdb_key(cfg: &MetadataConfig) -> Option<&str> {
    cfg.steamgriddb_key.as_deref().map(str::trim).filter(|k| !k.is_empty())
}

/// Why a lookup the user asked for can't run: no source applies to the game.
fn no_source_text(cfg: &MetadataConfig) -> String {
    if !cfg.wikipedia && cfg.igdb_credentials().is_none() {
        "No metadata source can look this game up: switch Wikipedia on or set up IGDB in Settings → Metadata".into()
    } else {
        "No metadata source can look this game up: set up IGDB in Settings → Metadata".into()
    }
}

fn eligible(job: &Job, cfg: &MetadataConfig) -> bool {
    if job.backfill.is_some() {
        return igdb_wanted(job, cfg);
    }
    if job.wiki.only {
        return wikipedia_wanted(job, cfg);
    }
    let w = job.want;
    if !w.any_art() && !w.meta && !w.protondb {
        return false;
    }
    let steam = job.steam_appid.is_some() && cfg.steam_store;
    let sgdb = sgdb_key(cfg).is_some() && (w.cover || w.hero || w.logo);
    steam || sgdb || igdb_wanted(job, cfg) || wikipedia_wanted(job, cfg)
}

/// Wikipedia is on and could supply this game's description.
fn wikipedia_wanted(job: &Job, cfg: &MetadataConfig) -> bool {
    cfg.wikipedia && wikipedia::allowed() && job.wiki.allowed && (job.wiki.missing || cfg.prefer_wikipedia)
}

/// IGDB is set up for metadata and the game misses something it could fill.
fn igdb_wanted(job: &Job, cfg: &MetadataConfig) -> bool {
    cfg.igdb_metadata && cfg.igdb_credentials().is_some() && (job.want.meta || job.want.extra)
}

/// Something an add-on could fill (description or art).
fn addon_eligible(job: &Job) -> bool {
    let w = job.want;
    w.meta || w.cover || w.hero || w.logo
}

fn run_worker(sh: &Shared, tx: &Sender<FetchResult>, ctx: &egui::Context, w: &mut Worker<impl Http>) {
    let mut offline_until: Option<Instant> = None;
    loop {
        let job = {
            let mut q = sh.q.lock();
            loop {
                if sh.shutdown.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(j) = q.jobs.pop_front() {
                    sh.in_flight.fetch_add(1, Ordering::Relaxed);
                    break (j, false);
                }
                if let Some(j) = q.background.pop_front() {
                    break (j, true);
                }
                sh.cv.wait(&mut q);
            }
        };
        let (job, background) = job;
        let cfg = sh.cfg.lock().clone();
        // The user waits for their own lookup: it is tried even while background lookups pause.
        let offline = job.lookup.is_none() && offline_until.is_some_and(|t| Instant::now() < t);
        let addons = sh.addons.lock().clone().filter(|a| a.any() && addon_eligible(&job) && !background);
        let builtin = eligible(&job, &cfg);
        if !offline && (builtin || addons.is_some()) {
            let r = if builtin { w.process(&job, &cfg) } else { Ok(FetchResult::empty(&job)) };
            let r = match (r, &addons) {
                (Ok(mut r), Some(a)) => {
                    w.ask_addons(&job, a, &mut r);
                    Ok(r)
                }
                (r, _) => r,
            };
            match r {
                Ok(r)
                    if job.want.protondb_only()
                        && (r.meta.protondb.is_none() || r.meta.protondb == job.known_protondb) =>
                {
                    // Nothing new (same tier, no report, paused): no DB write.
                }
                Ok(r) => {
                    let _ = tx.send(r);
                    ctx.request_repaint();
                }
                Err(Transient(e)) => {
                    log::info!("metadata: {} ({}): {e}; pausing online lookups", job.name, job.id);
                    offline_until = Some(Instant::now() + OFFLINE_BACKOFF);
                    if job.lookup.is_some() {
                        let mut r = FetchResult::empty(&job);
                        r.error =
                            Some("Couldn't reach the metadata sources. Check the connection and try again.".into());
                        let _ = tx.send(r);
                        ctx.request_repaint();
                    }
                }
            }
        } else if job.lookup.is_some() {
            // Nothing could be asked: still answer, so the dialog doesn't wait forever.
            let _ = tx.send(FetchResult::empty(&job));
            ctx.request_repaint();
        }
        if !background {
            sh.in_flight.fetch_sub(1, Ordering::Relaxed);
        }
        let idle = {
            let mut q = sh.q.lock();
            if job.lookup.is_none() {
                // (A lookup the user asked for never holds the game's place in the queue.)
                q.ids.remove(&job.id);
            }
            q.jobs.is_empty() && q.background.is_empty()
        };
        // Save the ProtonDB timestamps now and then (and when the queue runs dry), not per job.
        let dirty = w.proton.lock().dirty;
        if dirty > 0 && (idle || dirty >= 25) {
            w.save_protondb();
        }
    }
}

#[cfg(test)]
mod tests;
