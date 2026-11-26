//! Playnite theme importer: a `.pthm` package (a zip) or a theme folder (theme.yaml +
//! Constants.xaml / Media.xaml / Views … + Fonts / Images / audio) → a Lodestar theme.
//!
//! CONTRACT:
//!   pub enum Mode { Desktop, Fullscreen }
//!   pub enum Family { Classic, Row, Rail }   (the layout a fullscreen theme is laid onto)
//!   pub struct ImportOptions { family, media }
//!   pub struct ImportReport { key, dir, name, author, version, mode, layout, family, suggested,
//!                             groups, carried, missed, media_bytes, media_included, … }
//!   /// Import the theme at `src` into `<themes_dir>/<key>/` (theme.toml + fonts/ images/ media/).
//!   pub fn import(src: &Path, themes_dir: &Path) -> anyhow::Result<ImportReport>
//!   pub fn import_with(src: &Path, themes_dir: &Path, opts: &ImportOptions) -> anyhow::Result<ImportReport>
//!   /// Theme options (thememodifier.yaml, options.rs): import again from the theme's kept files
//!   /// with these values; then mark which options matter again (quick first, exact after).
//!   pub fn apply_options(themes_dir: &Path, key: &str, values: &BTreeMap<String, String>) -> anyhow::Result<ImportReport>
//!   pub fn refresh_option_effects(themes_dir: &Path, key: &str) -> anyhow::Result<bool>
//!   pub mod options / extras / loc / convert — theme options, banners and ThemeFileBinding
//!   pictures, LOC strings, Playnite's value converters (for the view compiler too).
//!
//! Playnite themes are WPF/XAML programs, so there's no 1:1 conversion. The
//! theme is read into a `PlayniteLook` (its colours, brushes with all their stops, the cover's
//! focus look, bars, fonts per role and sizes, logos, add-ons, videos and sounds), then laid
//! onto a Lodestar layout: fullscreen themes onto "Top Bar" (Playnite's own top bar /
//! cover grid / bottom bar), "Single Row" (PS5 / Xbox style) or Lodestar's "Fullscreen" (rail and
//! rows), picked from the shape of the theme's game list, with the others offered in the report;
//! desktop themes onto Lodestar's desktop (palette, fonts, the desktop style). The report groups
//! what came along and what can't. Nothing from the theme is ever executed.

mod compile;
pub mod convert;
mod defaults;
pub(crate) mod extract;
pub mod extras;
mod fill;
mod fonts;
mod layout;
pub mod loc;
pub(crate) mod look;
mod map;
mod media;
pub mod options;
pub mod report;
mod settings;
mod source;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_options;
mod tree;
mod xaml;

use super::model::*;
pub use fill::Family;
use report::Area;
pub use report::Group;
use serde::{Deserialize, Serialize};
use source::Source;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use xaml::{Res, Resources, Val};

/// Which Playnite mode a theme is for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    #[default]
    Desktop,
    Fullscreen,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Desktop => "Desktop",
            Mode::Fullscreen => "Fullscreen",
        }
    }
}

/// What an import did, in plain words for the report window / CLI.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ImportReport {
    /// Theme key (directory name under the themes dir).
    pub key: String,
    #[serde(skip)]
    pub dir: PathBuf,
    pub name: String,
    pub author: String,
    pub version: String,
    pub mode: Mode,
    /// Where it came from (folder or package path).
    pub source: String,
    /// The same, as a path (to import it again with other choices).
    #[serde(skip)]
    pub source_path: PathBuf,
    /// Playnite's theme id (theme.yaml `Id`).
    pub playnite_id: String,
    /// The Lodestar layout a fullscreen theme was laid onto ("" for desktop themes).
    pub layout: String,
    /// Fullscreen: the layout family used, and the one the theme's game list points to.
    pub family: Option<Family>,
    pub suggested: Option<Family>,
    /// The theme's own main screen could be translated (the Translated layout can be used).
    pub translatable: bool,
    /// The report, by area.
    pub groups: Vec<Group>,
    /// The same lines, flat (CLI).
    pub carried: Vec<String>,
    pub missed: Vec<String>,
    /// Size of the theme's videos and sounds Lodestar can use (0: it has none).
    pub media_bytes: u64,
    /// They were copied into the theme.
    pub media_included: bool,
    /// Copied videos need ffmpeg / copied sounds need a sound player, and it's missing.
    pub needs_video: bool,
    pub needs_audio: bool,
    /// Theme options Lodestar can apply (thememodifier.yaml), and the values chosen.
    pub options: usize,
    pub option_values: BTreeMap<String, String>,
}

/// Choices for an import (None = Lodestar decides).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ImportOptions {
    /// The layout a fullscreen theme is laid onto (None: from the theme's game list).
    pub family: Option<Family>,
    /// Copy the theme's videos and sounds (None: when they are under 150 MB).
    pub media: Option<bool>,
}

const RECORD: &str = "playnite-import.toml";
const XAML_LIMIT: u64 = 4 * 1024 * 1024;
const FONT_LIMIT: u64 = 30 * 1024 * 1024;
const IMAGE_LIMIT: u64 = 32 * 1024 * 1024;
const MAX_XAML: usize = 400;
const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];
const VIDEO_EXTS: [&str; 6] = ["mp4", "wmv", "avi", "webm", "mkv", "mov"];
const SOUND_EXTS: [&str; 4] = ["wav", "mp3", "ogg", "flac"];

fn ext(rel: &str) -> String {
    rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

fn base_name(rel: &str) -> &str {
    rel.rsplit('/').next().unwrap_or(rel)
}

/// theme.yaml: flat `Key: Value` lines (that's all Playnite writes there).
fn parse_manifest(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            let k = k.trim().trim_start_matches('\u{feff}');
            if k.is_empty() || k.contains(' ') {
                return None;
            }
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'').trim();
            Some((k.to_string(), v.to_string()))
        })
        .collect()
}

/// A theme file a XAML path points at: `{ThemeFile 'Images/bg.jpg'}`, `../Images/bg.jpg`,
/// `pack://siteoforigin:,,,/Themes/Fullscreen/X/Images/bg.jpg`, `Images\bg.jpg` …
fn resolve_file(files: &[(String, u64)], raw: &str) -> Option<String> {
    let mut s = raw.trim().trim_matches(|c| c == '{' || c == '}').trim().to_string();
    if let Some(r) = s.strip_prefix("ThemeFile") {
        s = r.trim().trim_matches('\'').trim_matches('"').to_string();
    }
    if let Some((_, r)) = s.split_once(",,,") {
        s = r.to_string();
    }
    if let Some((_, r)) = s.split_once(";component/") {
        s = r.to_string();
    }
    let s = s.replace('\\', "/");
    let parts: Vec<&str> = s.split('/').filter(|c| !c.is_empty() && *c != "." && *c != "..").collect();
    if parts.is_empty() {
        return None;
    }
    let lower: Vec<String> = files.iter().map(|(f, _)| f.to_ascii_lowercase()).collect();
    for start in 0..parts.len() {
        let want = parts[start..].join("/").to_ascii_lowercase();
        if let Some(i) = lower.iter().position(|f| *f == want) {
            return Some(files[i].0.clone());
        }
    }
    let name = parts.last()?.to_ascii_lowercase();
    let mut by_name = files.iter().filter(|(f, _)| base_name(f).eq_ignore_ascii_case(&name));
    let first = by_name.next()?;
    by_name.next().is_none().then(|| first.0.clone())
}

/// Import a Playnite theme package or folder with Lodestar's own choices. See the module docs.
pub fn import(src: &Path, themes_dir: &Path) -> anyhow::Result<ImportReport> {
    import_with(src, themes_dir, &ImportOptions::default())
}

/// Windows fonts themes name (not shipped by Playnite or the theme; Lodestar's font stands in).
const WINDOWS_FONTS: [&str; 14] = [
    "segoe ui",
    "segoe ui light",
    "segoe ui semibold",
    "segoe ui semilight",
    "segoe ui black",
    "trebuchet ms",
    "arial",
    "calibri",
    "verdana",
    "tahoma",
    "bahnschrift",
    "consolas",
    "franklin gothic",
    "corbel",
];

fn is_titillium(family: &str) -> bool {
    fonts::norm(family).starts_with("titilliumweb")
}

/// The newest ThemeApiVersion the importer was checked with.
const TESTED_API: (u32, u32) = (2, 10);

fn api_newer(v: &str) -> bool {
    let mut it = v.trim().split('.').map(|p| p.trim().parse::<u32>().unwrap_or(0));
    let (a, b) = (it.next().unwrap_or(0), it.next().unwrap_or(0));
    (a, b) > TESTED_API
}

/// Import with `opts` (a layout family, videos and sounds or not). Theme options chosen for an
/// earlier import of the same theme are kept.
pub fn import_with(src: &Path, themes_dir: &Path, opts: &ImportOptions) -> anyhow::Result<ImportReport> {
    import_with_values(src, themes_dir, opts, None)
}

/// Apply theme options to an imported Playnite theme (`key` in `themes_dir`): the import runs
/// again from the theme's kept files with `values` (option key → value; options left out keep
/// the theme's own value), keeping its layout, videos and where it was installed from.
/// Serialises rebuilding a theme for its options with writing its re-marked options: a
/// refresh that started before a newer rebuild must not write its older values over it.
static OPTIONS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn apply_options(themes_dir: &Path, key: &str, values: &BTreeMap<String, String>) -> anyhow::Result<ImportReport> {
    let _guard = OPTIONS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let dir = themes_dir.join(key);
    let rec: ImportReport = toml::from_str(&std::fs::read_to_string(dir.join(RECORD))?)?;
    let kept = dir.join(options::SNAPSHOT);
    let src = if kept.join("theme.yaml").is_file() { kept } else { PathBuf::from(&rec.source) };
    if !src.exists() {
        anyhow::bail!("the theme's files aren't there any more; import the theme again");
    }
    let opts = ImportOptions { family: rec.family, media: Some(rec.media_included) };
    let known = std::fs::read_to_string(dir.join(options::FILE)).ok().and_then(|t| toml::from_str(&t).ok());
    let rep = import_inner(&src, themes_dir, &opts, Some(values), known.as_ref())?;
    if rep.key != key {
        log::warn!("theme options: {key} was imported again as {}", rep.key);
    }
    Ok(rep)
}

/// Which options change what Lodestar draws may depend on the values chosen (a banner's place
/// matters once banners are on): after [`apply_options`] reused the earlier marks to be quick,
/// mark them again from the theme's kept files. Only `playnite-options.toml` is rewritten;
/// true when the list of options to show changed.
pub fn refresh_option_effects(themes_dir: &Path, key: &str) -> anyhow::Result<bool> {
    let dir = themes_dir.join(key);
    let old_text = std::fs::read_to_string(dir.join(options::FILE))?;
    let old: options::OptionsSpec = toml::from_str(&old_text)?;
    let rec: ImportReport = toml::from_str(&std::fs::read_to_string(dir.join(RECORD))?)?;
    let source = Source::open(&dir.join(options::SNAPSHOT))?;
    let Some((spec, _)) = options::prepare(&source, rec.mode, &old.values, None) else { return Ok(false) };
    let changed = spec.options.iter().map(|o| o.effect).ne(old.options.iter().map(|o| o.effect));
    if !changed {
        return Ok(false);
    }
    // Only over the options this was worked out from (not a newer rebuild's).
    let _guard = OPTIONS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if std::fs::read_to_string(dir.join(options::FILE)).ok().as_deref() != Some(old_text.as_str()) {
        return Ok(false);
    }
    spec.write(&dir)?;
    Ok(true)
}

/// [`import_with`] with theme option values (None: those of an earlier import of the theme).
pub fn import_with_values(
    src: &Path,
    themes_dir: &Path,
    opts: &ImportOptions,
    values: Option<&BTreeMap<String, String>>,
) -> anyhow::Result<ImportReport> {
    import_inner(src, themes_dir, opts, values, None)
}

fn import_inner(
    src: &Path,
    themes_dir: &Path,
    opts: &ImportOptions,
    values: Option<&BTreeMap<String, String>>,
    known: Option<&options::OptionsSpec>,
) -> anyhow::Result<ImportReport> {
    let source = Source::open(src)?;
    // Re-applying options reads the theme's copy kept inside the imported theme.
    let from_kept = matches!(&source, Source::Dir(d) if d.file_name().is_some_and(|n| n == options::SNAPSHOT));
    let files = source.files();
    let manifest = source.read_text("theme.yaml", source::YAML_LIMIT).map(|t| parse_manifest(&t)).unwrap_or_default();
    let get = |k: &str| {
        manifest.iter().find(|(m, _)| m.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone()).filter(|v| !v.is_empty())
    };
    let name = get("Name").unwrap_or_else(|| fallback_name(src));
    let mode = match get("Mode") {
        Some(m) if m.eq_ignore_ascii_case("fullscreen") => Mode::Fullscreen,
        Some(_) => Mode::Desktop,
        None if source.display().replace('\\', "/").to_ascii_lowercase().contains("themes/fullscreen/") => {
            Mode::Fullscreen
        }
        None => Mode::Desktop,
    };
    let fullscreen = mode == Mode::Fullscreen;
    let mut rep = ImportReport {
        name: name.clone(),
        author: get("Author").unwrap_or_default(),
        version: get("Version").unwrap_or_default(),
        mode,
        source: source.display(),
        source_path: src.to_path_buf(),
        playnite_id: get("Id").unwrap_or_default(),
        ..Default::default()
    };
    let mut out = report::Builder::default();
    std::fs::create_dir_all(themes_dir)?;
    let key = pick_key(themes_dir, &super::sanitize_key(&name), mode)?;
    let earlier_dir = themes_dir.join(&key);
    let earlier: Option<ImportReport> =
        std::fs::read_to_string(earlier_dir.join(RECORD)).ok().and_then(|t| toml::from_str(&t).ok());

    let xaml = xaml_files(&files);
    if xaml.is_empty() && manifest.is_empty() {
        anyhow::bail!("{} doesn't look like a Playnite theme (no theme.yaml, no XAML files)", source.display());
    }
    let scan = scan_xaml(&source, &xaml, &rep.source);

    // Theme options (thememodifier.yaml, and Playnite settings that are the player's choice):
    // typed, tried, and the chosen values laid over the theme.
    let wanted_values = match values {
        Some(v) => v.clone(),
        None => earlier.as_ref().map(|e| e.option_values.clone()).unwrap_or_default(),
    };
    let prepared = options::prepare(&source, mode, &wanted_values, known);
    let chosen = prepared.as_ref().map(|(spec, v)| options::choices(spec, v)).unwrap_or_default();
    // The theme's own words (its Localization/en_US.xaml, its constants, chosen option labels).
    let mut strings = loc::Strings::load(&source);
    if let Some(x) = &chosen.constants {
        strings.extend_from_xaml(x);
    }
    // The theme's look and its resources, resolved over Playnite's full default theme.
    let extracted = extract::extract_with(&source, mode, &chosen);
    let banner = extracted.banner;
    let mut look = extracted.look;
    let res = extracted.resources;

    // Background picture (its colour is the background colour under it). Desktop: the
    // picture the theme's window really draws (the look), not any resource named "background".
    let picture = if fullscreen { background_picture(&res, &files, mode) } else { desktop_picture(&look, &files) };
    let picture_color = if fullscreen {
        match &picture {
            Some(Ok(rel)) => source.read(rel, IMAGE_LIMIT).and_then(|b| average_color(&b)),
            _ => None,
        }
    } else {
        window_image_color(&look, &source)
    };
    let mapped = colours(&res, &mut look, mode, picture_color, &mut out);
    let mut fonts = TextFonts::pick(&source, &files, &res, &look, mode, &mut out);

    // The theme's own main screen and details page, translated (compile.rs); Lodestar's
    // layout families (fill.rs) are the fallback and stay on offer.
    let compiled = fullscreen.then(|| fonts.compile(&source, &look, &chosen));
    let family = layout_family(&mut rep, &mut out, opts, &look, compiled.as_ref());

    // Videos and sounds.
    let plan = media::plan(&look.media, &files);
    rep.media_bytes = plan.bytes;
    let include = fullscreen && !plan.is_empty() && opts.media.unwrap_or(plan.bytes <= media::DEFAULT_LIMIT);
    rep.media_included = include;

    // Build the theme in a scratch folder, then move it into place.
    let tmp = themes_dir.join(format!(
        ".playnite-import-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
    ));
    let mut copied = media::Copied::default();
    let build = Build {
        source: &source,
        files: &files,
        tmp: &tmp,
        mode,
        family,
        look: &look,
        compiled: compiled.as_ref(),
        plan: &plan,
        include,
    };
    let built = (|| -> anyhow::Result<()> {
        std::fs::create_dir_all(&tmp)?;
        if let Some(set) = build.copy_fonts(&fonts)? {
            fonts.set = Some(set);
        }
        let picture_rel = build.copy_picture(&picture, &mut out)?;
        if include {
            copied = media::copy(&plan, &source, &tmp)?;
        }
        build.copy_extras(banner, &rep.source, &mut out)?;
        build.keep_options(prepared.as_ref(), &mut rep, &mut out)?;
        let inputs = fill::Inputs {
            name: name.clone(),
            author: rep.author.clone(),
            version: rep.version.clone().if_empty("1"),
            description: format!("Imported from the Playnite {} theme “{name}”.", mode.label().to_lowercase()),
            palette: mapped.palette.clone(),
            fonts: fonts.set.clone(),
            picture: picture_rel,
            main_video: copied.main_video.clone(),
            details_video: copied.details_video.clone(),
            details_translated: compiled.as_ref().is_some_and(|c| c.details_with(family)),
            sounds: copied.sounds.clone(),
            music: copied.music.clone(),
            strings: strings.clone(),
        };
        build.write_theme(&inputs, &mut out)
    })();
    if let Err(e) = built {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e);
    }

    media_lines(&mut rep, &mut out, &files, &plan, &copied, mode, include);
    not_possible_lines(&mut out, &scan, &look, &files, &picture, compiled.as_ref(), rep.family, get("ThemeApiVersion"));
    rep.groups = out.groups();
    for g in &rep.groups {
        for l in &g.lines {
            if l.ok {
                rep.carried.push(l.text.clone());
            } else {
                rep.missed.push(l.text.clone());
            }
        }
    }

    // Move into place (replacing an earlier import of the same theme). Re-applying options
    // keeps where the theme came from and how it was installed.
    let dest = themes_dir.join(&key);
    rep.key = key.clone();
    if from_kept {
        if let Some(e) = &earlier {
            rep.source = e.source.clone();
        }
        let install = earlier_dir.join(super::catalog::RECORD);
        if install.is_file() {
            let _ = std::fs::copy(&install, tmp.join(super::catalog::RECORD));
        }
    }
    if let Err(e) = std::fs::write(tmp.join(RECORD), toml::to_string_pretty(&rep).unwrap_or_default()) {
        log::warn!("playnite import record: {e}");
    }
    if let Err(e) = crate::theme::replace_dir(&tmp, &dest) {
        let _ = std::fs::remove_dir_all(&tmp);
        return Err(e.into());
    }
    rep.dir = dest;
    Ok(rep)
}

/// The theme's name when theme.yaml has none: the package's or folder's name.
fn fallback_name(src: &Path) -> String {
    src.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.eq_ignore_ascii_case("theme"))
        .or_else(|| src.parent().and_then(|p| p.file_name()).map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "Playnite theme".into())
}

/// The theme's XAML files, in the order resource dictionaries override each other:
/// Constants.xaml (colours) wins over Media.xaml over the rest.
fn xaml_files(files: &[(String, u64)]) -> Vec<&String> {
    let mut xaml: Vec<&String> = files.iter().map(|(f, _)| f).filter(|f| ext(f) == "xaml").take(MAX_XAML).collect();
    let rank = |f: &str| match base_name(f).to_ascii_lowercase().as_str() {
        "constants.xaml" => 2,
        "media.xaml" => 1,
        _ => 0,
    };
    xaml.sort_by_key(|f| (rank(f), std::cmp::Reverse(f.matches('/').count())));
    xaml
}

/// What the report says about the theme's XAML as a whole.
struct XamlScan {
    /// Control templates and animations: the theme's own screens.
    templates: usize,
    animations: usize,
    /// Files that couldn't be read or parsed.
    broken: Vec<String>,
}

fn scan_xaml(source: &Source, xaml: &[&String], source_name: &str) -> XamlScan {
    let mut scan = XamlScan { templates: 0, animations: 0, broken: vec![] };
    for f in xaml {
        let Some(text) = source.read_text(f, XAML_LIMIT) else {
            scan.broken.push((*f).clone());
            continue;
        };
        match xaml::parse(&text) {
            Ok(d) => {
                scan.templates += d.templates;
                scan.animations += d.animations;
            }
            Err(e) => {
                log::info!("playnite theme {source_name}: {f}: {e}");
                scan.broken.push((*f).clone());
            }
        }
    }
    scan
}

/// The colour a desktop window painted with a picture or a tiled texture shows: the file's
/// average at the brush's opacity, over the black under a Playnite window.
fn window_image_color(look: &look::PlayniteLook, source: &Source) -> Option<Color> {
    let look::LookBrush::Image { file: Some(f), opacity, .. } = look.desktop.window_background.as_ref()? else {
        return None;
    };
    let average = source.read(f, IMAGE_LIMIT).and_then(|b| average_color(&b))?;
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    Some(map::over(average.with_alpha(alpha), Color::BLACK))
}

/// The palette (and, where the look has no window background, the gradient it was worked
/// out from), with the report's colour line. `picture_color`: fullscreen, the background
/// picture's colour; desktop, the colour of the picture or texture the window is painted with.
fn colours(
    res: &Resources,
    look: &mut look::PlayniteLook,
    mode: Mode,
    picture_color: Option<Color>,
    out: &mut report::Builder,
) -> map::Mapped {
    let fullscreen = mode == Mode::Fullscreen;
    // The brush the theme's window / main view paints, where it is a mostly opaque colour or
    // gradient.
    let window_brush = if fullscreen { look.main.background.as_ref() } else { look.desktop.window_background.as_ref() };
    let window_color = window_brush
        .and_then(look::LookBrush::average)
        .filter(|c| c.0[3] >= 200)
        .or(picture_color.filter(|_| !fullscreen));
    let mapped = map::palette(
        res,
        mode,
        picture_color.filter(|_| fullscreen),
        window_color,
        look.desktop.play_button.as_ref().and_then(look::LookBrush::average).filter(|_| !fullscreen),
    );
    // The window background the palette was worked out from, where the look has none.
    let fallback_bg = mapped.background.clone().filter(|_| mapped.gradient).and_then(|b| match b {
        xaml::Brush::Gradient { stops, radial: false, .. } => {
            Some(look::LookBrush::Linear { stops, start: [0.5, 0.0], end: [0.5, 1.0], absolute: false })
        }
        _ => None,
    });
    if fullscreen && look.main.background.is_none() {
        look.main.background = fallback_bg;
    } else if !fullscreen && look.desktop.window_background.is_none() {
        look.desktop.window_background = fallback_bg;
    }
    if mapped.from_theme.is_empty() {
        out.ok(Area::Colours, "The theme doesn't change Playnite's default colours, so those are used");
    } else {
        let mut line = format!("From the theme: {}", mapped.from_theme.join(", "));
        if !mapped.worked_out.is_empty() {
            line.push_str(&format!(" (filled in to match: {})", mapped.worked_out.join(", ")));
        }
        out.ok(Area::Colours, line);
    }
    mapped
}

/// The theme's text fonts: its own files where it ships them, else what stands in.
struct TextFonts {
    /// The theme's own font files by role.
    picked: Option<fonts::Picked>,
    /// The `[fonts]` values (stand-ins until the theme's own files are copied).
    set: Option<fill::FontSet>,
    /// Titles in Playnite's Titillium Web Bold next to the theme's own text font.
    titillium_titles: bool,
}

impl TextFonts {
    fn pick(
        source: &Source,
        files: &[(String, u64)],
        res: &Resources,
        look: &look::PlayniteLook,
        mode: Mode,
        out: &mut report::Builder,
    ) -> TextFonts {
        let fullscreen = mode == Mode::Fullscreen;
        let (font_value, font_key) = text_font(res, mode);
        let wanted =
            font_value.as_deref().and_then(fonts::family_of_value).or_else(|| font_key.clone().map(|(f, _)| f));
        let wanted_weight = font_key.as_ref().map(|(_, s)| fonts::weight_of(s)).unwrap_or(400);
        let font_files: Vec<fonts::FontFile> = files
            .iter()
            .filter(|(f, size)| matches!(ext(f).as_str(), "ttf" | "otf" | "ttc") && *size <= FONT_LIMIT)
            .take(60)
            .filter_map(|(f, _)| {
                let data = source.read(f, FONT_LIMIT)?;
                let (family, style) = fonts::names(&data)?;
                Some(fonts::FontFile { rel: f.clone(), family, style })
            })
            .collect();
        let picked = fonts::pick(&font_files, wanted.as_deref(), wanted_weight);
        let mut set: Option<fill::FontSet> = None;
        let titillium = |weight: u16, out: &mut report::Builder| {
            if super::fonts::bundled::has_titillium() {
                out.ok(Area::Fonts, titillium_line("Font"));
            } else {
                out.no(Area::Fonts, titillium_line("Font"));
            }
            Some(fill::FontSet::titillium(weight))
        };
        match (&picked, &wanted) {
            (Some(p), _) => out.ok(
                Area::Fonts,
                match mode {
                    Mode::Desktop => format!("Font: {} (included with the theme) — for headings and titles", p.family),
                    Mode::Fullscreen => format!("Font: {} (included with the theme)", p.family),
                },
            ),
            (None, Some(w)) if is_titillium(w) => set = titillium(wanted_weight, out),
            (None, Some(w)) if WINDOWS_FONTS.contains(&w.trim().to_ascii_lowercase().as_str()) => {
                if fullscreen {
                    set = Some(fill::FontSet::system());
                }
                out.no(Area::Fonts, format!("The font “{w}” — a Windows font: Lodestar's font is used instead"));
            }
            (None, Some(w)) => {
                if fullscreen {
                    set = Some(fill::FontSet::system());
                }
                out.no(
                    Area::Fonts,
                    format!("The font “{w}” — it isn't included with the theme, so Lodestar's own font is used"),
                );
            }
            // Playnite Fullscreen's own: Titillium Web Light.
            (None, None) if fullscreen => set = titillium(300, out),
            (None, None) => {}
        }
        // Titillium titles only when Lodestar includes it (else the theme's own bold font is
        // closer than the system's).
        let titillium_titles = fullscreen
            && picked.is_some()
            && super::fonts::bundled::has_titillium()
            && look.fonts.title.as_ref().and_then(|t| t.family.as_deref()).is_some_and(is_titillium);
        if titillium_titles {
            out.ok(Area::Fonts, "Titles: Titillium Web Bold, as in Playnite");
        }
        let icon_fonts = font_files.iter().filter(|f| f.is_icons()).count();
        let uses_icofont = icon_fonts > 0 || res.get("FontIcoFont").is_some() || fullscreen;
        if uses_icofont {
            out.ok(Area::Fonts, "Icons (battery, menu, notifications): Lodestar draws its own symbols in their place");
        }
        TextFonts { picked, set, titillium_titles }
    }

    /// Compile the theme's views, measuring text in the font it will be drawn with: the
    /// theme's own, else the one Lodestar puts in its place (Titillium Web, the system font).
    fn compile(&self, source: &Source, look: &look::PlayniteLook, chosen: &options::Choices) -> compile::Compiled {
        let font = |f: &Option<fonts::FontFile>| {
            f.as_ref().and_then(|f| source.read(&f.rel, FONT_LIMIT)).map(egui::FontData::from_owned)
        };
        let stand_in = |v: Option<&String>| v.and_then(|v| super::fonts::value_data(v));
        let (regular, bold) = match (&self.picked, &self.set) {
            (Some(p), _) => (font(&p.regular), font(&p.bold).or_else(|| font(&p.semibold))),
            (None, Some(fs)) => (stand_in(fs.regular.as_ref()), stand_in(fs.bold.as_ref().or(fs.semibold.as_ref()))),
            (None, None) => (None, None),
        };
        let mut measurer = compile::FontMeasurer::new(regular, bold);
        compile::compile(source, look, chosen, &mut measurer)
    }
}

/// The report line for Playnite's Titillium Web, which Lodestar may or may not include.
fn titillium_line(what: &str) -> String {
    if super::fonts::bundled::has_titillium() {
        format!("{what}: Titillium Web — it comes with Playnite, and Lodestar includes it")
    } else {
        format!(
            "{what}: Titillium Web — it comes with Playnite; this copy of Lodestar was built without it, so the system's font stands in"
        )
    }
}

/// The layout family a fullscreen theme is laid onto (the translated views when they work,
/// else the one its game list's shape points to, unless the user chose), with its report
/// lines. Desktop themes: Classic, unused.
fn layout_family(
    rep: &mut ImportReport,
    out: &mut report::Builder,
    opts: &ImportOptions,
    look: &look::PlayniteLook,
    compiled: Option<&compile::Compiled>,
) -> Family {
    let translated_ok = compiled.is_some_and(compile::Compiled::main_ok);
    let shape_family = Family::for_shape(look.main.list.shape);
    let suggested = if translated_ok { Family::Translated } else { shape_family };
    let mut family = opts.family.unwrap_or(suggested);
    if family == Family::Translated && !translated_ok {
        family = shape_family;
    }
    if rep.mode != Mode::Fullscreen {
        return family;
    }
    rep.family = Some(family);
    rep.suggested = Some(suggested);
    rep.translatable = translated_ok;
    rep.layout = if family == Family::Translated {
        "Translated from the theme".to_string()
    } else {
        family.builtin().to_string()
    };
    let why = match look.main.list.shape {
        look::ListShape::Grid => "the theme's game list is a cover grid",
        look::ListShape::Row => "the theme shows one row of covers",
        look::ListShape::Carousel => "the theme shows its games as a carousel",
    };
    let picked_by = if opts.family.is_some_and(|f| f != suggested) { "chosen by you" } else { why };
    if family == Family::Translated {
        out.ok(Area::Layout, "Layout: translated from the theme — its own main screen and game details page");
    } else {
        out.ok(Area::Layout, format!("Layout: Lodestar's “{}” in the theme's style — {picked_by}", family.builtin()));
        if let Some(why) = compiled.and_then(|c| c.main_problem.as_deref()) {
            out.no(
                Area::Layout,
                format!("The theme's own main screen {why}, so Lodestar uses {} in its style.", family.label()),
            );
        }
    }
    if let Some(c) = compiled.filter(|c| family == Family::Translated || c.details_with(family)) {
        for (a, ok, line) in compile::report_lines(c, family == Family::Translated) {
            if ok {
                out.ok(a, line);
            } else {
                out.no(a, line);
            }
        }
    }
    family
}

/// Writing the imported theme into its scratch folder.
struct Build<'a> {
    source: &'a Source,
    files: &'a [(String, u64)],
    tmp: &'a Path,
    mode: Mode,
    family: Family,
    look: &'a look::PlayniteLook,
    compiled: Option<&'a compile::Compiled>,
    plan: &'a media::Plan,
    /// The theme's videos and sounds are copied.
    include: bool,
}

impl Build<'_> {
    /// Copy the theme's own font files into `fonts/`; the `[fonts]` values for them (None:
    /// the theme ships none, or its regular one couldn't be copied).
    fn copy_fonts(&self, text_fonts: &TextFonts) -> anyhow::Result<Option<fill::FontSet>> {
        let Some(p) = &text_fonts.picked else { return Ok(None) };
        std::fs::create_dir_all(self.tmp.join("fonts"))?;
        let copy = |f: &Option<fonts::FontFile>| -> Option<String> {
            let f = f.as_ref()?;
            let rel = format!("fonts/{}", base_name(&f.rel));
            if !self.tmp.join(&rel).exists() {
                std::fs::write(self.tmp.join(&rel), self.source.read(&f.rel, FONT_LIMIT)?).ok()?;
            }
            Some(rel)
        };
        let regular = copy(&p.regular);
        if regular.is_none() {
            return Ok(None);
        }
        Ok(Some(fill::FontSet {
            regular,
            semibold: copy(&p.semibold),
            bold: copy(&p.bold),
            title: text_fonts.titillium_titles.then(|| super::fonts::bundled::titillium_for_weight(700).to_string()),
        }))
    }

    /// Copy the background picture into `images/`; its path in the new theme.
    fn copy_picture(
        &self,
        picture: &Option<Result<String, String>>,
        out: &mut report::Builder,
    ) -> anyhow::Result<Option<String>> {
        match picture {
            Some(Ok(rel)) => match self.source.read(rel, IMAGE_LIMIT) {
                Some(bytes) => {
                    std::fs::create_dir_all(self.tmp.join("images"))?;
                    let dest = format!("images/{}", base_name(rel));
                    std::fs::write(self.tmp.join(&dest), bytes)?;
                    out.ok(Area::Background, format!("Background picture: {rel}"));
                    return Ok(Some(dest));
                }
                None => {
                    out.no(Area::Background, format!("The background picture {rel} (couldn't be read or is too big)"))
                }
            },
            Some(Err(what)) => out.no(Area::Background, what.clone()),
            None => {}
        }
        Ok(None)
    }

    /// Banners and other per-game pictures of the theme.
    fn copy_extras(
        &self,
        banner: Option<(bool, extras::Placement)>,
        source_name: &str,
        out: &mut report::Builder,
    ) -> anyhow::Result<()> {
        let mut extras = extras::Extras::read(self.source, self.files, banner);
        let pictures = extras.write(self.source, self.tmp)?;
        if let Some(b) = extras.banners.as_ref().filter(|_| self.tmp.join(extras::BANNERS).is_file()) {
            let n = b.sources.len() + b.platforms.len();
            let what = format!("{n} store and console banner{}", if n == 1 { "" } else { "s" });
            match (self.mode, b.shown) {
                (Mode::Desktop, true) => out.ok(Area::Covers, format!("Banners: the theme's {what} on the covers")),
                (Mode::Desktop, false) => {
                    out.ok(Area::Covers, format!("Banners: the theme's {what} (switched off in the theme's options)"))
                }
                (Mode::Fullscreen, _) => {}
            }
        } else if pictures > 0 {
            log::info!("playnite theme {source_name}: {pictures} per-game pictures");
        }
        Ok(())
    }

    /// Theme options, and the theme's own files kept to apply them again later.
    fn keep_options(
        &self,
        prepared: Option<&(options::OptionsSpec, BTreeMap<String, String>)>,
        rep: &mut ImportReport,
        out: &mut report::Builder,
    ) -> anyhow::Result<()> {
        let Some((spec, values)) = prepared else { return Ok(()) };
        if spec.effective().next().is_none() {
            match spec.others() {
                0 => {}
                1 => out.no(Area::Options, "The theme's one option only changes parts Lodestar draws its own way"),
                n => out
                    .no(Area::Options, format!("The theme's {n} options only change parts Lodestar draws its own way")),
            }
            return Ok(());
        }
        spec.write(self.tmp)?;
        let plan = self.plan;
        let mut linked = vec![];
        for v in &plan.videos {
            linked.push(v.file.clone());
        }
        linked.extend(plan.music.iter().map(|(f, _)| f.clone()));
        linked.extend(plan.sounds.iter().map(|(_, f, _)| f.clone()));
        let media_links: Vec<(String, String)> = if self.include {
            linked
                .into_iter()
                .map(|f| {
                    let d = format!("media/{}", media::dest_name(&f));
                    (f, d)
                })
                .collect()
        } else {
            vec![]
        };
        options::snapshot(self.source, self.files, self.tmp, &media_links)?;
        rep.options = spec.effective().count();
        rep.option_values = values.clone();
        let place = match self.mode {
            Mode::Desktop => "Settings → Appearance → Theme options",
            Mode::Fullscreen => {
                "controller mode: Settings → Visuals → Theme options (or desktop Settings → Appearance)"
            }
        };
        let (player, own): (Vec<_>, Vec<_>) = spec.effective().partition(|o| o.is_setting());
        match own.len() {
            0 => {}
            1 => out.ok(Area::Options, format!("One of the theme's options can be changed in {place}")),
            n => out.ok(Area::Options, format!("{n} of the theme's options can be changed in {place}")),
        }
        for o in player {
            let name = settings::player_setting(&o.key).map_or(o.label.as_str(), |s| s.playnite_name);
            let place = if own.is_empty() { place } else { "the same place" };
            out.ok(
                Area::Options,
                format!("Playnite's “{name}” setting, which the theme reads, can be changed in {place}"),
            );
        }
        if !values.is_empty() {
            out.ok(
                Area::Options,
                format!("{} chosen option{} applied", values.len(), if values.len() == 1 { "" } else { "s" }),
            );
        }
        match spec.others() {
            0 => {}
            1 => out.no(Area::Options, "One more option only changes parts Lodestar draws its own way"),
            n => out.no(Area::Options, format!("{n} more options only change parts Lodestar draws its own way")),
        }
        Ok(())
    }

    /// Copy the theme pictures the translated views show.
    fn copy_view_images(&self, c: &compile::Compiled) -> anyhow::Result<()> {
        for (from, to) in &c.images {
            if let Some(bytes) = self.source.read(from, IMAGE_LIMIT) {
                let dest = self.tmp.join(to);
                if let Some(d) = dest.parent() {
                    std::fs::create_dir_all(d)?;
                }
                std::fs::write(dest, bytes)?;
            }
        }
        Ok(())
    }

    /// Lay the look onto its layout (or the translated views) and write theme.toml.
    fn write_theme(&self, inputs: &fill::Inputs, out: &mut report::Builder) -> anyhow::Result<()> {
        let mut notes = fill::Notes::default();
        let value = if self.mode == Mode::Fullscreen
            && self.family == Family::Translated
            && let Some(c) = self.compiled
        {
            let mut v = fill::controller(Family::Classic, self.look, inputs, &mut notes)?;
            // Top Bar's own placement notes don't apply to the translated screens.
            notes.lines.retain(|(a, _, _)| matches!(a, Area::Covers | Area::Background | Area::Fonts));
            self.copy_view_images(c)?;
            compile::apply(&mut v, c, self.look, copied_videos(inputs))?;
            v
        } else if self.mode == Mode::Fullscreen {
            let mut v = fill::controller(self.family, self.look, inputs, &mut notes)?;
            if let Some(c) = self.compiled.filter(|c| c.details_with(self.family)) {
                self.copy_view_images(c)?;
                compile::apply_details(&mut v, c, self.look, copied_videos(inputs))?;
            }
            v
        } else {
            fill::desktop(self.look, inputs, &mut notes)?
        };
        out.extend(notes.lines);
        let text = toml::to_string_pretty(&value)?;
        // It must load as a theme (fields of a newer theme model are kept as written).
        ThemeFile::parse(&text)?;
        std::fs::write(self.tmp.join("theme.toml"), text)?;
        Ok(())
    }
}

/// The copied main screen and details page videos.
fn copied_videos(inputs: &fill::Inputs) -> [Option<&str>; 2] {
    [inputs.main_video.as_ref(), inputs.details_video.as_ref()].map(|v| v.map(|(p, _)| p.as_str()))
}

/// The report's lines on videos and sounds.
fn media_lines(
    rep: &mut ImportReport,
    out: &mut report::Builder,
    files: &[(String, u64)],
    plan: &media::Plan,
    copied: &media::Copied,
    mode: Mode,
    include: bool,
) {
    let mb = report::size_text(plan.bytes);
    let n_videos = plan.videos.len();
    let n_sounds = plan.sounds.len() + plan.music.len();
    let what = match (n_videos, n_sounds) {
        (0, _) => "sounds".to_string(),
        (_, 0) => {
            if n_videos == 1 {
                "background video".into()
            } else {
                "background videos".into()
            }
        }
        _ => "videos and sounds".to_string(),
    };
    if mode != Mode::Fullscreen {
        let count = |exts: &[&str]| files.iter().filter(|(f, _)| exts.contains(&ext(f).as_str())).count();
        let (v, a) = (count(&VIDEO_EXTS), count(&SOUND_EXTS));
        if v + a > 0 {
            out.no(Area::Media, "Videos and sounds — desktop mode doesn't play them (controller themes do)");
        }
    } else if plan.is_empty() {
        if !plan.left_out.is_empty() {
            out.no(
                Area::Media,
                format!(
                    "{} video{} the theme's own screens or add-ons play (Lodestar shows the background instead)",
                    plan.left_out.len(),
                    if plan.left_out.len() == 1 { "" } else { "s" }
                ),
            );
        }
    } else if include {
        let failed = copied.failed.len();
        use super::catalog::install::media_line;
        if copied.main_video.is_some() || copied.details_video.is_some() {
            out.ok(Area::Media, media_line("The theme's background video", false, true));
        }
        let audio = match (!copied.sounds.is_empty(), !copied.music.is_empty()) {
            (true, true) => Some(("The theme's navigation sounds and music", true)),
            (true, false) => Some(("The theme's navigation sounds", true)),
            (false, true) => Some(("The theme's background music", false)),
            (false, false) => None,
        };
        if let Some((a, plural)) = audio {
            out.ok(Area::Media, media_line(a, plural, false));
        }
        if failed > 0 {
            out.no(Area::Media, format!("{failed} file{} couldn't be read", if failed == 1 { "" } else { "s" }));
        }
        if !plan.left_out.is_empty() {
            out.no(
                Area::Media,
                format!(
                    "{} other video{} the theme's own screens or add-ons play",
                    plan.left_out.len(),
                    if plan.left_out.len() == 1 { "" } else { "s" }
                ),
            );
        }
        rep.needs_video =
            (copied.main_video.is_some() || copied.details_video.is_some()) && !crate::media::videos_available();
        rep.needs_audio = (!copied.sounds.is_empty() || !copied.music.is_empty()) && !media::sound_player_found();
    } else {
        out.no(
            Area::Media,
            format!("Its {what} ({mb}) weren't copied — tick “Include its videos and sounds” to add them"),
        );
    }
}

/// The report's lines on what can't come along.
#[allow(clippy::too_many_arguments)] // one report section drawing on most of the import
fn not_possible_lines(
    out: &mut report::Builder,
    scan: &XamlScan,
    look: &look::PlayniteLook,
    files: &[(String, u64)],
    picture: &Option<Result<String, String>>,
    compiled: Option<&compile::Compiled>,
    family: Option<Family>,
    api_version: Option<String>,
) {
    if scan.templates + scan.animations > 0 && family != Some(Family::Translated) {
        out.no(
            Area::NotPossible,
            "The theme's own screens, animations and custom controls — Playnite themes are Windows (WPF) programs, \
             so Lodestar lays the theme's look onto its own layout",
        );
    }
    if let Some(l) = report::addons_line(&look.plugins.names) {
        out.no(Area::NotPossible, l);
    }
    let count = |exts: &[&str]| files.iter().filter(|(f, _)| exts.contains(&ext(f).as_str())).count();
    let used_picture = matches!(picture, Some(Ok(_)));
    let view_pictures = match (compiled, family) {
        (Some(c), Some(Family::Translated)) => c.images.len(),
        _ => 0,
    };
    let other_pictures = count(&IMAGE_EXTS).saturating_sub(used_picture as usize + view_pictures);
    if other_pictures > 0 {
        out.no(
            Area::NotPossible,
            format!(
                "{other_pictures} other picture{} (icons, logos, overlays used by the theme's own screens)",
                if other_pictures == 1 { "" } else { "s" }
            ),
        );
    }
    let broken = &scan.broken;
    if !broken.is_empty() {
        out.no(
            Area::NotPossible,
            format!(
                "{} XAML file{} couldn't be read: {}",
                broken.len(),
                if broken.len() == 1 { "" } else { "s" },
                broken.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
            ),
        );
    }
    if let Some(v) = api_version
        && api_newer(&v)
    {
        out.no(
            Area::NotPossible,
            format!("It's made for a newer Playnite (theme version {v}); parts of it may come out differently"),
        );
    }
}

/// Average colour of a picture (decoded small; None: not a picture we can read).
fn average_color(bytes: &[u8]) -> Option<Color> {
    let mut r = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().ok()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(256 * 1024 * 1024);
    r.limits(limits);
    let img = r.decode().ok()?.thumbnail(32, 32).to_rgb8();
    let n = (img.width() * img.height()).max(1) as u64;
    let mut sum = [0u64; 3];
    for p in img.pixels() {
        for i in 0..3 {
            sum[i] += p.0[i] as u64;
        }
    }
    Some(Color([(sum[0] / n) as u8, (sum[1] / n) as u8, (sum[2] / n) as u8, 255]))
}

trait IfEmpty {
    fn if_empty(self, d: &str) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, d: &str) -> String {
        if self.trim().is_empty() { d.to_string() } else { self }
    }
}

/// The text font: (a FontFamily value, or a Playnite font key's family + style).
fn text_font(res: &Resources, mode: Mode) -> (Option<String>, Option<(String, String)>) {
    let from_val = |v: &str| -> (Option<String>, Option<(String, String)>) {
        match Val::parse(v) {
            Val::Lit(s) => (Some(s), None),
            Val::Ref(k) => match res.font(&k) {
                Some(s) => (Some(s), None),
                None => (None, fonts::family_of_key(&k)),
            },
        }
    };
    let styles: &[&str] = match mode {
        Mode::Desktop => &["BaseTextBlockStyle", "TextBlockBaseStyle"],
        Mode::Fullscreen => &["TextBlockBaseStyle", "BaseTextBlockStyle"],
    };
    // What the theme itself sets wins; Playnite's defaults last.
    for pass in [true, false] {
        if res.theme_defines("FontFamily") == pass
            && let Some(f) = res.font("FontFamily")
        {
            return (Some(f), None);
        }
        for s in styles {
            if res.theme_defines(s) == pass
                && let Some(v) = res.style_setter(s, "FontFamily")
            {
                return from_val(&v);
            }
        }
    }
    (None, None)
}

/// Pictures smaller than this are tiles (noise, grain, patterns WPF repeats over a panel), not
/// a picture to stretch behind the window.
const TEXTURE_BYTES: u64 = 16 * 1024;

/// The desktop window's picture as the theme's MainWindow / Library draw it (extract.rs), if
/// it is a real picture rather than a tiled texture.
fn desktop_picture(look: &look::PlayniteLook, files: &[(String, u64)]) -> Option<Result<String, String>> {
    let look::LookBrush::Image { file: Some(f), .. } = look.desktop.background_picture.as_ref()? else {
        return None;
    };
    let (path, size) = files.iter().find(|(p, _)| p == f)?;
    (*size >= TEXTURE_BYTES && IMAGE_EXTS.contains(&ext(path).as_str())).then(|| Ok(path.clone()))
}

/// The theme's background picture: Ok(theme file) or Err(what to report).
fn background_picture(res: &Resources, files: &[(String, u64)], mode: Mode) -> Option<Result<String, String>> {
    let named: &[&str] = match mode {
        Mode::Fullscreen => &["MainBackgourndBrush", "MainBackgroundBrush", "BackgroundImage", "MainBackgroundImage"],
        Mode::Desktop => &["WindowBackgourndBrush", "WindowBackgroundBrush", "BackgroundImage"],
    };
    let mut keys: Vec<&str> = named.iter().copied().filter(|k| res.theme_defines(k)).collect();
    keys.extend(res.theme.keys().map(String::as_str).filter(|k| {
        let l = k.to_ascii_lowercase();
        (l.contains("background") || l.contains("wallpaper")) && !l.contains("cover") && !named.contains(k)
    }));
    // Add-on icons, options previews and the like are never the background.
    let not_background = |f: &str| {
        let path = f.to_ascii_lowercase();
        ["add-on", "addon", "option", "icon", "logo", "preview", "button"].iter().any(|w| path.contains(w))
    };
    for k in keys {
        if let Some(Res::Image(src)) = res.theme.get(k)
            && !src.trim().is_empty()
        {
            let ok = resolve_file(files, src).filter(|f| IMAGE_EXTS.contains(&ext(f).as_str()));
            match ok {
                Some(f) if not_background(&f) && !named.contains(&k) => continue,
                Some(f) => return Some(Ok(f)),
                None if named.contains(&k) => {
                    return Some(Err(format!("The background picture “{src}” (not found in the theme)")));
                }
                None => continue,
            }
        }
    }
    // No resource: a picture called like a background in the theme's image folder (not an
    // add-on's icon or an options preview: big enough to be a background).
    files
        .iter()
        .filter(|(f, size)| IMAGE_EXTS.contains(&ext(f).as_str()) && *size >= 40 * 1024)
        .filter(|(f, _)| {
            let n = base_name(f).to_ascii_lowercase();
            (n.contains("background") || n.contains("wallpaper")) && !n.contains("cover") && !not_background(f)
        })
        .max_by_key(|(_, size)| *size)
        .map(|(f, _)| Ok(f.clone()))
}

/// A free key: `key`, or a previous import of the same theme mode (replaced), else
/// `key-playnite`, `key-playnite-2` …. Never replaces a theme we didn't create, nor shadows a
/// built-in's name.
fn pick_key(themes_dir: &Path, key: &str, mode: Mode) -> anyhow::Result<String> {
    let ours = |dir: &Path| -> bool {
        let Ok(meta) = std::fs::symlink_metadata(dir) else {
            return true;
        };
        if !meta.is_dir() {
            return false;
        }
        let origin_ok = std::fs::read_to_string(dir.join("theme.toml"))
            .ok()
            .and_then(|t| ThemeFile::parse(&t).ok())
            .is_some_and(|f| f.theme.origin == "playnite");
        let mode_ok = std::fs::read_to_string(dir.join(RECORD))
            .ok()
            .and_then(|t| toml::from_str::<ImportReport>(&t).ok())
            .is_some_and(|r| r.mode == mode);
        origin_ok && mode_ok
    };
    let builtin_names: Vec<String> = super::builtin::all().into_iter().map(|(k, _)| super::sanitize_key(k)).collect();
    let mut candidates = vec![];
    if !builtin_names.contains(&key.to_string()) {
        candidates.push(key.to_string());
    }
    let suffix = match mode {
        Mode::Desktop => "playnite",
        Mode::Fullscreen => "playnite-fullscreen",
    };
    candidates.push(format!("{key}-{suffix}"));
    candidates.extend((2..100).map(|n| format!("{key}-{suffix}-{n}")));
    candidates
        .into_iter()
        .find(|c| ours(&themes_dir.join(c)))
        .ok_or_else(|| anyhow::anyhow!("no free theme name for {key} in {}", themes_dir.display()))
}

/// A Playnite theme found on this machine.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// Theme folder or .pthm package.
    pub path: PathBuf,
    pub name: String,
    pub mode: Mode,
}

/// Name and mode from a theme.yaml.
fn found_from_manifest(path: PathBuf, text: &str) -> Found {
    let m = parse_manifest(text);
    let get = |k: &str| m.iter().find(|(a, _)| a.eq_ignore_ascii_case(k)).map(|(_, v)| v.clone());
    let fallback = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Found {
        name: get("Name").filter(|n| !n.is_empty()).unwrap_or(fallback),
        mode: if get("Mode").is_some_and(|v| v.eq_ignore_ascii_case("fullscreen")) {
            Mode::Fullscreen
        } else {
            Mode::Desktop
        },
        path,
    }
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<PathBuf> =
        rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.path()).collect();
    v.sort();
    v
}

/// Playnite themes on this machine: downloaded packages (`~/Downloads/*.pthm`) and themes
/// installed in a Windows Playnite inside a Wine / Proton / Bottles prefix
/// (`drive_c/users/*/AppData/{Roaming,Local}/Playnite/Themes/{Desktop,Fullscreen}/*`).
/// Read-only, a few directory listings; Playnite's built-in themes are left out.
pub fn discover(home: &Path) -> Vec<Found> {
    let mut out: Vec<Found> = vec![];
    for dl in [home.join("Downloads"), home.join("downloads")] {
        let Ok(rd) = std::fs::read_dir(&dl) else { continue };
        let mut pkgs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("pthm")) && p.is_file())
            .collect();
        pkgs.sort();
        for p in pkgs.into_iter().take(100) {
            let text = Source::open(&p).ok().and_then(|s| s.read_text("theme.yaml", 64 * 1024));
            if let Some(t) = text {
                out.push(found_from_manifest(p, &t));
            }
        }
    }
    let mut prefixes = vec![home.join(".wine")];
    for d in [home.join(".local/share/Steam/steamapps/compatdata"), home.join(".steam/steam/steamapps/compatdata")] {
        prefixes.extend(subdirs(&d).into_iter().map(|p| p.join("pfx")));
    }
    for d in [
        home.join("Games"),
        home.join("Faugus"),
        home.join(".local/share/bottles/bottles"),
        home.join(".var/app/com.usebottles.bottles/data/bottles/bottles"),
    ] {
        prefixes.extend(subdirs(&d));
    }
    for pfx in prefixes {
        for user in subdirs(&pfx.join("drive_c/users")) {
            for app in ["AppData/Roaming/Playnite", "AppData/Local/Playnite"] {
                for mode in ["Desktop", "Fullscreen"] {
                    for theme in subdirs(&user.join(app).join("Themes").join(mode)) {
                        let Ok(t) = std::fs::read_to_string(theme.join("theme.yaml")) else {
                            continue;
                        };
                        if t.contains("Playnite_builtin_") {
                            continue;
                        }
                        let mut f = found_from_manifest(theme, t.trim_start_matches('\u{feff}'));
                        if mode == "Fullscreen" {
                            f.mode = Mode::Fullscreen;
                        }
                        out.push(f);
                    }
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|f| seen.insert(f.path.clone()));
    out
}
