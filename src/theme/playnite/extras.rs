//! A theme's per-game pictures from its own files:
//!
//! - **Banners** (`themeExtras.yaml`, the ThemeExtras add-on): a strip on each cover naming
//!   where the game comes from — by library add-on id (Steam, GOG …), by platform (spec id or
//!   name) or by source name, else a default. Lodestar copies the ones its sources and consoles
//!   need into `images/banners/{source,platform}/…` and writes [`BANNERS`] (where the banner
//!   sits, whether the theme shows it), which the desktop grid draws.
//!
//! CONTRACT:
//!   pub const BANNERS: &str = "banners.toml";
//!   pub struct Banners { placement, shown, sources, platforms, default }  — `Banners::load(theme_dir)`,
//!       `image(source_key, platform)` → the file to draw
//!   pub(crate) struct Extras — `Extras::read(&Source, &files, banner_look)`, `write(&Source, dir)`
//!       (copies the pictures)

use super::convert;
use super::source::Source;
use super::tree::{Tree, Value, name_of, tag};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Where the banner settings are written in the imported theme.
pub const BANNERS: &str = "banners.toml";
/// Pictures bigger than this aren't banners / icons.
const PICTURE_CAP: u64 = 4 * 1024 * 1024;
/// Most per-game pictures copied from one theme.
const MAX_PICTURES: usize = 600;
const PICTURE_EXTS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "bmp"];

/// Lodestar sources → Playnite's library add-on ids and source names.
const SOURCES: &[(&str, &[&str], &[&str])] = &[
    ("steam", &["cb91dfc9-b977-43bf-8e70-55f46e410fab"], &["Steam"]),
    ("gog", &["aebe8b7c-6dc3-4a66-af31-e7375c6b5e9e"], &["GOG", "GOG Galaxy"]),
    ("epic", &["00000002-dbd1-46c6-b5d0-b1ba559d10e4"], &["Epic", "Epic Games", "Epic Games Store"]),
    ("itch", &["00000001-ebb2-4eec-abcb-7c89937a42bb"], &["itch.io", "itch"]),
    ("amazon", &["402674cd-4af6-4886-b6ec-0e695bfa0688"], &["Amazon", "Amazon Games", "Prime Gaming"]),
    ("battlenet", &["e3c26a3d-d695-4cb7-a769-5ff7612c7edd"], &["Battle.net", "Blizzard"]),
    ("ea", &["85dd7072-2f20-4e76-a007-41035e390724"], &["EA app", "EA", "Origin"]),
    ("ubisoft", &["c2f038e5-8b92-4877-91f1-da9094155fc5"], &["Ubisoft Connect", "Ubisoft", "Uplay"]),
    ("rockstar", &["88409022-088a-4de8-805a-fdbac291f00a"], &["Rockstar Games", "Rockstar"]),
    ("riot", &[], &["Riot Games", "Riot"]),
    ("hoyoplay", &[], &["HoYoPlay"]),
    ("minecraft", &[], &["Minecraft"]),
    ("lutris", &[], &["Lutris"]),
    ("heroic", &[], &["Heroic"]),
    ("bottles", &[], &["Bottles"]),
    ("romm", &[], &["RomM"]),
];

/// Lodestar's `Game.platform` values for PC games → Playnite platform ids.
const PC: &[(&str, &str)] = &[("windows", "pc_windows"), ("linux", "pc_linux")];

/// Where the copied banners go in the imported theme.
const BANNER_DIR: &str = "images/banners";

/// Where the banner sits on a cover.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Placement {
    /// A strip above the cover (the cover moves down), as Stardust's StackPanel.
    Above,
    /// Over the cover's top edge.
    #[default]
    Top,
    /// Over the cover's bottom edge.
    Bottom,
}

/// `banners.toml`: the banner look of an imported theme.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Banners {
    pub placement: Placement,
    /// The theme shows banners (its own switch, a theme option, says so).
    pub shown: bool,
    /// Lodestar sources / consoles with a banner of their own.
    pub sources: Vec<String>,
    pub platforms: Vec<String>,
    /// There is a default banner for everything else.
    pub default: bool,
}

impl Banners {
    pub fn load(theme_dir: &Path) -> Option<Banners> {
        toml::from_str(&std::fs::read_to_string(theme_dir.join(BANNERS)).ok()?).ok()
    }

    /// The banner for a game (theme-relative path), as ThemeExtras picks it: its store's,
    /// else its console's, else the default.
    pub fn image(&self, source_key: &str, platform: Option<&str>) -> Option<String> {
        if self.sources.iter().any(|s| s == source_key) {
            return Some(format!("images/banners/source/{source_key}.png"));
        }
        if let Some(p) = platform.map(str::trim).filter(|p| !p.is_empty())
            && let Some(name) = self.platforms.iter().find(|x| x.eq_ignore_ascii_case(p))
        {
            return Some(format!("images/banners/platform/{name}.png"));
        }
        self.default.then(|| "images/banners/default.png".to_string())
    }

    /// [`Banners::load`] once per theme folder (again when the file changes); shown banners
    /// only. For the desktop grid, which asks every frame.
    pub fn cached(theme_dir: &Path) -> Option<std::sync::Arc<Banners>> {
        type Cache = Option<(PathBuf, Option<std::time::SystemTime>, Option<std::sync::Arc<Banners>>)>;
        static C: std::sync::Mutex<Cache> = std::sync::Mutex::new(None);
        let stamp = std::fs::metadata(theme_dir.join(BANNERS)).and_then(|m| m.modified()).ok();
        let mut c = C.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((d, s, b)) = c.as_ref()
            && d == theme_dir
            && *s == stamp
        {
            return b.clone();
        }
        let b = stamp.and_then(|_| Banners::load(theme_dir)).filter(|b| b.shown).map(std::sync::Arc::new);
        *c = Some((theme_dir.to_path_buf(), stamp, b.clone()));
        b
    }
}

/// What a banner folder is keyed by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum By {
    // In the order ThemeExtras looks: plugin id before source name, spec id before name.
    PluginId,
    SourceName,
    PlatformId,
    PlatformName,
}

/// One folder of banners to copy.
#[derive(Clone, Debug)]
struct Folder {
    /// Theme folder ("Images/Banners/PluginId"), its file name pattern ("{0}.png").
    dir: String,
    pattern: String,
    by: By,
}

/// The theme's per-game pictures (see the module docs).
#[derive(Clone, Debug, Default)]
pub(crate) struct Extras {
    files: Vec<(String, u64)>,
    folders: Vec<Folder>,
    /// themeExtras.yaml was found (banners).
    pub banners: Option<Banners>,
    default_banner: Option<String>,
}

fn ext(rel: &str) -> String {
    rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

/// A file name Lodestar can bind to (no path separators or dots that walk folders).
fn safe(name: &str) -> Option<String> {
    let n = name.trim();
    (!n.is_empty() && !n.contains(['/', '\\']) && n != "." && n != ".." && n.len() <= 80).then(|| n.to_string())
}

/// The flat `Key: Value` lines of themeExtras.yaml.
fn yaml_map(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with(['#', '-']) && !l.starts_with([' ', '\t']))
        .filter_map(|l| {
            let (k, v) = l.split_once(':')?;
            let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
            (!v.is_empty()).then(|| (k.trim().trim_start_matches('\u{feff}').to_string(), v.replace('\\', "/")))
        })
        .collect()
}

impl Extras {
    /// Read `themeExtras.yaml` and how the theme's item template shows the banner.
    pub fn read(source: &Source, files: &[(String, u64)], banner: Option<(bool, Placement)>) -> Extras {
        let mut x = Extras { files: files.to_vec(), ..Default::default() };
        let yaml = files
            .iter()
            .filter(|(f, _)| f.rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case("themeExtras.yaml")))
            .min_by_key(|(f, _)| f.matches('/').count())
            .and_then(|(f, _)| source.read_text(f, super::source::YAML_LIMIT));
        let Some(yaml) = yaml else { return x };
        let m = yaml_map(&yaml);
        let mut any = false;
        for (key, by) in [
            ("BannersByPluginIdPath", By::PluginId),
            ("BannersBySpecIdPath", By::PlatformId),
            ("BannersByPlatformNamePath", By::PlatformName),
            ("BannersBySourceNamePath", By::SourceName),
        ] {
            if let Some(dir) = m.get(key) {
                x.folders.push(Folder { dir: dir.clone(), pattern: "{0}.*".into(), by });
                any = true;
            }
        }
        x.default_banner = m.get("DefaultBannerPath").and_then(|p| super::tree::theme_file(files, p));
        if any || x.default_banner.is_some() {
            let (shown, placement) = banner.unwrap_or((true, Placement::Top));
            x.banners = Some(Banners { placement, shown, ..Default::default() });
        }
        x
    }

    /// The theme file for `name` in a folder (`{0}` → name, any picture extension when the
    /// pattern says `.*` or the theme used another one).
    fn find(&self, f: &Folder, name: &str) -> Option<String> {
        let want = f.pattern.replace("{0}", name);
        let stem = want.rsplit_once('.').map(|(s, _)| s).unwrap_or(&want).to_ascii_lowercase();
        let dir = f.dir.trim_matches('/').to_ascii_lowercase();
        self.files
            .iter()
            .filter(|(p, size)| *size > 0 && *size <= PICTURE_CAP && PICTURE_EXTS.contains(&ext(p).as_str()))
            .find(|(p, _)| {
                let l = p.to_ascii_lowercase();
                let (d, b) = l.rsplit_once('/').unwrap_or(("", &l));
                d == dir && b.rsplit_once('.').map(|(s, _)| s) == Some(stem.as_str())
            })
            .map(|(p, _)| p.clone())
    }

    /// Copy the pictures Lodestar's sources and consoles need into `dir` (the new theme) and
    /// write `banners.toml`. Returns how many pictures were copied.
    pub fn write(&mut self, source: &Source, dir: &Path) -> anyhow::Result<usize> {
        let mut copied = 0usize;
        let mut done: BTreeMap<String, ()> = BTreeMap::new();
        let mut put = |rel: &str, dest: String, copied: &mut usize| -> bool {
            if done.contains_key(&dest) {
                return true;
            }
            if *copied >= MAX_PICTURES {
                return false;
            }
            let Some(bytes) = source.read(rel, PICTURE_CAP) else { return false };
            let png = to_png(&bytes, rel).unwrap_or(bytes);
            let path: PathBuf = dir.join(&dest);
            if let Some(p) = path.parent()
                && std::fs::create_dir_all(p).is_err()
            {
                return false;
            }
            if std::fs::write(&path, png).is_err() {
                return false;
            }
            done.insert(dest, ());
            *copied += 1;
            true
        };
        let mut sources: Vec<String> = vec![];
        let mut platforms: Vec<String> = vec![];
        // Plugin id beats source name, spec id beats platform name (ThemeExtras' order).
        let mut folders = self.folders.clone();
        folders.sort_by_key(|f| f.by);
        for f in &folders {
            match f.by {
                By::PluginId | By::SourceName => {
                    for (key, ids, names) in SOURCES {
                        let names: &[&str] = if f.by == By::PluginId { ids } else { names };
                        let Some(rel) = names.iter().find_map(|n| self.find(f, n)) else { continue };
                        if put(&rel, format!("{BANNER_DIR}/source/{key}.png"), &mut copied)
                            && !sources.contains(&key.to_string())
                        {
                            sources.push(key.to_string());
                        }
                    }
                }
                By::PlatformName | By::PlatformId => {
                    let mut wanted: Vec<(String, &str, &str)> =
                        crate::emulation::platforms::all().iter().map(|p| (p.name.to_string(), p.id, p.name)).collect();
                    for (value, id) in PC {
                        if let Some(p) = crate::emulation::platforms::by_id(id) {
                            wanted.push((value.to_string(), p.id, p.name));
                        }
                    }
                    for (value, id, name) in wanted {
                        let Some(value) = safe(&value) else { continue };
                        let key = if f.by == By::PlatformId { id } else { name };
                        let Some(rel) = self.find(f, key) else { continue };
                        if put(&rel, format!("{BANNER_DIR}/platform/{value}.png"), &mut copied)
                            && !platforms.contains(&value)
                        {
                            platforms.push(value);
                        }
                    }
                }
            }
        }
        if let Some(b) = self.banners.as_mut() {
            b.default = self
                .default_banner
                .as_ref()
                .is_some_and(|rel| put(rel, format!("{BANNER_DIR}/default.png"), &mut copied));
            sources.sort();
            platforms.sort();
            b.sources = sources;
            b.platforms = platforms;
            if !b.sources.is_empty() || !b.platforms.is_empty() || b.default {
                std::fs::write(dir.join(BANNERS), toml::to_string_pretty(b)?)?;
            }
        }
        Ok(copied)
    }
}

/// JPEG / WebP / BMP banners as PNG (the bound paths end in .png); None when it already is one
/// or can't be read.
fn to_png(bytes: &[u8], rel: &str) -> Option<Vec<u8>> {
    if ext(rel) == "png" {
        return None;
    }
    let img = image::load_from_memory(bytes).ok()?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// Whether the theme's cover template shows the `ThemeExtras_Banner` control and where:
/// `Visibility` (literal, a resource, or `{Binding Tag, RelativeSource Self, Converter=…}` over
/// a `Tag` resource — the usual theme-option switch), and its place (before the cover in a
/// StackPanel = above it; else its VerticalAlignment over the cover).
pub(crate) fn banner_look(tree: &Tree, fullscreen: bool) -> (bool, Placement) {
    let item_docs: &[&str] = if fullscreen { &["listgameitem"] } else { &["gridviewitem"] };
    let mut found = None;
    for d in &tree.docs {
        let in_item = item_docs.iter().any(|w| d.rel.to_ascii_lowercase().contains(w));
        for n in d.doc.descendants().filter(|n| n.is_element()) {
            if name_of(n) == Some("ThemeExtras_Banner") && (in_item || found.is_none()) {
                found = Some(n);
                if in_item {
                    break;
                }
            }
        }
        if found.is_some_and(|_| in_item) {
            break;
        }
    }
    let Some(el) = found else { return (true, Placement::Top) };
    let text_of = |v: Option<Value>| -> Option<String> {
        match v? {
            Value::Bool(b) => Some(if b { "True" } else { "False" }.to_string()),
            Value::Text(t) => Some(t),
            Value::Num(n) => Some(n.to_string()),
            // Enum resources (`<VerticalAlignment x:Key="…">Bottom</VerticalAlignment>`).
            Value::Node(n) => n.text().map(|t| t.trim().to_string()),
            _ => None,
        }
    };
    let vis_attr = el.attributes().find(|a| a.name() == "Visibility").map(|a| a.value().to_string());
    let shown = match vis_attr.as_deref().map(str::trim) {
        None => true,
        // A binding the theme fixes (its own Tag, a named element's Tag, a resource).
        Some(v) if v.starts_with("{Binding") && tree.static_binding(el, v).is_some() => {
            tree.static_binding(el, v).and_then(|out| convert::boolean(&out)).unwrap_or(true)
        }
        Some(v) if v.starts_with("{Binding") => {
            let conv = v
                .split("Converter=")
                .nth(1)
                .map(|c| c.trim_start_matches('{').split(['}', ',']).next().unwrap_or("").to_string())
                .unwrap_or_default();
            let input = if v.contains("Path=Tag") || v.trim_start_matches("{Binding").trim_start().starts_with("Tag") {
                text_of(tree.local(el, "Tag")).unwrap_or_default()
            } else {
                String::new()
            };
            match convert::apply(&conv, &input, None) {
                Some(out) => out == "Visible",
                None => convert::boolean(&input).unwrap_or(true),
            }
        }
        Some(_) => text_of(tree.local(el, "Visibility")).is_none_or(|t| t.trim().eq_ignore_ascii_case("Visible")),
    };
    let above = el.parent_element().is_some_and(|p| {
        tag(p) == "StackPanel"
            && el.next_sibling_element().is_some_and(|s| {
                name_of(s) == Some("PART_ImageCover") || s.descendants().any(|d| name_of(d) == Some("PART_ImageCover"))
            })
    });
    // Docked below the cover in a DockPanel (DH Dawn): along the cover's bottom edge.
    let docked_bottom = el.parent_element().is_some_and(|p| tag(p) == "DockPanel")
        && text_of(tree.local(el, "DockPanel.Dock")).is_some_and(|d| d.trim().eq_ignore_ascii_case("Bottom"));
    let placement = if above {
        Placement::Above
    } else if docked_bottom {
        Placement::Bottom
    } else {
        match text_of(tree.local(el, "VerticalAlignment")).as_deref().map(str::trim) {
            Some("Bottom") => Placement::Bottom,
            _ => Placement::Top,
        }
    };
    (shown, placement)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::playnite::Mode;
    use crate::theme::playnite::tree::Texts;

    const EXTRAS: &str = "ThemeId: Test_1\nPersistentPaths:\n- Images/Banners\nBannersBySpecIdPath: Images/Banners/PlatformSpecId\nBannersByPlatformNamePath: Images\\Banners\\PlatformName\nBannersByPluginIdPath: Images/Banners/PluginId\nBannersBySourceNamePath: Images/Banners/SourceName\nDefaultBannerPath: Images/Banners/Unknown.png\n";

    fn item(tag_value: &str, layout: &str) -> String {
        format!(
            r#"<ResourceDictionary xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
            xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" xmlns:sys="clr-namespace:System;assembly=mscorlib">
            <sys:Boolean x:Key="ShowBanners">{tag_value}</sys:Boolean>
            <DataTemplate x:Key="GridViewItemTemplate">{layout}</DataTemplate>
            </ResourceDictionary>"#
        )
    }

    const ABOVE: &str = r#"<StackPanel>
        <ContentControl x:Name="ThemeExtras_Banner" Tag="{DynamicResource ShowBanners}"
            Visibility="{Binding Tag, RelativeSource={RelativeSource Self}, Converter={StaticResource BooleanToVisibilityConverter}}" />
        <Image Name="PART_ImageCover" /></StackPanel>"#;
    const OVER: &str = r#"<Grid><Image Name="PART_ImageCover" />
        <ContentControl x:Name="ThemeExtras_Banner" VerticalAlignment="Bottom" /></Grid>"#;
    const OVER_RES: &str = r#"<Grid><Image Name="PART_ImageCover" />
        <ContentControl x:Name="ThemeExtras_Banner" VerticalAlignment="{DynamicResource BannerAlign}" /></Grid>"#;

    /// DH Dawn: the switch is another element's Tag; the banner is docked below the cover.
    const DOCKED: &str = r#"<DockPanel>
        <Grid x:Name="MainGrid" Tag="{DynamicResource ShowBanners}" DockPanel.Dock="Top"><Image Name="PART_ImageCover" /></Grid>
        <Border x:Name="ThemeExtras_Banner" Tag="{DynamicResource Radius}" DockPanel.Dock="Bottom"
            Visibility="{Binding Tag, ElementName=MainGrid, Converter={StaticResource BooleanToVisibilityConverter}}" />
        </DockPanel>"#;

    #[test]
    fn banner_switch_on_a_named_element_and_docked_below() {
        let with_radius = |v: &str| {
            item(v, DOCKED).replace("<DataTemplate", "<sys:Double x:Key=\"Radius\">4</sys:Double><DataTemplate")
        };
        assert_eq!(look(&with_radius("True")), (true, Placement::Bottom));
        assert_eq!(look(&with_radius("False")), (false, Placement::Bottom));
    }

    fn look(xaml: &str) -> (bool, Placement) {
        let texts =
            Texts::with_theme(Mode::Desktop, vec![("DerivedStyles/GridViewItemTemplate.xaml".into(), xaml.into())]);
        let tree = Tree::build(&texts, &[]);
        banner_look(&tree, false)
    }

    #[test]
    fn banner_place_and_switch_come_from_the_template() {
        assert_eq!(look(&item("True", ABOVE)), (true, Placement::Above));
        assert_eq!(look(&item("False", ABOVE)), (false, Placement::Above));
        assert_eq!(look(&item("True", OVER)), (true, Placement::Bottom));
        let res = item("True", OVER_RES).replace(
            "<DataTemplate",
            "<VerticalAlignment x:Key=\"BannerAlign\">Bottom</VerticalAlignment><DataTemplate",
        );
        assert_eq!(look(&res), (true, Placement::Bottom));
    }

    fn png() -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(8, 2, image::Rgba([200, 10, 10, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
        out.into_inner()
    }

    fn jpg() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(8, 2, image::Rgb([10, 10, 200]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img).write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
        out.into_inner()
    }

    #[test]
    fn banners_are_copied_for_lodestar_sources_and_consoles() {
        let src = tempfile::tempdir().unwrap();
        let p = png();
        let j = jpg();
        let files: &[(&str, &[u8])] = &[
            ("theme.yaml", b"Name: T"),
            ("themeExtras.yaml", EXTRAS.as_bytes()),
            ("Images/Banners/PluginId/CB91DFC9-B977-43BF-8E70-55F46E410FAB.png", &p),
            ("Images/Banners/PluginId/aebe8b7c-6dc3-4a66-af31-e7375c6b5e9e.jpg", &j),
            ("Images/Banners/SourceName/Steam.png", &p),
            ("Images/Banners/SourceName/Ubisoft Connect.png", &p),
            ("Images/Banners/PlatformSpecId/sony_playstation2.png", &p),
            ("Images/Banners/PlatformSpecId/pc_windows.png", &p),
            ("Images/Banners/PlatformName/Sega Dreamcast.png", &p),
            ("Images/Banners/PlatformName/Sony PlayStation 2.jpg", &j),
            ("Images/Banners/Unknown.png", &p),
            ("Icons/Source/Steam.png", &p),
            ("Icons/Source/GOG.png", &p),
        ];
        for (f, d) in files {
            let path = src.path().join(f);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, d).unwrap();
        }
        let source = Source::open(src.path()).unwrap();
        let list = source.files();
        let mut x = Extras::read(&source, &list, None);
        assert!(x.banners.is_some());
        let out = tempfile::tempdir().unwrap();
        let n = x.write(&source, out.path()).unwrap();
        assert!(n >= 8, "{n}");
        let b = Banners::load(out.path()).unwrap();
        assert_eq!(b.sources, ["gog", "steam", "ubisoft"]);
        assert_eq!(b.platforms, ["PC (Windows)", "Sega Dreamcast", "Sony PlayStation 2", "windows"]);
        assert!(b.default && b.shown);
        assert_eq!(b.image("steam", None).as_deref(), Some("images/banners/source/steam.png"));
        assert_eq!(
            b.image("emulator", Some("sony playstation 2")).as_deref(),
            Some("images/banners/platform/Sony PlayStation 2.png")
        );
        assert_eq!(b.image("lutris", Some("Nope")).as_deref(), Some("images/banners/default.png"));
        // The JPEG became a PNG; every written path exists.
        // The spec id's banner wins over the platform name's (ThemeExtras' order).
        let ps2 = std::fs::read(out.path().join("images/banners/platform/Sony PlayStation 2.png")).unwrap();
        assert_eq!(ps2, p);
        let gog = std::fs::read(out.path().join("images/banners/source/gog.png")).unwrap();
        assert!(gog.starts_with(b"\x89PNG"));
        for s in &b.sources {
            assert!(out.path().join(format!("images/banners/source/{s}.png")).is_file());
        }
    }

    #[test]
    fn no_extras_no_banners() {
        let src = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("theme.yaml"), "Name: T").unwrap();
        let source = Source::open(src.path()).unwrap();
        let mut x = Extras::read(&source, &source.files(), None);
        assert!(x.banners.is_none());
        let out = tempfile::tempdir().unwrap();
        assert_eq!(x.write(&source, out.path()).unwrap(), 0);
        assert!(!out.path().join(BANNERS).exists());
    }
}
