//! Registers theme fonts with egui.
//!
//! CONTRACT:
//!   pub struct FontRegistry
//!   impl FontRegistry {
//!     pub fn new() -> Self
//!     /// Make sure every font of `themes` is registered (rebuilds egui fonts only when the set changes).
//!     pub fn ensure(&mut self, ctx: &egui::Context, themes: &[&Theme])
//!     /// Load the CJK fallback font when `text` needs it (call with game names).
//!     pub fn request_coverage(&mut self, text: &str)
//!     /// egui family for a font reference (name from [fonts] or relative path). Unknown → Proportional.
//!     pub fn family(&self, theme: &Theme, font: Option<&str>) -> egui::FontFamily
//!   }
//!
//! Font references (values in `[fonts]` or an element's `font`):
//!   * a name from `[fonts]` (`regular`, `bold`, `title`, extra names),
//!   * a path relative to the theme directory, or an absolute path (ttf/otf/ttc),
//!   * `system:<alias>` — the first matching font installed on the host, e.g. `system:sans`,
//!     `system:sans-light`, `system:sans-bold`, `system:sans-semibold`, `system:display`,
//!     `system:mono`, `system:serif`. Never required: when nothing matches, egui's built-in font
//!     is used.
//!   * `builtin:<name>` — a font compiled into Lodestar (`bundled.rs`: Playnite's Titillium Web);
//!     a build without it uses the matching `system:` alias instead.
//!
//! Files are read and validated on a background thread; until a font is ready (and active in
//! egui, i.e. from the pass after `set_fonts`), `family()` returns `Proportional`, so text never
//! references a family egui does not know (egui panics on unknown families and on unparsable
//! font data — hence `sfnt_looks_valid`).

pub mod bundled;

use super::{Element, TextStyle, Theme, View};
use crossbeam_channel::{Receiver, Sender, unbounded};
use egui::{FontData, FontDefinitions, FontFamily};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// Where a font comes from (deduplicated across themes).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum FontSource {
    File(PathBuf),
    System(String),
    /// Compiled in (`builtin:<name>`).
    Bundled(String),
}

impl FontSource {
    fn key(&self) -> String {
        match self {
            FontSource::File(p) => format!("file:{}", p.display()),
            FontSource::System(a) => format!("system:{a}"),
            FontSource::Bundled(n) => format!("builtin:{n}"),
        }
    }
}

struct Loaded {
    key: String,
    data: Option<Arc<FontData>>,
}

/// Host fonts appended to every family (after egui's built-in fonts) so glyphs that neither the
/// theme's font nor egui's cover (★ ▶ ● ✓ arrows, …) do not render as boxes.
const SYMBOL_FALLBACK: &str = "fallback-symbols";
const SYMBOL_FALLBACK2: &str = "fallback-symbols2";
/// Chinese / Japanese / Korean fallback: large (~20 MB), so only loaded when some text needs it
/// (see `FontRegistry::request_coverage`).
const CJK_FALLBACK: &str = "fallback-cjk";
/// Symbols built into the binary (a subset of DejaVu Sans: arrows, shapes, ✓ ✕ ★ ▶ ●, box drawing,
/// punctuation, Latin / Greek / Cyrillic), right after egui's own fonts in every family, so UI text
/// such as "Settings → Emulation" never shows boxes on a computer without DejaVu / Noto fonts.
const BUILTIN_SYMBOLS: &str = "lodestar-symbols";
static BUILTIN_SYMBOLS_TTF: &[u8] = include_bytes!("../../assets/fonts/LodestarSymbols.ttf");

/// True for characters only a CJK font covers (Han, Kana, Hangul, CJK punctuation, full-width forms).
pub fn needs_cjk(c: char) -> bool {
    matches!(c as u32,
        0x1100..=0x11FF   // Hangul Jamo
        | 0x2E80..=0x2FDF // CJK radicals
        | 0x3000..=0x30FF // CJK punctuation, Hiragana, Katakana
        | 0x3100..=0x31FF // Bopomofo, Hangul compat, Katakana ext
        | 0x3400..=0x4DBF // CJK ext A
        | 0x4E00..=0x9FFF // CJK unified
        | 0xA960..=0xA97F
        | 0xAC00..=0xD7AF // Hangul syllables
        | 0xF900..=0xFAFF // CJK compatibility
        | 0xFF00..=0xFFEF // half/full-width forms
        | 0x20000..=0x2FFFF)
}

pub struct FontRegistry {
    /// theme key -> font ref -> egui family name (only families that are active in egui).
    active: HashMap<String, HashMap<String, Arc<str>>>,
    /// Same shape, for families registered with egui but not active until the next pass.
    pending: Option<HashMap<String, HashMap<String, Arc<str>>>>,
    /// theme key -> font ref -> source key (everything the themes ask for).
    wanted: HashMap<String, HashMap<String, String>>,
    /// Source key -> font data (None = failed to load).
    ready: BTreeMap<String, Option<Arc<FontData>>>,
    /// Source keys of host fallback fonts, in chain order (symbols first, then CJK).
    fallbacks: Vec<String>,
    /// The symbol fallbacks were queued (once, on the first `ensure`).
    symbols_requested: bool,
    requested: HashSet<String>,
    tx: Sender<Loaded>,
    rx: Receiver<Loaded>,
    /// (theme key, theme pointer) of the last `ensure` call.
    last_sig: Vec<(String, usize)>,
    dirty: bool,
    /// A family name we registered, used to notice if someone else replaced egui's fonts.
    marker: Option<FontFamily>,
}

impl Default for FontRegistry {
    fn default() -> Self {
        let (tx, rx) = unbounded();
        FontRegistry {
            active: HashMap::new(),
            pending: None,
            wanted: HashMap::new(),
            ready: BTreeMap::new(),
            fallbacks: vec![],
            symbols_requested: false,
            requested: HashSet::new(),
            tx,
            rx,
            last_sig: vec![],
            dirty: false,
            marker: None,
        }
    }
}

impl FontRegistry {
    pub fn new() -> Self {
        FontRegistry::default()
    }

    /// Make sure every font of `themes` is registered (rebuilds egui fonts only when the set changes).
    pub fn ensure(&mut self, ctx: &egui::Context, themes: &[&Theme]) {
        // Families registered last pass are active now.
        if let Some(p) = self.pending.take() {
            self.active = p;
        }
        // Someone else replaced egui's font definitions: register ours again.
        if let Some(m) = &self.marker {
            let present = ctx.fonts(|f| f.definitions().families.contains_key(m));
            if !present {
                self.active.clear();
                self.dirty = true;
            }
        }
        if !self.symbols_requested {
            self.symbols_requested = true;
            // Register the built-in symbol font right away (host fallbacks follow when loaded).
            self.dirty = true;
            self.request_fallbacks(&[SYMBOL_FALLBACK, SYMBOL_FALLBACK2]);
        }
        while let Ok(l) = self.rx.try_recv() {
            if l.data.is_none() {
                log::info!("font {} unavailable; using the built-in font", l.key);
            }
            self.ready.insert(l.key, l.data);
            self.dirty = true;
        }
        let sig: Vec<(String, usize)> = themes.iter().map(|t| (t.key.clone(), *t as *const Theme as usize)).collect();
        if sig != self.last_sig {
            self.last_sig = sig;
            self.collect(themes);
        }
        if self.dirty {
            self.dirty = false;
            self.rebuild(ctx);
        }
    }

    /// Make sure glyphs of `text` can be drawn: loads the CJK fallback font (once, in the
    /// background) when `text` contains Chinese / Japanese / Korean characters. Cheap to call
    /// with every game name whenever the library changes.
    pub fn request_coverage(&mut self, text: &str) {
        // Three fallbacks at most, so this scan is cheap; no allocation per call.
        let requested = self.fallbacks.iter().any(|k| k.ends_with(CJK_FALLBACK));
        if !requested && !text.is_ascii() && text.chars().any(needs_cjk) {
            self.request_fallback(CJK_FALLBACK);
        }
    }

    fn request_fallback(&mut self, alias: &str) {
        self.request_fallbacks(&[alias]);
    }

    /// Queue host fallback fonts (chain order = call order), loaded together on one thread so
    /// they usually land in a single font rebuild.
    fn request_fallbacks(&mut self, aliases: &[&str]) {
        let mut to_load = vec![];
        for alias in aliases {
            let src = FontSource::System((*alias).into());
            let key = src.key();
            if self.fallbacks.contains(&key) {
                continue;
            }
            self.fallbacks.push(key.clone());
            if self.requested.insert(key.clone()) {
                to_load.push((key, src));
            }
        }
        if !to_load.is_empty() {
            self.spawn_loader(to_load);
        }
    }

    fn spawn_loader(&self, to_load: Vec<(String, FontSource)>) {
        let tx = self.tx.clone();
        let spawned = std::thread::Builder::new().name("fonts".into()).spawn(move || {
            for (key, src) in to_load {
                let data = load_source(&src).map(Arc::new);
                if tx.send(Loaded { key, data }).is_err() {
                    return;
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("font loader thread: {e}");
        }
    }

    /// egui family for a font reference (name from [fonts] or relative path). Unknown → Proportional.
    pub fn family(&self, theme: &Theme, font: Option<&str>) -> FontFamily {
        let Some(map) = self.active.get(theme.key.as_str()) else {
            return FontFamily::Proportional;
        };
        let r = font.map(str::trim).filter(|s| !s.is_empty()).unwrap_or("regular");
        if let Some(n) = map.get(r) {
            return FontFamily::Name(n.clone());
        }
        // A missing bold/title face falls back to the theme's regular face.
        if r != "regular"
            && theme_has_name(theme, r)
            && let Some(n) = map.get("regular")
        {
            return FontFamily::Name(n.clone());
        }
        FontFamily::Proportional
    }

    fn collect(&mut self, themes: &[&Theme]) {
        let mut to_load: Vec<(String, FontSource)> = vec![];
        for t in themes {
            let mut refs: Vec<String> = vec!["regular".into(), "bold".into(), "title".into()];
            refs.extend(t.file.fonts.extra.keys().cloned());
            theme_element_fonts(t, &mut refs);
            refs.sort();
            refs.dedup();
            let mut map = HashMap::new();
            for r in refs {
                let Some(src) = resolve_ref(t, &r) else {
                    continue;
                };
                let key = src.key();
                if self.requested.insert(key.clone()) {
                    to_load.push((key.clone(), src));
                }
                map.insert(r, key);
            }
            if self.wanted.get(&t.key) != Some(&map) {
                self.wanted.insert(t.key.clone(), map);
                self.dirty = true;
            }
        }
        if !to_load.is_empty() {
            self.spawn_loader(to_load);
        }
    }

    fn rebuild(&mut self, ctx: &egui::Context) {
        let mut defs = FontDefinitions::default();
        // Host fallbacks that loaded, in order; appended to every family (egui's own included).
        let host: Vec<String> =
            self.fallbacks.iter().filter(|k| matches!(self.ready.get(*k), Some(Some(_)))).cloned().collect();
        defs.font_data.insert(BUILTIN_SYMBOLS.into(), Arc::new(FontData::from_static(BUILTIN_SYMBOLS_TTF)));
        // Appended after each family's own fonts: built-in symbols first, then host fallbacks.
        let tail: Vec<String> = std::iter::once(BUILTIN_SYMBOLS.to_string()).chain(host.iter().cloned()).collect();
        let fallback: Vec<String> = defs
            .families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .chain(tail.iter().cloned())
            .collect();
        let mut next: HashMap<String, HashMap<String, Arc<str>>> = HashMap::new();
        let mut used: HashSet<&str> = host.iter().map(String::as_str).collect();
        for (theme, refs) in &self.wanted {
            for (r, src_key) in refs {
                let Some(Some(_)) = self.ready.get(src_key) else {
                    continue;
                };
                let fam: Arc<str> = format!("{theme}:{r}").into();
                let mut chain = vec![src_key.clone()];
                chain.extend(fallback.iter().filter(|k| *k != src_key).cloned());
                defs.families.insert(FontFamily::Name(fam.clone()), chain);
                next.entry(theme.clone()).or_default().insert(r.clone(), fam);
                used.insert(src_key.as_str());
            }
        }
        for fam in [FontFamily::Proportional, FontFamily::Monospace] {
            if let Some(chain) = defs.families.get_mut(&fam) {
                chain.extend(tail.iter().cloned());
            }
        }
        for (k, d) in &self.ready {
            if let Some(d) = d
                && used.contains(k.as_str())
            {
                defs.font_data.insert(k.clone(), d.clone());
            }
        }
        // A family of our own, only used to notice when someone else replaces egui's fonts.
        let marker = FontFamily::Name(MARKER.into());
        defs.families.insert(marker.clone(), fallback);
        self.marker = Some(marker);
        ctx.set_fonts(defs);
        self.pending = Some(next);
        ctx.request_repaint();
    }
}

const MARKER: &str = "lodestar:fallback";

fn theme_has_name(t: &Theme, r: &str) -> bool {
    matches!(r, "bold" | "title") || t.file.fonts.extra.contains_key(r)
}

/// Font reference → where to load it from.
fn resolve_ref(t: &Theme, r: &str) -> Option<FontSource> {
    let f = &t.file.fonts;
    let named = match r {
        "regular" => Some(f.regular.as_deref()),
        "bold" => Some(f.bold.as_deref()),
        "title" => Some(f.title.as_deref()),
        other => f.extra.get(other).map(|s| Some(s.as_str())),
    };
    let value = match named {
        Some(Some(v)) => v,
        Some(None) => return None,
        None => r,
    };
    let value = value.trim();
    if let Some(alias) = value.strip_prefix("system:") {
        return Some(FontSource::System(alias.trim().to_lowercase()));
    }
    if let Some(name) = value.strip_prefix("builtin:") {
        return Some(FontSource::Bundled(name.trim().to_lowercase()));
    }
    if value.is_empty() || !(value.contains('.') || value.contains('/')) {
        return None;
    }
    t.resolve(value).map(FontSource::File)
}

fn style_font(s: &TextStyle, out: &mut Vec<String>) {
    if let Some(f) = &s.font {
        out.push(f.clone());
    }
}

fn view_fonts(v: &View, out: &mut Vec<String>) {
    // Also the texts inside stacks and item templates.
    for top in &v.elements {
        top.for_each_nested(&mut |e| element_fonts(e, out));
    }
}

fn element_fonts(e: &Element, out: &mut Vec<String>) {
    match e {
        Element::Text { style, .. }
        | Element::TextList { style, .. }
        | Element::DateTime { style, .. }
        | Element::Rail { style, .. }
        | Element::HelpBar { style, .. } => style_font(style, out),
        Element::Rows { title_style, item_text_style, .. } => {
            style_font(title_style, out);
            style_font(item_text_style, out);
        }
        Element::Carousel { item_text_style, .. } | Element::Grid { item_text_style, .. } => {
            style_font(item_text_style, out)
        }
        _ => {}
    }
}

fn table_fonts(t: &toml::Table, out: &mut Vec<String>) {
    for (k, v) in t {
        match v {
            toml::Value::String(s) if k == "font" => out.push(s.clone()),
            toml::Value::Table(inner) => table_fonts(inner, out),
            // A stack's `children`, item templates.
            toml::Value::Array(items) => {
                for i in items {
                    if let toml::Value::Table(inner) = i {
                        table_fonts(inner, out);
                    }
                }
            }
            _ => {}
        }
    }
}

fn theme_element_fonts(t: &Theme, out: &mut Vec<String>) {
    let f = &t.file;
    view_fonts(&f.collections, out);
    view_fonts(&f.games, out);
    for d in f.details.iter().chain(&f.home) {
        view_fonts(d, out);
    }
    for o in f.overrides.values().chain(f.variants.values()) {
        for m in [&o.collections, &o.games, &o.details, &o.home] {
            for tbl in m.values() {
                table_fonts(tbl, out);
            }
        }
    }
}

// ---------------------------------------------------------------- loading

const MAX_FONT_BYTES: u64 = 40 * 1024 * 1024;

/// The font a `system:` / `builtin:` font value draws with (the Playnite importer measures
/// text with it); None for other values or when nothing on this machine matches.
pub(crate) fn value_data(value: &str) -> Option<FontData> {
    let v = value.trim();
    let src = if let Some(a) = v.strip_prefix("system:") {
        FontSource::System(a.trim().to_lowercase())
    } else {
        FontSource::Bundled(v.strip_prefix("builtin:")?.trim().to_lowercase())
    };
    load_source(&src)
}

fn load_source(src: &FontSource) -> Option<FontData> {
    match src {
        FontSource::File(p) => read_font(p).map(FontData::from_owned),
        FontSource::Bundled(name) => match bundled::data(name) {
            Some(d) => Some(FontData::from_static(d)),
            None => load_source(&FontSource::System(bundled::stand_in(name).into())),
        },
        FontSource::System(alias) if alias.starts_with("fallback-") => {
            // Fallbacks are used as-is (no weight tweak) from the first candidate present.
            // Big files (the CJK font is ~20 MB) in system directories are memory-mapped: only
            // the pages egui actually touches (tables + the glyphs drawn) become resident.
            let index = system_index();
            system_candidates(alias).into_iter().find_map(|stem| {
                let path = index.get(stem)?;
                let data = map_font(path).or_else(|| read_font(path).map(FontData::from_owned))?;
                log::info!("font {alias} -> {}", path.display());
                Some(data)
            })
        }
        FontSource::System(alias) => {
            let index = system_index();
            for cand in system_candidates(alias) {
                let (stem, weight) = match cand.split_once('@') {
                    Some((s, w)) => (s, w.parse::<f32>().ok()),
                    None => (cand, None),
                };
                let Some(path) = index.get(stem) else {
                    continue;
                };
                if let Some(bytes) = read_font(path) {
                    log::info!("font system:{alias} -> {}", path.display());
                    let mut d = FontData::from_owned(bytes);
                    if let Some(w) = weight {
                        let mut tweak = egui::FontTweak::default();
                        tweak.coords.push(b"wght", w);
                        d = d.tweak(tweak);
                    }
                    return Some(d);
                }
            }
            None
        }
    }
}

fn read_font(p: &Path) -> Option<Vec<u8>> {
    let meta = std::fs::metadata(p).ok()?;
    if !meta.is_file() || meta.len() > MAX_FONT_BYTES {
        return None;
    }
    let bytes = std::fs::read(p).ok()?;
    if sfnt_looks_valid(&bytes) {
        Some(bytes)
    } else {
        log::warn!("font {} is not a usable TrueType/OpenType file", p.display());
        None
    }
}

/// Files at least this big are memory-mapped instead of read (see [`map_font`]).
const MAP_MIN_BYTES: u64 = 2 * 1024 * 1024;

/// Read-only private mapping of a whole file, unmapped on drop.
struct Mapped {
    ptr: *const u8,
    len: usize,
}

// SAFETY: the mapping is read-only and owned; the pointer is valid until drop.
unsafe impl Send for Mapped {}
unsafe impl Sync for Mapped {}

impl AsRef<[u8]> for Mapped {
    fn as_ref(&self) -> &[u8] {
        // SAFETY: `ptr..ptr+len` is a live PROT_READ mapping for the lifetime of `self`.
        unsafe { std::slice::from_raw_parts(self.ptr, self.len) }
    }
}

impl Drop for Mapped {
    fn drop(&mut self) {
        // SAFETY: exactly the region returned by mmap in `map_file`.
        unsafe {
            libc::munmap(self.ptr as *mut libc::c_void, self.len);
        }
    }
}

fn map_file(p: &Path) -> Option<Mapped> {
    use std::os::fd::AsRawFd;
    let f = std::fs::File::open(p).ok()?;
    let len = usize::try_from(f.metadata().ok()?.len()).ok()?;
    if len == 0 {
        return None;
    }
    // SAFETY: plain read-only private mapping of an open file; checked for MAP_FAILED.
    let ptr = unsafe { libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_PRIVATE, f.as_raw_fd(), 0) };
    if ptr == libc::MAP_FAILED {
        return None;
    }
    Some(Mapped { ptr: ptr as *const u8, len })
}

/// Memory-map a large font from a root-owned system directory (files there are replaced, not
/// rewritten in place, so the mapping stays valid; user font dirs are read instead).
fn map_font(p: &Path) -> Option<FontData> {
    let system = p.starts_with("/usr/");
    let meta = std::fs::metadata(p).ok()?;
    if !system || !meta.is_file() || meta.len() < MAP_MIN_BYTES || meta.len() > MAX_MAPPED_BYTES {
        return None;
    }
    let m = map_file(p)?;
    if !sfnt_looks_valid(m.as_ref()) {
        log::warn!("font {} is not a usable TrueType/OpenType file", p.display());
        return None;
    }
    Some(FontData::from_blob(Arc::new(m), 0))
}

/// Mapped fonts cost address space, not memory, so they may be bigger than read ones.
const MAX_MAPPED_BYTES: u64 = 64 * 1024 * 1024;

/// Candidate file stems (lowercase, without extension / variation-axis suffix) for an alias,
/// best first. `stem@wght` selects a weight of a variable font.
fn system_candidates(alias: &str) -> Vec<&'static str> {
    match alias {
        "sans" | "regular" | "ui" => vec![
            "inter-regular",
            "intervariable@400",
            "inter@400",
            "notosans-regular",
            "notosans@400",
            "cantarell-regular",
            "cantarell-vf@400",
            "firasans-regular",
            "roboto-regular",
            "opensans-regular",
            "sourcesans3-regular",
            "ubuntu-regular",
            "dejavusans",
        ],
        "sans-light" | "light" => vec![
            "inter-light",
            "intervariable@300",
            "inter@300",
            "notosans-light",
            "notosans@300",
            "cantarell-light",
            "cantarell-vf@300",
            "firasans-light",
            "roboto-light",
            "opensans-light",
            "sourcesans3-light",
            "ubuntu-light",
            "inter-regular",
            "notosans-regular",
            "cantarell-regular",
            "dejavusans",
        ],
        "sans-medium" | "medium" => vec![
            "inter-medium",
            "intervariable@500",
            "inter@500",
            "notosans-medium",
            "notosans@500",
            "cantarell-regular",
            "firasans-medium",
            "roboto-medium",
            "opensans-medium",
            "ubuntu-medium",
            "notosans-regular",
            "dejavusans",
        ],
        "sans-semibold" | "semibold" => vec![
            "inter-semibold",
            "intervariable@600",
            "inter@600",
            "notosans-semibold",
            "notosans@600",
            "cantarell-bold",
            "cantarell-vf@650",
            "firasans-semibold",
            "roboto-medium",
            "opensans-semibold",
            "sourcesans3-semibold",
            "ubuntu-medium",
            "notosans-bold",
            "dejavusans-bold",
        ],
        "sans-bold" | "bold" => vec![
            "inter-bold",
            "intervariable@700",
            "inter@700",
            "notosans-bold",
            "notosans@700",
            "cantarell-bold",
            "cantarell-vf@700",
            "firasans-bold",
            "roboto-bold",
            "opensans-bold",
            "sourcesans3-bold",
            "ubuntu-bold",
            "dejavusans-bold",
        ],
        "display" | "title" => vec![
            "inter-extrabold",
            "intervariable@800",
            "inter@800",
            "notosans-extrabold",
            "notosans@800",
            "notosans-bold",
            "cantarell-extrabold",
            "cantarell-bold",
            "firasans-extrabold",
            "firasans-bold",
            "roboto-black",
            "roboto-bold",
            "ubuntu-bold",
            "dejavusans-bold",
        ],
        "condensed" => vec![
            "notosans-condensedsemibold",
            "notosans-condensedbold",
            "robotocondensed-bold",
            "firasanscondensed-semibold",
            "ubuntu-condensed",
            "dejavusanscondensed-bold",
            "notosans-bold",
        ],
        "mono" => vec![
            "jetbrainsmono-regular",
            "firamono-regular",
            "notosansmono-regular",
            "sourcecodepro-regular",
            "ubuntumono-regular",
            "dejavusansmono",
        ],
        // Broad symbol coverage (★ ☆ ▶ ● ■ ✓ ← → ♥ ♪ …), small files.
        "fallback-symbols" => vec!["dejavusans", "freesans"],
        // What DejaVu lacks: more dingbats, arrows, chess / card / game symbols, …
        "fallback-symbols2" => vec!["notosanssymbols2-regular", "notosanssymbols-regular"],
        "fallback-cjk" => vec![
            "notosanscjk-regular",
            "notosanscjksc-regular",
            "notosanscjkjp-regular",
            "notosanscjkkr-regular",
            "notosanscjktc-regular",
            "sourcehansans-regular",
            "sourcehansanssc-regular",
            "wqy-microhei",
            "wqy-zenhei",
            "droidsansfallbackfull",
            "droidsansfallback",
            "notosanscjk-medium",
            "notosanscjk-light",
        ],
        "serif" => vec!["notoserif-regular", "notoserif@400", "sourceserif4-regular", "dejavuserif"],
        _ => vec![],
    }
}

/// Lowercase file stem with any `[axes]` suffix removed: "NotoSans[wdth,wght].ttf" -> "notosans".
fn normalise_stem(name: &str) -> Option<String> {
    let lower = name.to_lowercase();
    let (stem, ext) = lower.rsplit_once('.')?;
    if !matches!(ext, "ttf" | "otf" | "ttc") {
        return None;
    }
    let stem = match stem.find('[') {
        Some(i) => &stem[..i],
        None => stem,
    };
    Some(stem.trim_end_matches(['-', '_']).to_string())
}

fn system_font_dirs() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("/usr/share/fonts"), PathBuf::from("/usr/local/share/fonts")];
    if let Some(h) = dirs::home_dir() {
        v.push(h.join(".local/share/fonts"));
        v.push(h.join(".fonts"));
    }
    v
}

/// stem -> path, built once (on the font thread) by a bounded walk of the font directories.
fn system_index() -> &'static HashMap<String, PathBuf> {
    static INDEX: OnceLock<HashMap<String, PathBuf>> = OnceLock::new();
    INDEX.get_or_init(|| build_index(&system_font_dirs()))
}

fn build_index(dirs: &[PathBuf]) -> HashMap<String, PathBuf> {
    let mut out: HashMap<String, PathBuf> = HashMap::new();
    let mut budget = 30_000usize;
    let mut stack: Vec<(PathBuf, u32)> = dirs.iter().rev().map(|d| (d.clone(), 0)).collect();
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if budget == 0 {
                return out;
            }
            budget -= 1;
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                if depth < 5 {
                    stack.push((p, depth + 1));
                }
            } else if let Some(stem) = e.file_name().to_str().and_then(normalise_stem) {
                // Prefer .ttf/.otf over .ttc for the same stem; first found wins otherwise.
                let is_ttc = p.extension().is_some_and(|x| x.eq_ignore_ascii_case("ttc"));
                match out.get(&stem) {
                    Some(prev) if !(prev.extension().is_some_and(|x| x.eq_ignore_ascii_case("ttc")) && !is_ttc) => {}
                    _ => {
                        out.insert(stem, p);
                    }
                }
            }
        }
    }
    out
}

fn be_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn be_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Conservative structural check of a TrueType/OpenType file (or the first face of a
/// collection): sane table directory, required tables present and in bounds, valid `head`.
/// egui panics on font data it cannot parse, so anything doubtful is rejected.
pub fn sfnt_looks_valid(b: &[u8]) -> bool {
    let mut base = 0usize;
    if b.get(0..4) == Some(b"ttcf") {
        let (Some(n), Some(off)) = (be_u32(b, 8), be_u32(b, 12)) else {
            return false;
        };
        if n == 0 {
            return false;
        }
        base = off as usize;
    }
    let Some(version) = be_u32(b, base) else {
        return false;
    };
    if !matches!(version, 0x0001_0000 | 0x4F54_544F /* OTTO */ | 0x7472_7565 /* true */) {
        return false;
    }
    let Some(num_tables) = be_u16(b, base + 4) else {
        return false;
    };
    if num_tables == 0 || num_tables > 512 {
        return false;
    }
    let mut have = HashSet::new();
    let mut head: Option<(usize, usize)> = None;
    for i in 0..num_tables as usize {
        let rec = base + 12 + i * 16;
        let Some(tag) = b.get(rec..rec + 4) else {
            return false;
        };
        let (Some(off), Some(len)) = (be_u32(b, rec + 8), be_u32(b, rec + 12)) else {
            return false;
        };
        let (off, len) = (off as usize, len as usize);
        if off.checked_add(len).is_none_or(|end| end > b.len()) {
            return false;
        }
        let tag: [u8; 4] = [tag[0], tag[1], tag[2], tag[3]];
        if &tag == b"head" {
            head = Some((off, len));
        }
        have.insert(tag);
    }
    let req = [b"cmap", b"head", b"hhea", b"maxp", b"hmtx"];
    if !req.iter().all(|t| have.contains(*t)) {
        return false;
    }
    let outlines =
        (have.contains(b"glyf") && have.contains(b"loca")) || have.contains(b"CFF ") || have.contains(b"CFF2");
    if !outlines {
        return false;
    }
    match head {
        Some((off, len)) if len >= 54 => be_u32(b, off + 12) == Some(0x5F0F_3CF5),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeFile;

    fn theme(key: &str, file: ThemeFile, dir: Option<PathBuf>) -> Theme {
        Theme { key: key.into(), file, dir }
    }

    #[test]
    fn fonts_inside_stacks_and_item_templates_are_found() {
        let file = ThemeFile::parse(
            r#"
[[games.elements]]
type = "grid"
[[games.elements.item_elements]]
type = "text"
text = "x"
style = { font = "fonts/item.ttf" }
[[games.elements]]
type = "stack"
[[games.elements.children]]
type = "stack"
[[games.elements.children.children]]
type = "text"
text = "y"
style = { font = "fonts/nested.ttf" }
"#,
        )
        .unwrap();
        let mut out = vec![];
        theme_element_fonts(&theme("t", file, None), &mut out);
        assert!(out.contains(&"fonts/item.ttf".to_string()), "{out:?}");
        assert!(out.contains(&"fonts/nested.ttf".to_string()), "{out:?}");
        let mut tbl = toml::Table::new();
        tbl.insert("children".into(), toml::Value::Array(vec![toml::Value::Table(toml::toml! { font = "o.ttf" })]));
        let mut out = vec![];
        table_fonts(&tbl, &mut out);
        assert_eq!(out, ["o.ttf"]);
    }

    #[test]
    fn rejects_garbage() {
        assert!(!sfnt_looks_valid(b""));
        assert!(!sfnt_looks_valid(b"\x00\x01\x00\x00garbage"));
        assert!(!sfnt_looks_valid(&[0u8; 2048]));
        let mut fake = vec![0u8; 64];
        fake[0..4].copy_from_slice(&[0, 1, 0, 0]);
        fake[4..6].copy_from_slice(&1u16.to_be_bytes());
        assert!(!sfnt_looks_valid(&fake));
    }

    #[test]
    fn accepts_egui_builtin_font() {
        let defs = FontDefinitions::default();
        for d in defs.font_data.values() {
            assert!(sfnt_looks_valid(d.bytes()), "egui's own fonts must validate");
        }
    }

    #[test]
    fn stems() {
        assert_eq!(normalise_stem("NotoSans[wdth,wght].ttf").as_deref(), Some("notosans"));
        assert_eq!(normalise_stem("Inter-Bold.otf").as_deref(), Some("inter-bold"));
        assert_eq!(normalise_stem("readme.txt"), None);
    }

    #[test]
    fn index_and_refs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        std::fs::write(dir.path().join("a/b/NotoSans-Bold.ttf"), b"x").unwrap();
        std::fs::write(dir.path().join("a/Cantarell-Regular.otf"), b"x").unwrap();
        let idx = build_index(&[dir.path().to_path_buf()]);
        assert!(idx.contains_key("notosans-bold"));
        assert!(idx.contains_key("cantarell-regular"));

        let mut f = ThemeFile::default();
        f.fonts.regular = Some("system:sans".into());
        f.fonts.bold = Some("fonts/B.ttf".into());
        let t = theme("T", f, Some(PathBuf::from("/themes/T")));
        assert_eq!(resolve_ref(&t, "regular"), Some(FontSource::System("sans".into())));
        assert_eq!(resolve_ref(&t, "bold"), Some(FontSource::File(PathBuf::from("/themes/T/fonts/B.ttf"))));
        assert_eq!(resolve_ref(&t, "title"), None);
        assert_eq!(resolve_ref(&t, "nonsense"), None);
        assert_eq!(resolve_ref(&t, "fonts/x.otf"), Some(FontSource::File(PathBuf::from("/themes/T/fonts/x.otf"))));
    }

    #[test]
    fn registers_font_file_and_falls_back() {
        // Write egui's own (valid) font to a temp theme dir and register it.
        let dir = tempfile::tempdir().unwrap();
        let defs = FontDefinitions::default();
        let bytes = defs.font_data.values().next().unwrap().bytes().to_vec();
        std::fs::write(dir.path().join("R.ttf"), &bytes).unwrap();
        std::fs::write(dir.path().join("broken.ttf"), b"not a font").unwrap();
        let mut f = ThemeFile::default();
        f.fonts.regular = Some("R.ttf".into());
        f.fonts.bold = Some("broken.ttf".into());
        let t = theme("T", f, Some(dir.path().to_path_buf()));
        let ctx = egui::Context::default();
        let mut reg = FontRegistry::new();
        let mut ok = false;
        for _ in 0..200 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                reg.ensure(ui.ctx(), &[&t]);
                // Using the family must never panic.
                let fam = reg.family(&t, None);
                ui.label(egui::RichText::new("hello").family(fam));
                let fam = reg.family(&t, Some("bold"));
                ui.label(egui::RichText::new("bold").family(fam));
            });
            out.textures_delta.clear();
            if reg.family(&t, None) != FontFamily::Proportional {
                ok = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(ok, "theme font should become active");
        // Broken bold falls back to the regular face, unknown refs to Proportional.
        assert_eq!(reg.family(&t, Some("bold")), reg.family(&t, None));
        assert_eq!(reg.family(&t, Some("nope")), FontFamily::Proportional);
    }

    /// Glyphs missing from egui's built-in fonts (★, ▶, ●, ✓, arrows) come from a host fallback
    /// font appended to every family — theme families and egui's own — instead of drawing boxes.
    #[test]
    fn host_fallback_covers_symbols() {
        if !system_candidates(SYMBOL_FALLBACK).iter().any(|s| system_index().contains_key(*s)) {
            eprintln!("no symbol fallback font on this host; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let defs = FontDefinitions::default();
        let bytes = defs.font_data.values().next().unwrap().bytes().to_vec();
        std::fs::write(dir.path().join("R.ttf"), &bytes).unwrap();
        let mut f = ThemeFile::default();
        f.fonts.regular = Some("R.ttf".into());
        let t = theme("T", f, Some(dir.path().to_path_buf()));
        let ctx = egui::Context::default();
        let mut reg = FontRegistry::new();
        let covered = |ctx: &egui::Context, fam: FontFamily| {
            ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::new(14.0, fam), "★▶●✓→"))
        };
        let mut ok = false;
        for _ in 0..300 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                reg.ensure(ui.ctx(), &[&t]);
            });
            out.textures_delta.clear();
            let themed = reg.family(&t, None);
            if themed != FontFamily::Proportional && covered(&ctx, themed) && covered(&ctx, FontFamily::Proportional) {
                ok = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(ok, "theme and default families should fall back to the host symbol font");
        assert!(!needs_cjk('a') && !needs_cjk('™') && needs_cjk('東') && needs_cjk('한'));
    }

    /// The symbols Lodestar's own text uses (→ ✓ ✕ ▸ ▾ ● …, the changelog included) are drawn by the
    /// built-in symbol font alone — no host font needed (without DejaVu they drew as boxes).
    #[test]
    fn builtin_symbols_cover_lodestar_text_without_host_fonts() {
        let ctx = egui::Context::default();
        let mut reg = FontRegistry::new();
        // Only the first pass: host fallbacks load on a thread and are not ready yet.
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| reg.ensure(ui.ctx(), &[]));
        out.textures_delta.clear();
        let ui_text: String = include_str!("../../CHANGELOG.md")
            .chars()
            .chain("←↑→↓□△▸▾●◦✓✕★▶⋯…·—–“”‘’×™©°±½⅓".chars())
            .filter(|c| !c.is_ascii() && !needs_cjk(*c) && !c.is_whitespace())
            .collect();
        for fam in [FontFamily::Proportional, FontFamily::Monospace] {
            let missing: String = ui_text
                .chars()
                .filter(|c| !ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::new(14.0, fam.clone()), &c.to_string())))
                .collect();
            assert!(missing.is_empty(), "{fam:?} lacks {missing}");
        }
    }

    /// CJK game names must not render as boxes: once requested, the CJK fallback is appended to
    /// Proportional, Monospace and every theme family, after the primary font.
    #[test]
    fn cjk_fallback_covers_every_family() {
        if !system_candidates(CJK_FALLBACK).iter().any(|s| system_index().contains_key(*s)) {
            eprintln!("no CJK font on this host; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let defs = FontDefinitions::default();
        let bytes = defs.font_data.values().next().unwrap().bytes().to_vec();
        std::fs::write(dir.path().join("R.ttf"), &bytes).unwrap();
        let mut f = ThemeFile::default();
        f.fonts.regular = Some("R.ttf".into());
        let t = theme("T", f, Some(dir.path().to_path_buf()));
        let ctx = egui::Context::default();
        let mut reg = FontRegistry::new();
        reg.request_coverage("ドラゴンクエスト 東方 한국어");
        let covered = |ctx: &egui::Context, fam: FontFamily| {
            ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::new(14.0, fam), "ドラゴン東方한"))
        };
        let mut ok = false;
        for _ in 0..500 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                reg.ensure(ui.ctx(), &[&t]);
            });
            out.textures_delta.clear();
            let themed = reg.family(&t, None);
            if themed != FontFamily::Proportional
                && covered(&ctx, themed.clone())
                && covered(&ctx, FontFamily::Proportional)
                && covered(&ctx, FontFamily::Monospace)
            {
                ok = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(ok, "every family should fall back to the host CJK font");
        // Fallbacks come after the primary fonts.
        let fams = ctx.fonts(|f| f.definitions().families.clone());
        let prop = &fams[&FontFamily::Proportional];
        let cjk = prop.iter().position(|k| k.ends_with(CJK_FALLBACK)).unwrap();
        assert!(cjk > 0, "CJK font must not replace the primary font");
        let mono = &fams[&FontFamily::Monospace];
        assert!(!mono[0].starts_with("system:fallback"));
        assert!(mono.iter().any(|k| k.ends_with(CJK_FALLBACK)));
    }

    #[test]
    fn maps_big_system_fonts_only() {
        let Some(path) = system_candidates(CJK_FALLBACK).iter().find_map(|s| system_index().get(*s)) else {
            eprintln!("no CJK font on this host; skipping");
            return;
        };
        if path.starts_with("/usr/") {
            let d = map_font(path).expect("large system font is mapped");
            assert!(sfnt_looks_valid(d.bytes()));
            assert_eq!(d.bytes().len() as u64, std::fs::metadata(path).unwrap().len());
        }
        // Small or non-system files are not mapped.
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("S.ttf");
        let defs = FontDefinitions::default();
        std::fs::write(&small, defs.font_data.values().next().unwrap().bytes()).unwrap();
        assert!(map_font(&small).is_none());
    }

    #[test]
    fn cjk_fallback_only_on_demand() {
        let mut reg = FontRegistry::new();
        reg.request_coverage("Crash Bandicoot™ N. Sane Trilogy");
        reg.request_coverage("Mirror's Edge™ Catalyst — Ünïcödé");
        assert!(reg.fallbacks.is_empty(), "no CJK text, no 20 MB font");
        reg.request_coverage("東方紅魔郷");
        reg.request_coverage("ドラゴンクエスト");
        assert_eq!(reg.fallbacks.len(), 1, "requested once");
    }
}
