//! Theme options: a Playnite theme's `thememodifier.yaml` (the ThemeModifier add-on's list of
//! Constants a user may change) as Lodestar settings, and for fullscreen themes the Playnite
//! settings that are the player's choice rather than the theme's (`settings::PLAYER_SETTINGS`,
//! "Show game titles"), at Playnite's default until the player changes them. Each
//! thememodifier option is typed from the constant it names in the resolved theme:
//! `sys:Boolean` / `Visibility` → a switch, an enum (HorizontalAlignment, Stretch …) → a
//! choice, `Key(min,max)` → a slider, `Color` / `SolidColorBrush` → a colour, `sys:String` →
//! a text.
//!
//! Applying options re-runs the import with the chosen values laid over the theme's own
//! constants (one extra resource dictionary after Constants.xaml, as Playnite would load the
//! add-on's changes), from a copy of the theme's own files kept in the imported theme
//! (`playnite-source/`, its XAML, fonts and pictures; videos and sounds are hard links to the
//! copies the theme already has). An import takes a few tens of milliseconds, so a change feels
//! immediate. Only options that change what Lodestar draws are offered (each one is tried at
//! import: the look and the colours are compared with the option flipped); the others are
//! counted.
//!
//! CONTRACT:
//!   pub const SNAPSHOT: &str  — the kept source folder inside an imported theme
//!   pub const FILE: &str      — `playnite-options.toml` (the spec + the chosen values)
//!   pub struct OptionsSpec { options: Vec<ThemeOption>, values: BTreeMap<key, value>, other }
//!   /// The options of an imported theme (None: it has none Lodestar can use).
//!   pub fn read(theme_dir: &Path) -> Option<OptionsSpec>
//!   pub(crate) fn spec_of(tree, yaml) -> OptionsSpec
//!   pub(crate) fn overrides_xaml(spec, values) -> Option<String>
//!   /// The chosen values as the importer reads them (constants dictionary + Playnite settings).
//!   pub(crate) struct Choices; pub(crate) fn choices(spec, values) -> Choices

use super::Mode;
use super::settings::{self, PlayerSettings, SVal};
use super::source::Source;
use super::tree::{Texts, Tree, Value, tag};
use crate::theme::Color;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// The theme's own files kept inside the imported theme (for re-applying options).
pub const SNAPSHOT: &str = "playnite-source";
/// The options and the values chosen.
pub const FILE: &str = "playnite-options.toml";
/// Most options read from one thememodifier.yaml.
const MAX_OPTIONS: usize = 200;
/// The `element` of an option that stands for a Playnite setting, not a theme constant.
const SETTING: &str = "Setting";

/// What kind of control an option is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Kind {
    /// On / off; `on` / `off` are the XAML values ("True" / "False", "Visible" / "Collapsed").
    Switch { on: String, off: String },
    /// A number between `min` and `max` (`step` 1 for whole numbers).
    Range { min: f64, max: f64, step: f64 },
    /// One of a few named values.
    Choice { choices: Vec<String> },
    /// A colour (#AARRGGBB).
    Color,
    /// Free text (button labels).
    Text,
}

/// One theme option.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeOption {
    /// The Constants key it changes.
    pub key: String,
    pub label: String,
    /// The heading it is listed under in thememodifier.yaml.
    #[serde(default)]
    pub section: String,
    pub kind: Kind,
    /// The theme's own value (XAML text).
    pub default: String,
    /// The XAML element written for it ("sys:Boolean", "Visibility", "SolidColorBrush" …).
    pub element: String,
    /// It changes something Lodestar draws.
    #[serde(default)]
    pub effect: bool,
}

impl ThemeOption {
    /// The value in effect (`values` over the theme's own).
    pub fn value<'a>(&'a self, values: &'a BTreeMap<String, String>) -> &'a str {
        values.get(&self.key).map(String::as_str).unwrap_or(&self.default)
    }

    /// It stands for a Playnite setting (its key is the setting's path), not a theme constant.
    pub fn is_setting(&self) -> bool {
        self.element == SETTING
    }

    pub fn is_on(&self, values: &BTreeMap<String, String>) -> bool {
        match &self.kind {
            Kind::Switch { on, .. } => self.value(values).eq_ignore_ascii_case(on),
            _ => false,
        }
    }

    pub fn number(&self, values: &BTreeMap<String, String>) -> f64 {
        let v = self
            .value(values)
            .split(',')
            .next()
            .unwrap_or("")
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .unwrap_or(0.0);
        match &self.kind {
            Kind::Range { min, max, .. } => in_range(v, *min, *max),
            _ => v,
        }
    }

    /// The value one step on (`dir` +1 / −1): a switch flips, a choice moves, a slider moves
    /// a tenth of its range (whole steps for whole numbers).
    pub fn stepped(&self, values: &BTreeMap<String, String>, dir: i8) -> String {
        let cur = self.value(values);
        match &self.kind {
            Kind::Switch { on, off } => {
                if cur.eq_ignore_ascii_case(on) {
                    off.clone()
                } else {
                    on.clone()
                }
            }
            Kind::Choice { choices } => {
                let n = choices.len().max(1) as i64;
                let i = choices.iter().position(|c| c.eq_ignore_ascii_case(cur)).unwrap_or(0) as i64;
                choices.get((i + dir as i64).rem_euclid(n) as usize).cloned().unwrap_or_default()
            }
            Kind::Range { min, max, step } => {
                let span = (max - min).abs();
                let mut d = if *step >= 1.0 { (span / 10.0).round().max(*step) } else { span / 10.0 };
                if d <= 0.0 {
                    d = 1.0;
                }
                if !d.is_finite() {
                    d = 1.0;
                }
                let v = in_range(self.number(values) + d * dir.signum() as f64, *min, *max);
                number_text(v, *step)
            }
            Kind::Color | Kind::Text => cur.to_string(),
        }
    }

    /// The value in words ("On", "Center", "0.45").
    pub fn value_text(&self, values: &BTreeMap<String, String>) -> String {
        match &self.kind {
            Kind::Switch { .. } => if self.is_on(values) { "On" } else { "Off" }.into(),
            Kind::Range { step, .. } => number_text(self.number(values), *step),
            _ => self.value(values).to_string(),
        }
    }
}

/// `v` kept between two bounds given in any order. Bounds that aren't numbers (a theme can
/// write `nan` or `inf`; an older import may have stored them) don't limit it: `f64::clamp`
/// would panic on them.
fn in_range(v: f64, a: f64, b: f64) -> f64 {
    let (lo, hi) = (a.min(b), a.max(b));
    if lo.is_finite() && hi.is_finite() { v.clamp(lo, hi) } else { v }
}

/// "12" for whole-number steps, else up to 2 decimals ("0.45").
pub fn number_text(v: f64, step: f64) -> String {
    if step >= 1.0 {
        return format!("{}", v.round() as i64);
    }
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.to_string() }
}

/// A theme's options and the values chosen for it (stored in [`FILE`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OptionsSpec {
    /// Every option thememodifier.yaml lists that Lodestar could type.
    pub options: Vec<ThemeOption>,
    /// Chosen values that differ from the theme's (key → XAML text).
    pub values: BTreeMap<String, String>,
}

impl OptionsSpec {
    pub fn is_empty(&self) -> bool {
        self.options.is_empty()
    }

    /// The options that change what Lodestar draws.
    pub fn effective(&self) -> impl Iterator<Item = &ThemeOption> {
        self.options.iter().filter(|o| o.effect)
    }

    /// Options the theme has that only change parts Lodestar draws its own way. A Playnite
    /// setting that changes nothing isn't one: it's the player's, not the theme's.
    pub fn others(&self) -> usize {
        self.options.iter().filter(|o| !o.effect && !o.is_setting()).count()
    }

    /// Only values that are real options and differ from the theme's own.
    pub fn clean(&self, values: &BTreeMap<String, String>) -> BTreeMap<String, String> {
        values
            .iter()
            .filter_map(|(k, v)| {
                let o = self.options.iter().find(|o| &o.key == k)?;
                let v = v.trim();
                (!v.eq_ignore_ascii_case(o.default.trim())).then(|| (k.clone(), v.to_string()))
            })
            .collect()
    }

    pub fn write(&self, theme_dir: &Path) -> anyhow::Result<()> {
        std::fs::write(theme_dir.join(FILE), toml::to_string_pretty(self)?)?;
        Ok(())
    }
}

/// The options of an imported theme (None: it has none that change what Lodestar draws, or it
/// isn't a Playnite import).
pub fn read(theme_dir: &Path) -> Option<OptionsSpec> {
    let text = std::fs::read_to_string(theme_dir.join(FILE)).ok()?;
    let spec: OptionsSpec = toml::from_str(&text).ok()?;
    let any = spec.options.iter().any(|o| o.effect);
    any.then_some(spec)
}

/// Enum types → their values (what a Choice offers).
fn enum_choices(element: &str) -> Option<&'static [&'static str]> {
    Some(match element {
        "HorizontalAlignment" => &["Left", "Center", "Right", "Stretch"],
        "VerticalAlignment" => &["Top", "Center", "Bottom", "Stretch"],
        "Stretch" => &["None", "Fill", "Uniform", "UniformToFill"],
        "Orientation" => &["Horizontal", "Vertical"],
        "TextAlignment" => &["Left", "Center", "Right", "Justify"],
        "Dock" => &["Left", "Top", "Right", "Bottom"],
        "TextWrapping" => &["NoWrap", "Wrap", "WrapWithOverflow"],
        "TextTrimming" => &["None", "CharacterEllipsis", "WordEllipsis"],
        "FontWeight" => &["Thin", "Light", "Normal", "Medium", "SemiBold", "Bold", "Black"],
        "ScrollBarVisibility" => &["Disabled", "Auto", "Hidden", "Visible"],
        _ => return None,
    })
}

fn color_text(c: Color) -> String {
    let [r, g, b, a] = c.0;
    format!("#{a:02X}{r:02X}{g:02X}{b:02X}")
}

/// `#AARRGGBB` / `#RRGGBB` → Color.
pub fn parse_color(s: &str) -> Option<Color> {
    super::xaml::parse_color(s.trim())
}

/// The typed options of `thememodifier.yaml` against the resolved theme: `Constants:` then
/// `- "Section"` headings and `- Key: 'Label'` / `- Key(min,max): 'Label'` entries (other
/// top-level lists, e.g. `Styles:`, aren't options Lodestar can apply).
pub(crate) fn spec_of(tree: &Tree, yaml: &str) -> OptionsSpec {
    let mut out = OptionsSpec::default();
    let mut section = String::new();
    let mut in_constants = false;
    for line in yaml.lines() {
        let l = line.trim_end();
        if l.trim().is_empty() || l.trim_start().starts_with('#') {
            continue;
        }
        if !l.starts_with([' ', '\t', '-']) {
            in_constants = l.trim().trim_start_matches('\u{feff}').starts_with("Constants");
            continue;
        }
        if !in_constants {
            continue;
        }
        let Some(item) = l.trim().strip_prefix('-') else { continue };
        let item = item.trim();
        let unq = |s: &str| s.trim().trim_matches(|c| c == '"' || c == '\'').trim().to_string();
        let Some((head, label)) = item.split_once(':').filter(|(h, _)| !h.trim().starts_with(['"', '\''])) else {
            section = unq(item);
            continue;
        };
        let head = head.trim();
        let (key, range) = match head.split_once('(') {
            Some((k, r)) => {
                // Only real numbers make a range (`nan` / `inf` parse as floats too).
                let nums: Vec<f64> = r
                    .trim_end_matches(')')
                    .split(',')
                    .filter_map(|v| v.trim().parse::<f64>().ok())
                    .filter(|v| v.is_finite())
                    .collect();
                (k.trim(), (nums.len() == 2).then(|| (nums[0], nums[1])))
            }
            None => (head, None),
        };
        if key.is_empty() || out.options.len() >= MAX_OPTIONS || out.options.iter().any(|o| o.key == key) {
            continue;
        }
        if let Some(o) = typed(tree, key, unq(label), section.clone(), range) {
            out.options.push(o);
        }
    }
    out
}

/// One option typed from the constant it names (None: the theme doesn't define it, or its type
/// isn't one Lodestar can offer — gradients, styles …).
fn typed(tree: &Tree, key: &str, label: String, section: String, range: Option<(f64, f64)>) -> Option<ThemeOption> {
    let node = tree.resource(key)?;
    let element = tag(node).to_string();
    let val = tree.eval_node(node, 0);
    let text: String = node.children().filter_map(|c| c.text()).collect::<String>().trim().to_string();
    let label = clean_label(&label);
    let label = if label.is_empty() { key.to_string() } else { label };
    let mk = |kind: Kind, default: String, element: &str| ThemeOption {
        key: key.to_string(),
        label: label.clone(),
        section: section.clone(),
        kind,
        default,
        element: element.to_string(),
        effect: false,
    };
    if let Some((min, max)) = range {
        let whole = matches!(element.as_str(), "Int32" | "Int64" | "Int16")
            || (min.fract() == 0.0 && max.fract() == 0.0 && (max - min).abs() > 2.0);
        let default = val.as_ref().and_then(|v| v.num().or_else(|| v.max_side().map(f64::from))).unwrap_or(min);
        let step = if whole { 1.0 } else { 0.01 };
        let el = match element.as_str() {
            "Thickness" | "CornerRadius" | "GridLength" => element.clone(),
            "Int32" | "Int64" | "Int16" => format!("sys:{element}"),
            _ => "sys:Double".into(),
        };
        return Some(mk(Kind::Range { min, max, step }, number_text(default, step), &el));
    }
    match element.as_str() {
        "Boolean" => {
            let b = val.as_ref().and_then(Value::boolean).or_else(|| super::convert::boolean(&text))?;
            Some(mk(
                Kind::Switch { on: "True".into(), off: "False".into() },
                if b { "True" } else { "False" }.into(),
                "sys:Boolean",
            ))
        }
        "Visibility" => {
            let v = if text.is_empty() { "Visible".to_string() } else { text.clone() };
            let off = if v.eq_ignore_ascii_case("Hidden") { "Hidden" } else { "Collapsed" };
            Some(mk(Kind::Switch { on: "Visible".into(), off: off.into() }, v, "Visibility"))
        }
        "Double" | "Single" | "Int32" | "Int64" => None,
        "String" => {
            // Text constants: button labels and the like (LOC strings).
            Some(mk(Kind::Text, text, "sys:String"))
        }
        "Color" => {
            let c = val.as_ref().and_then(Value::color).or_else(|| parse_color(&text))?;
            Some(mk(Kind::Color, color_text(c), "Color"))
        }
        "SolidColorBrush" => {
            let c = val.as_ref().and_then(Value::color)?;
            Some(mk(Kind::Color, color_text(c), "SolidColorBrush"))
        }
        e => {
            let choices = enum_choices(e)?;
            let cur = choices.iter().find(|c| c.eq_ignore_ascii_case(&text)).copied().unwrap_or(choices[0]);
            Some(mk(Kind::Choice { choices: choices.iter().map(|c| c.to_string()).collect() }, cur.into(), e))
        }
    }
}

/// A label without notes about Playnite add-ons or Windows performance ("Show banners (Requires
/// ThemeExtras extension)" → "Show banners"): Lodestar draws those itself.
fn clean_label(label: &str) -> String {
    let mut out = label.to_string();
    while let Some(open) = out.find('(') {
        let Some(close) = out[open..].find(')').map(|c| open + c) else { break };
        let inner = out[open + 1..close].to_ascii_lowercase();
        if inner.contains("requires") || inner.contains("performance") || inner.contains("extension") {
            out.replace_range(open..=close, "");
        } else {
            break;
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The chosen values as one resource dictionary laid after the theme's own (None: nothing to
/// change).
pub(crate) fn overrides_xaml(spec: &OptionsSpec, values: &BTreeMap<String, String>) -> Option<String> {
    let mut body = String::new();
    for o in spec.options.iter().filter(|o| !o.is_setting()) {
        let Some(v) = values.get(&o.key) else { continue };
        let v = v.trim();
        if v.eq_ignore_ascii_case(o.default.trim()) {
            continue;
        }
        let (k, v) = (esc(&o.key), esc(v));
        let el = &o.element;
        let line = match el.as_str() {
            "SolidColorBrush" => format!("<SolidColorBrush x:Key=\"{k}\" Color=\"{v}\" />"),
            _ => format!("<{el} x:Key=\"{k}\">{v}</{el}>"),
        };
        body.push_str("  ");
        body.push_str(&line);
        body.push('\n');
    }
    (!body.is_empty()).then(|| {
        format!(
            "<ResourceDictionary xmlns=\"http://schemas.microsoft.com/winfx/2006/xaml/presentation\" \
             xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\" \
             xmlns:sys=\"clr-namespace:System;assembly=mscorlib\">\n{body}</ResourceDictionary>\n"
        )
    })
}

/// The chosen values as the importer reads them.
#[derive(Clone, Debug, Default)]
pub(crate) struct Choices {
    /// The chosen constants as one resource dictionary laid after the theme's own.
    pub constants: Option<String>,
    /// The Playnite settings the player chose.
    pub settings: PlayerSettings,
}

pub(crate) fn choices(spec: &OptionsSpec, values: &BTreeMap<String, String>) -> Choices {
    let mut settings = PlayerSettings::default();
    for o in spec.options.iter().filter(|o| o.is_setting()) {
        settings.set(&o.key, SVal::Bool(o.is_on(values)));
    }
    Choices { constants: overrides_xaml(spec, values), settings }
}

/// The Playnite settings a fullscreen theme offers as its options, at Playnite's defaults.
fn player_options(mode: Mode) -> Vec<ThemeOption> {
    if mode != Mode::Fullscreen {
        return vec![];
    }
    settings::PLAYER_SETTINGS
        .iter()
        .map(|s| {
            let on = matches!(settings::setting(s.key), Some(SVal::Bool(true)));
            ThemeOption {
                key: s.key.to_string(),
                label: s.label.to_string(),
                section: String::new(),
                kind: Kind::Switch { on: "True".into(), off: "False".into() },
                default: if on { "True" } else { "False" }.into(),
                element: SETTING.into(),
                effect: false,
            }
        })
        .collect()
}

/// The values an option is tried with to see whether it changes anything.
fn probes(o: &ThemeOption) -> Vec<String> {
    match &o.kind {
        Kind::Switch { on, off } => vec![if o.default.eq_ignore_ascii_case(on) { off.clone() } else { on.clone() }],
        Kind::Choice { choices } => choices.iter().filter(|c| !c.eq_ignore_ascii_case(&o.default)).cloned().collect(),
        Kind::Range { min, max, step } => {
            let d: f64 = o.default.parse().unwrap_or(*min);
            let far = if (d - min).abs() > (max - d).abs() { *min } else { *max };
            vec![number_text(far, *step)]
        }
        Kind::Color => match parse_color(&o.default) {
            Some(c) => {
                let [r, g, b, a] = c.0;
                vec![color_text(Color([255 - r, 255 - g, 128u8.wrapping_add(b), a.max(200)]))]
            }
            None => vec![],
        },
        // Labels Lodestar's own screens don't show.
        Kind::Text => vec![],
    }
}

/// Which options change what Lodestar draws: each is tried once, flipped, against the
/// theme's look and colours (`fingerprint`). The theme's texts are read once and only the
/// small overrides dictionary changes between tries.
pub(crate) fn mark_effects(
    spec: &mut OptionsSpec,
    source: &Source,
    mode: Mode,
    base_values: &BTreeMap<String, String>,
    fingerprint: &dyn Fn(&Tree) -> String,
) {
    if spec.options.is_empty() {
        return;
    }
    let files = source.files();
    let mut texts = Texts::load(source, &files, mode);
    let base_n = texts.files.len();
    // Text of the translated screens measured in Titillium Web (loaded once: it takes a while).
    let mut measurer = None;
    // `views`: the theme's own screens as Lodestar translates them count too. Only Playnite
    // settings are tried on them: those screens are where the player sees such a setting
    // (a game's name beside the list), and translating them takes longer than the look.
    let mut with = |texts: &mut Texts, values: &BTreeMap<String, String>, views: bool| -> String {
        texts.files.truncate(base_n);
        let chosen = choices(spec, values);
        if let Some(x) = chosen.constants {
            texts.push_theme("thememodifier.xaml", x);
        }
        let mut tree = Tree::build(texts, &files);
        tree.settings = chosen.settings;
        let mut print = fingerprint(&tree);
        if views {
            let m = measurer.get_or_insert_with(|| super::compile::FontMeasurer::new(None, None));
            print.push_str(&super::compile::views_print(&tree, source, m));
        }
        print
    };
    let base = with(&mut texts, base_values, false);
    let mut base_views = None;
    let mut effects = vec![false; spec.options.len()];
    for (i, o) in spec.options.iter().enumerate() {
        let views = o.is_setting() && mode == Mode::Fullscreen;
        let base: &str =
            if views { base_views.get_or_insert_with(|| with(&mut texts, base_values, true)) } else { &base };
        let cur = o.value(base_values).to_string();
        for p in probes(o) {
            // Against the value in effect (a chosen value may already equal the probe).
            let p = if p.eq_ignore_ascii_case(&cur) { o.default.clone() } else { p };
            let mut vals = base_values.clone();
            vals.insert(o.key.clone(), p);
            if with(&mut texts, &vals, views) != base {
                effects[i] = true;
                break;
            }
        }
    }
    for (o, e) in spec.options.iter_mut().zip(effects) {
        o.effect = e;
    }
}

/// What the importer draws from a resolved theme, as one comparable text: the look (without
/// its coverage counts), the colours and the banner switch.
pub(crate) fn fingerprint(tree: &Tree) -> String {
    let look = super::extract::look_of(tree, None);
    let mut v = serde_json::to_value(&look).unwrap_or_default();
    if let serde_json::Value::Object(o) = &mut v {
        o.remove("coverage");
    }
    let res = super::extract::resources_of(tree);
    let pal = super::map::palette(&res, tree.mode, None, None, None);
    let banner = look.plugins.banners.then(|| super::extras::banner_look(tree, tree.mode == Mode::Fullscreen));
    format!("{v}|{:?}|{:?}|{banner:?}", pal.palette, pal.background)
}

/// The theme's options typed against the theme itself, with `values` (cleaned of anything
/// that isn't an option or equals the theme's own value) and each option's effect marked.
/// None: the theme has no options (no thememodifier.yaml, and not a fullscreen theme).
///
/// `known`: the options as an earlier import marked them — reused when the theme lists the same
/// options, so applying a value is quick (`refresh_effects` marks them again afterwards).
pub(crate) fn prepare(
    source: &Source,
    mode: Mode,
    values: &BTreeMap<String, String>,
    known: Option<&OptionsSpec>,
) -> Option<(OptionsSpec, BTreeMap<String, String>)> {
    let files = source.files();
    let yaml = files
        .iter()
        .filter(|(f, _)| f.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case("thememodifier.yaml")))
        .min_by_key(|(f, _)| f.matches('/').count())
        .and_then(|(f, _)| source.read_text(f, super::source::YAML_LIMIT));
    let mut spec = OptionsSpec { options: player_options(mode), ..Default::default() };
    if let Some(yaml) = yaml {
        let texts = Texts::load(source, &files, mode);
        let tree = Tree::build(&texts, &files);
        spec.options.extend(spec_of(&tree, &yaml).options);
    }
    if spec.is_empty() {
        return None;
    }
    let values = spec.clean(values);
    let same = known.filter(|k| {
        k.options.len() == spec.options.len()
            && k.options.iter().zip(&spec.options).all(|(a, b)| a.key == b.key && a.kind == b.kind)
    });
    match same {
        Some(k) => {
            for (o, old) in spec.options.iter_mut().zip(&k.options) {
                o.effect = old.effect;
            }
        }
        None => mark_effects(&mut spec, source, mode, &values, &fingerprint),
    }
    spec.values = values.clone();
    Some((spec, values))
}

/// The theme's pictures kept for applying options again, in all.
const PICTURE_BUDGET: u64 = 64 << 20;
/// Biggest single file kept.
const KEPT_FILE_LIMIT: u64 = 32 << 20;

/// Keep the theme's own files in the new theme (`dir`/[`SNAPSHOT`]) so options can be applied
/// again later without the original package: its XAML and YAML, fonts and pictures (up to
/// [`PICTURE_BUDGET`] bytes in all, without [`is_preview`] ones), and the videos / sounds
/// already copied (`media`: theme file → the copy in `dir`) as hard links (left out where the
/// file system can't link).
pub(crate) fn snapshot(
    source: &Source,
    files: &[(String, u64)],
    dir: &Path,
    media: &[(String, String)],
) -> anyhow::Result<()> {
    let root = dir.join(SNAPSHOT);
    std::fs::create_dir_all(&root)?;
    let ext = |f: &str| f.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let mut budget = PICTURE_BUDGET;
    for (f, size) in files {
        if f.split('/').any(|c| c.is_empty() || c == "." || c == ".." || c.starts_with('.')) {
            continue;
        }
        let e = ext(f);
        let keep = match e.as_str() {
            "xaml" | "yaml" | "yml" => *size <= super::tree::FILE_LIMIT,
            "ttf" | "otf" | "ttc" => *size <= super::FONT_LIMIT,
            "png" | "jpg" | "jpeg" | "webp" | "bmp" | "gif" | "ico" => {
                let ok = *size <= budget && !is_preview(f);
                if ok {
                    budget -= size;
                }
                ok
            }
            _ => false,
        };
        if keep {
            let dest = root.join(f);
            if let Some(p) = dest.parent() {
                std::fs::create_dir_all(p)?;
            }
            source.put(f, &dest, KEPT_FILE_LIMIT);
        }
    }
    for (rel, copy) in media {
        let dest = root.join(rel);
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p)?;
        }
        let _ = std::fs::hard_link(dir.join(copy), &dest);
    }
    Ok(())
}

/// A picture of what an option looks like, for the settings screen of Playnite or of the
/// theme's add-on ("Theme Options/1. Main Page Styles/Previews/…", "Themes Option/10.Preview/…").
/// Themes only name them in their options lists, never in their views, and Lodestar's settings
/// don't show them, yet they can take tens of megabytes.
fn is_preview(path: &str) -> bool {
    let mut folders = path.split('/').rev().skip(1);
    folders.any(|c| {
        let c = c.to_ascii_lowercase();
        c.ends_with("preview") || c.ends_with("previews")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_previews_are_not_kept() {
        assert!(is_preview("Theme Options/1. Main Page Styles/Previews/0. Default Theme.jpg"));
        assert!(is_preview("Themes Option/10.Preview/Font/Roboto.jpg"));
        assert!(!is_preview("Images/Previews.png"));
        assert!(!is_preview("Images/Background.jpg"));
    }

    const CONSTANTS: &str = r##"<ResourceDictionary xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" xmlns:sys="clr-namespace:System;assembly=mscorlib">
        <sys:Boolean x:Key="UseRound">True</sys:Boolean>
        <Visibility x:Key="ShowBadge">Collapsed</Visibility>
        <sys:Double x:Key="Darken">0.45</sys:Double>
        <sys:Int32 x:Key="Width">300</sys:Int32>
        <CornerRadius x:Key="Radius">6</CornerRadius>
        <HorizontalAlignment x:Key="TitleAlign">Center</HorizontalAlignment>
        <Color x:Key="AccentColor">#FF3366CC</Color>
        <SolidColorBrush x:Key="AccentBrush" Color="#80FF0000" />
        <LinearGradientBrush x:Key="Fade" StartPoint="0,0" EndPoint="0,1">
            <GradientStop Color="Black" Offset="0" /><GradientStop Color="White" Offset="1" />
        </LinearGradientBrush>
        <sys:String x:Key="LOCPlayGame">Launch</sys:String>
    </ResourceDictionary>"##;

    const YAML: &str = "Constants:\n    - \"General\"\n    - UseRound: 'Use rounded corners'\n    - ShowBadge: 'Show badge'\n    - Darken(0.0,1.0): 'Darken amount'\n    - Width(80,600): 'Width'\n    - Radius(0,20): 'Corner radius'\n    - \"Look\"\n    - TitleAlign: Title alignment\n    - AccentColor: 'Accent'\n    - AccentBrush: 'Accent brush'\n    - Fade: 'A gradient'\n    - Missing: 'Not in the theme'\n    - LOCPlayGame: 'Play button label'\n    - UseRound: 'twice'\nStyles:\n    - Something: 'not a constant'\n";

    fn spec() -> OptionsSpec {
        let texts = Texts::with_theme(Mode::Desktop, vec![("Constants.xaml".into(), CONSTANTS.into())]);
        let tree = Tree::build(&texts, &[]);
        spec_of(&tree, YAML)
    }

    #[test]
    fn playnite_settings_are_options_of_fullscreen_themes_only() {
        assert!(player_options(Mode::Desktop).is_empty());
        let mut s = OptionsSpec { options: player_options(Mode::Fullscreen), ..Default::default() };
        assert_eq!(s.options[0].key, "Fullscreen.ShowGameTitles");
        // A setting that changes nothing isn't counted as one of the theme's options.
        assert_eq!(s.others(), 0);
        s.options.extend(spec().options);
        let values: BTreeMap<String, String> = [
            ("Fullscreen.ShowGameTitles".to_string(), "True".to_string()),
            ("UseRound".to_string(), "False".to_string()),
        ]
        .into();
        let c = choices(&s, &values);
        assert_eq!(c.settings.get("Fullscreen.ShowGameTitles"), Some(SVal::Bool(true)));
        // Only the theme's constant goes into its resources.
        let x = c.constants.unwrap();
        assert!(x.contains("UseRound") && !x.contains("ShowGameTitles"), "{x}");
        assert_eq!(choices(&s, &BTreeMap::new()).settings.get("ShowGameTitles"), Some(SVal::Bool(false)));
    }

    #[test]
    fn options_are_typed_from_the_theme() {
        let s = spec();
        let got: Vec<(&str, &str, &Kind, &str)> =
            s.options.iter().map(|o| (o.key.as_str(), o.section.as_str(), &o.kind, o.default.as_str())).collect();
        let sw = Kind::Switch { on: "True".into(), off: "False".into() };
        let vis = Kind::Switch { on: "Visible".into(), off: "Collapsed".into() };
        let r01 = Kind::Range { min: 0.0, max: 1.0, step: 0.01 };
        let rw = Kind::Range { min: 80.0, max: 600.0, step: 1.0 };
        let rr = Kind::Range { min: 0.0, max: 20.0, step: 1.0 };
        let align = Kind::Choice { choices: ["Left", "Center", "Right", "Stretch"].map(String::from).to_vec() };
        assert_eq!(
            got,
            vec![
                ("UseRound", "General", &sw, "True"),
                ("ShowBadge", "General", &vis, "Collapsed"),
                ("Darken", "General", &r01, "0.45"),
                ("Width", "General", &rw, "300"),
                ("Radius", "General", &rr, "6"),
                ("TitleAlign", "Look", &align, "Center"),
                ("AccentColor", "Look", &Kind::Color, "#FF3366CC"),
                ("AccentBrush", "Look", &Kind::Color, "#80FF0000"),
                ("LOCPlayGame", "Look", &Kind::Text, "Launch"),
            ]
        );
        assert_eq!(s.options[5].label, "Title alignment");
        assert_eq!(s.options[3].element, "sys:Int32");
    }

    #[test]
    fn values_step_and_read() {
        let s = spec();
        let mut v = BTreeMap::new();
        let o = |k: &str| s.options.iter().find(|o| o.key == k).unwrap().clone();
        assert_eq!(o("UseRound").stepped(&v, 1), "False");
        assert_eq!(o("ShowBadge").stepped(&v, 1), "Visible");
        assert_eq!(o("Darken").stepped(&v, 1), "0.55");
        assert_eq!(o("Darken").stepped(&v, -1), "0.35");
        assert_eq!(o("Width").stepped(&v, 1), "352");
        assert_eq!(o("TitleAlign").stepped(&v, 1), "Right");
        assert_eq!(o("TitleAlign").stepped(&v, -1), "Left");
        v.insert("TitleAlign".into(), "Left".into());
        assert_eq!(o("TitleAlign").stepped(&v, -1), "Stretch");
        v.insert("Darken".into(), "0.98".into());
        assert_eq!(o("Darken").stepped(&v, 1), "1");
        assert_eq!(o("Darken").value_text(&v), "0.98");
        assert_eq!(o("UseRound").value_text(&v), "On");
        // Only real, changed values are kept.
        v.insert("UseRound".into(), "True".into());
        v.insert("Nope".into(), "1".into());
        let clean = s.clean(&v);
        assert_eq!(clean.keys().collect::<Vec<_>>(), ["Darken", "TitleAlign"]);
    }

    #[test]
    fn ranges_that_are_not_numbers_never_panic() {
        // `nan` / `inf` bounds aren't a range: the option isn't offered as a slider.
        let texts = Texts::with_theme(Mode::Desktop, vec![("Constants.xaml".into(), CONSTANTS.into())]);
        let tree = Tree::build(&texts, &[]);
        let yaml = "Constants:\n    - Darken(nan,nan): 'Darken'\n    - Width(inf,-inf): 'Width'\n    - Radius(20,0): 'Corner'\n";
        let s = spec_of(&tree, yaml);
        assert!(s.options.iter().all(|o| o.key != "Darken" && o.key != "Width"), "{:?}", s.options);
        // Bounds written the wrong way round still work.
        let r = s.options.iter().find(|o| o.key == "Radius").unwrap();
        assert_eq!(r.stepped(&BTreeMap::new(), 1), "8");
        // A spec stored by an earlier import with such bounds is read without a panic.
        let bad = ThemeOption {
            key: "FontSize".into(),
            label: "Text size".into(),
            section: String::new(),
            kind: Kind::Range { min: f64::NAN, max: f64::NAN, step: 1.0 },
            default: "nan".into(),
            element: "sys:Double".into(),
            effect: true,
        };
        let v = BTreeMap::new();
        assert_eq!(bad.value_text(&v), "0");
        assert_eq!(bad.stepped(&v, 1), "1");
        assert_eq!(bad.number(&v), 0.0);
    }

    #[test]
    fn overrides_win_over_the_theme() {
        let s = spec();
        let mut v = BTreeMap::new();
        assert_eq!(overrides_xaml(&s, &v), None);
        v.insert("UseRound".into(), "False".into());
        v.insert("Darken".into(), "0.8".into());
        v.insert("AccentBrush".into(), "#FF00FF00".into());
        v.insert("TitleAlign".into(), "Right".into());
        v.insert("LOCPlayGame".into(), "Go <now>".into());
        let x = overrides_xaml(&s, &v).unwrap();
        let mut texts = Texts::with_theme(Mode::Desktop, vec![("Constants.xaml".into(), CONSTANTS.into())]);
        texts.push_theme("thememodifier.xaml", x);
        let tree = Tree::build(&texts, &[]);
        let val = |k: &str| tree.eval_node(tree.resource(k).unwrap(), 0).unwrap();
        assert_eq!(val("UseRound").boolean(), Some(false));
        assert_eq!(val("Darken").num(), Some(0.8));
        assert_eq!(val("AccentBrush").color(), Some(Color([0, 255, 0, 255])));
        let n = tree.resource("TitleAlign").unwrap();
        assert_eq!(n.text().map(str::trim), Some("Right"));
        let n = tree.resource("LOCPlayGame").unwrap();
        assert_eq!(n.text(), Some("Go <now>"));
    }

    #[test]
    fn labels_lose_add_on_notes() {
        assert_eq!(clean_label("Show banners (Requires ThemeExtras extension)"), "Show banners");
        assert_eq!(clean_label("Use rounded corners (Can cause performance degradation)"), "Use rounded corners");
        assert_eq!(clean_label("Library Banners (Requires ThemeExtras)"), "Library Banners");
        assert_eq!(clean_label("Darken amount (0.0-1.0)"), "Darken amount (0.0-1.0)");
    }

    #[test]
    fn numbers_read_well() {
        assert_eq!(number_text(0.45, 0.01), "0.45");
        assert_eq!(number_text(1.0, 0.01), "1");
        assert_eq!(number_text(0.0, 0.01), "0");
        assert_eq!(number_text(-0.001, 0.01), "0");
        assert_eq!(number_text(351.6, 1.0), "352");
    }
}
