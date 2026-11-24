//! Hand-made fixture themes modelled on Playnite's resource keys (no third-party theme files).

use super::fonts::tests::fake_font;
use super::*;
use crate::emulation::zipread::tests::write_zip;
use crate::theme::ThemeManager;

const NS: &str = r#"xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" xmlns:sys="clr-namespace:System;assembly=mscorlib""#;

fn dict(body: &str) -> String {
    format!("\u{feff}<ResourceDictionary {NS}>\n{body}\n</ResourceDictionary>")
}

fn write(dir: &Path, rel: &str, data: &[u8]) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
}

fn c(hex: &str) -> Color {
    Color::parse(hex).unwrap()
}

/// A desktop theme folder that changes the colours and uses a Windows font.
fn desktop_theme(dir: &Path) {
    write(
        dir,
        "theme.yaml",
        b"Id: Harbour_1234\nName: Harbour Night\nAuthor: Tester\nVersion: 1.2.0\nThemeApiVersion: 2.6.0\n",
    );
    write(
        dir,
        "Constants.xaml",
        dict(
            r##"<FontFamily x:Key="FontFamily">Trebuchet MS</FontFamily>
            <Color x:Key="TextColor">#FFF0F4F8</Color>
            <Color x:Key="TextColorDarker">#FF8899AA</Color>
            <Color x:Key="GlyphColor">#FF2A9D8F</Color>
            <Color x:Key="MainColor">#FF264653</Color>
            <Color x:Key="GridItemBackgroundColor">#FF1B2B33</Color>
            <SolidColorBrush x:Key="TextBrush" Color="{DynamicResource TextColor}" />
            <SolidColorBrush x:Key="GlyphBrush" Color="{DynamicResource GlyphColor}" />
            <LinearGradientBrush x:Key="WindowBackgourndBrush" StartPoint="0,0" EndPoint="0,1">
              <GradientStop Color="#FF14222A" Offset="0"/>
              <GradientStop Color="#FF0C161B" Offset="1"/>
            </LinearGradientBrush>"##,
        )
        .as_bytes(),
    );
    write(
        dir,
        "Views/Library.xaml",
        dict(r#"<Style x:Key="X" TargetType="Button"><Setter Property="Template"><Setter.Value><ControlTemplate TargetType="Button"><Grid/></ControlTemplate></Setter.Value></Setter></Style>"#).as_bytes(),
    );
    write(dir, "Images/applogo.png", b"\x89PNG fake");
    write(dir, "Fonts/icofont.ttf", &fake_font("IcoFont", "Regular"));
}

#[test]
fn desktop_theme_folder_becomes_a_desktop_palette() {
    let src = tempfile::tempdir().unwrap();
    let themes = tempfile::tempdir().unwrap();
    desktop_theme(src.path());
    // The picker may hand us theme.yaml itself.
    let rep = import(&src.path().join("theme.yaml"), themes.path()).unwrap();
    assert_eq!(rep.key, "harbour-night");
    assert_eq!((rep.name.as_str(), rep.author.as_str(), rep.version.as_str()), ("Harbour Night", "Tester", "1.2.0"));
    assert_eq!(rep.mode, Mode::Desktop);
    assert_eq!(rep.playnite_id, "Harbour_1234");
    assert!(rep.layout.is_empty());
    let f = ThemeFile::parse(&std::fs::read_to_string(rep.dir.join("theme.toml")).unwrap()).unwrap();
    assert_eq!(f.theme.origin, "playnite");
    assert!(!f.theme.controller, "desktop themes aren't offered for controller mode");
    let p = &f.palette;
    assert_eq!(p.text, c("#f0f4f8"));
    assert_eq!(p.text_dim, c("#8899aa"));
    assert_eq!(p.accent, c("#2a9d8f"));
    assert_eq!(p.surface, c("#1b2b33"));
    assert_eq!(p.surface_alt, c("#264653"));
    // Gradient → its average.
    assert_eq!(p.background, c("#101c23"));
    assert!(p.dark);
    // Report: colours from the theme; the Windows font, the gradient, the layout, the icon
    // font and the logo didn't come along.
    let all = format!("{:#?}", rep);
    assert!(rep.carried[0].contains("text") && rep.carried[0].contains("accent"), "{all}");
    let missed = rep.missed.join("\n");
    assert!(missed.contains("“Trebuchet MS” — a Windows font"), "{all}");
    assert!(missed.contains("own screens"), "{all}");
    assert!(missed.contains("1 other picture "), "{all}");
    // The window's gradient goes to the desktop style with all its colours.
    let t = raw(&rep.dir);
    let wg = t["desktop"]["window_gradient"].as_table().expect("window gradient");
    assert_eq!(wg["stops"].as_array().unwrap().len(), 2);
    assert!(rep.carried.iter().any(|l| l.contains("Gradients and fills of the window")), "{all}");
    assert!(!rep.dir.join("fonts").exists(), "no text font to copy");
    // Lodestar's theme list picks it up and it loads.
    let m = ThemeManager::new(themes.path().to_path_buf());
    let info = m.list().iter().find(|i| i.key == "harbour-night").expect("listed");
    assert_eq!(info.meta.name, "Harbour Night");
    let (t, problem) = m.load_checked("harbour-night");
    assert_eq!((t.key.as_str(), problem), ("harbour-night", None));
}

/// A fullscreen .pthm package: background picture, bundled font family, a video.
fn fullscreen_package(path: &Path) {
    let constants = dict(
        r##"<Color x:Key="TextColor">#FFFAFAFA</Color>
        <Color x:Key="GlyphColor">#FFE63946</Color>
        <Color x:Key="ControlBackgroundColor">#FF2A2D3A</Color>
        <SolidColorBrush x:Key="TextBrush" Color="{DynamicResource TextColor}" />
        <SolidColorBrush x:Key="GlyphBrush" Color="{DynamicResource GlyphColor}" />
        <SolidColorBrush x:Key="ControlBackgroundBrush" Color="{DynamicResource ControlBackgroundColor}" />
        <SolidColorBrush x:Key="SelectionBrush" Color="#FFFFB703" />
        <ImageBrush x:Key="MainBackgourndBrush" ImageSource="{ThemeFile 'Images\Backdrop.png'}" Stretch="UniformToFill" />
        <Style x:Key="TextBlockBaseStyle" TargetType="TextBlock">
          <Setter Property="FontFamily" Value="{DynamicResource FontTitilliumWebLight}" />
        </Style>"##,
    );
    let main = dict(
        r#"<Storyboard x:Key="Fade"/><Style x:Key="S"><Setter Property="Template"><Setter.Value><ControlTemplate/></Setter.Value></Setter></Style>"#,
    );
    let light = fake_font("Titillium Web", "Light");
    let regular = fake_font("Titillium Web", "Regular");
    let bold = fake_font("Titillium Web", "Bold");
    let entries: Vec<(&str, &[u8], bool)> = vec![
        ("theme.yaml", b"Id: Ember_77\nName: Ember\nAuthor: Someone\nVersion: 3.0\nMode: Fullscreen\n", false),
        ("Constants.xaml", constants.as_bytes(), true),
        ("Views/Main.xaml", main.as_bytes(), true),
        ("Images/Backdrop.png", b"\x89PNG backdrop", false),
        ("Images/Logo.png", b"\x89PNG logo", false),
        ("Fonts/TitilliumWeb-Light.ttf", &light, true),
        ("Fonts/TitilliumWeb-Regular.ttf", &regular, true),
        ("Fonts/TitilliumWeb-Bold.ttf", &bold, true),
        ("Video/intro.mp4", b"not really", false),
        ("Audio/select.wav", b"RIFF", false),
    ];
    write_zip(path, &entries);
}

/// The views of a theme.toml as TOML (fields of newer theme models included).
fn raw(dir: &Path) -> toml::Table {
    toml::from_str(&std::fs::read_to_string(dir.join("theme.toml")).unwrap()).unwrap()
}

/// The element named `name` in view `view` of a raw theme table.
fn el<'a>(t: &'a toml::Table, view: &str, name: &str) -> Option<&'a toml::Table> {
    t.get(view)?
        .get("elements")?
        .as_array()?
        .iter()
        .filter_map(|e| e.as_table())
        .find(|e| e.get("name").and_then(|n| n.as_str()) == Some(name))
}

#[test]
fn fullscreen_package_is_laid_onto_top_bar() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("Ember_77.pthm");
    fullscreen_package(&pkg);
    let themes = tmp.path().join("themes");
    let rep = import_with(&pkg, &themes, &ImportOptions { family: Some(Family::Classic), media: None }).unwrap();
    assert_eq!(rep.mode, Mode::Fullscreen);
    assert_eq!(rep.key, "ember");
    // A plain default-derived Main: translated by default (see below); Playnite's own
    // arrangement on request.
    assert_eq!(rep.family, Some(Family::Classic));
    assert_eq!(rep.suggested, Some(Family::Translated));
    assert_eq!(rep.layout, "Top Bar");
    let f = ThemeFile::parse(&std::fs::read_to_string(rep.dir.join("theme.toml")).unwrap()).unwrap();
    assert!(f.theme.controller);
    assert_eq!(f.theme.origin, "playnite");
    assert_eq!(f.palette.text, c("#fafafa"));
    assert_eq!(f.palette.accent, c("#e63946"));
    assert_eq!(f.palette.selection, c("#ffb703"));
    assert_eq!(f.palette.surface_alt, c("#2a2d3a"));
    // Not defined by the theme → Playnite's fullscreen default.
    assert_eq!(f.palette.surface, c("#121830"));
    assert!(f.details.is_some(), "Playnite's details page comes along");
    // Picture copied and drawn under everything in every view.
    assert_eq!(std::fs::read(rep.dir.join("images/Backdrop.png")).unwrap(), b"\x89PNG backdrop");
    for v in [&f.collections, &f.games] {
        let pic = v.elements.iter().find(|e| e.common().name == "pn_theme_picture").expect("picture layer");
        assert!(matches!(pic, Element::Image { path, .. } if path == "images/Backdrop.png"));
        let bg = v.elements.iter().find(|e| e.common().name == "pn_background").expect("background");
        assert!(bg.common().z < pic.common().z);
        assert!(v.elements.iter().all(|e| e.common().z >= bg.common().z));
        assert_eq!(v.background, Some(f.palette.background));
        assert!(v.focus_index().is_some(), "still navigable");
    }
    // The bundled family: light for text, bold for titles; only used files are copied.
    assert_eq!(f.fonts.regular.as_deref(), Some("fonts/TitilliumWeb-Light.ttf"));
    assert_eq!(f.fonts.bold.as_deref(), Some("fonts/TitilliumWeb-Bold.ttf"));
    // Titles: Playnite's Titillium Web Bold — Lodestar's own copy when this build has it.
    let title = f.fonts.title.as_deref().unwrap_or_default();
    assert!(title == "fonts/TitilliumWeb-Bold.ttf" || title == "builtin:titillium-web-bold", "{title}");
    assert!(f.fonts.extra.values().all(|v| v.starts_with("fonts/") || v.contains("mono")), "{:?}", f.fonts.extra);
    assert!(!rep.dir.join("fonts/TitilliumWeb-Regular.ttf").exists(), "only used fonts are copied");
    // The bars and the cover look of Playnite's defaults, in the theme's colours.
    let t = raw(&rep.dir);
    let top = el(&t, "games", "pn_topbar").unwrap();
    assert!(top.get("gradient").is_some(), "{top:?}");
    let size = top["size"].as_array().unwrap()[1].as_float().unwrap();
    assert!((size - 110.0 / 1080.0).abs() < 1e-3);
    let grid = el(&t, "games", "pn_list").unwrap();
    assert_eq!(grid["selected_border"].as_str(), Some("#ffb703"), "SelectionBrush");
    assert!((grid["selected_border_width"].as_float().unwrap() - 5.0 / 1080.0).abs() < 1e-3);
    assert!((grid["selected_scale"].as_float().unwrap() - 1.06).abs() < 1e-3);
    let glow = grid["selected_glow"].as_table().unwrap();
    assert_eq!(glow["color"].as_str(), Some("#e63946"), "GlyphColor glow");
    assert_eq!(grid["uninstalled_scrim"].as_str(), Some("#000000aa"));
    assert_eq!(grid["item_caption"].as_str(), Some("{game.name}"));
    // The details band: Playnite's GameDetailsBackgroundBrush fades in over a third of it.
    let band = el(&t, "details", "pn_band").unwrap();
    let band_end = el(&t, "details", "pn_band_1").expect("the band's solid part");
    let y0 = band["pos"].as_array().unwrap()[1].as_float().unwrap();
    let y1 = band_end["pos"].as_array().unwrap()[1].as_float().unwrap();
    assert!((y0 - 3.0 / 7.0).abs() < 1e-3 && (y1 - (3.0 / 7.0 + 0.32 * 4.0 / 7.0)).abs() < 2e-3, "{y0} {y1}");
    // Its media: the sound file isn't one Playnite plays (audio/navigation …), the video isn't
    // named by any MediaElement: nothing copied, the video is listed.
    assert_eq!(rep.media_bytes, 0);
    assert!(!rep.dir.join("media").exists());
    let titles: Vec<&str> = rep.groups.iter().map(|g| g.title.as_str()).collect();
    assert_eq!(titles.first(), Some(&"Colours"));
    assert!(titles.contains(&"Layout") && titles.contains(&"Covers") && titles.contains(&"Not possible"), "{titles:?}");
    let carried = rep.carried.join("\n");
    assert!(carried.contains("Font: Titillium Web"), "{carried}");
    assert!(carried.contains("Background picture: Images/Backdrop.png"), "{carried}");
    assert!(carried.contains("“Top Bar”"), "{carried}");
    assert!(carried.contains("focus border, glow, growing focused cover"), "{carried}");
    let missed = rep.missed.join("\n");
    assert!(missed.contains("1 other picture "), "{missed}");
    assert!(missed.contains("animations"), "{missed}");
    // Loads as a theme.
    let m = ThemeManager::new(themes.clone());
    let (t, problem) = m.load_checked("ember");
    assert_eq!((t.key.as_str(), problem), ("ember", None));
}

/// A PS5-style fullscreen theme folder: one fixed-size row, its own sounds, music and a
/// background video, logos through Extra Metadata Loader.
fn row_theme(dir: &Path) {
    write(dir, "theme.yaml", b"Name: Console Row\nMode: Fullscreen\nThemeApiVersion: 2.3.0\n");
    write(
        dir,
        "Constants.xaml",
        dict(r##"<Color x:Key="TextColor">#FFF0F0F0</Color><Color x:Key="GlyphColor">#FF3A7BFF</Color>"##).as_bytes(),
    );
    write(
        dir,
        "Views/Main.xaml",
        format!(
            r##"<ResourceDictionary {NS}><Style TargetType="Main"><Setter Property="Template"><Setter.Value>
            <ControlTemplate TargetType="Main"><Grid>
              <MediaElement Source="{{ThemeFile 'Videos/Waves.mp4'}}" Stretch="UniformToFill" IsMuted="True" />
              <ContentControl x:Name="ExtraMetadataLoader_LogoLoaderControl" />
              <ListBoxEx x:Name="PART_ListGameItems" Height="320" VerticalAlignment="Bottom" />
              <TextBlock x:Name="PART_TextClock" />
            </Grid></ControlTemplate></Setter.Value></Setter></Style></ResourceDictionary>"##
        )
        .as_bytes(),
    );
    write(dir, "Videos/Waves.mp4", &[7u8; 2048]);
    write(dir, "audio/navigation.wav", b"RIFFnav");
    write(dir, "audio/activation.wav", b"RIFFact");
    write(dir, "audio/background.mp3", &[1u8; 4096]);
}

#[test]
fn a_row_theme_gets_single_row_with_its_videos_and_sounds() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    row_theme(&src);
    let themes = tmp.path().join("themes");
    let rep = import_with(&src, &themes, &ImportOptions { family: Some(Family::Row), media: None }).unwrap();
    // Its main screen translates (a visible list, a background video, a logo hook).
    assert_eq!(rep.suggested, Some(Family::Translated));
    assert_eq!(rep.family, Some(Family::Row));
    assert_eq!(rep.layout, "Single Row");
    assert!(rep.media_included, "small media come along by default");
    assert_eq!(rep.media_bytes, 2048 + 7 + 7 + 4096);
    let f = ThemeFile::parse(&std::fs::read_to_string(rep.dir.join("theme.toml")).unwrap()).unwrap();
    assert!(f.home.is_some(), "Single Row starts on its home page");
    assert_eq!(f.sounds.get("scroll").map(String::as_str), Some("media/navigation.wav"));
    assert_eq!(f.sounds.get("select").map(String::as_str), Some("media/activation.wav"));
    assert_eq!(f.music, vec!["media/background.mp3".to_string()]);
    let pic = f.home.as_ref().unwrap().elements.iter().find(|e| e.common().name == "pn_theme_picture").unwrap();
    assert!(matches!(pic, Element::Image { video: Some(v), .. } if v == "media/Waves.mp4"));
    assert!(rep.dir.join("media/Waves.mp4").is_file());
    // Logos: the theme hooks Extra Metadata Loader's logo.
    let t = raw(&rep.dir);
    assert_eq!(el(&t, "details", "pn_logo").unwrap()["visible"].as_bool(), Some(true));
    assert_eq!(el(&t, "details", "pn_title").unwrap()["hidden_if"].as_str(), Some("{art.logo}"));
    let all = rep.groups.iter().flat_map(|g| &g.lines).map(|l| l.text.clone()).collect::<Vec<_>>().join("\n");
    assert!(all.contains("Extra Metadata Loader"), "{all}");
    assert!(all.contains("navigation sounds and music play automatically in controller mode"), "{all}");
    assert!(all.contains("background video plays automatically in controller mode"), "{all}");
    // Where to change it, on the desktop and in controller mode.
    assert!(all.contains("Settings → Controller → More options, or in controller mode: Settings → Audio"), "{all}");
    assert!(
        !all.contains("turn them on") && !all.contains("→ Theme videos") && !all.contains("→ Background music"),
        "{all}"
    );

    // Leave the media out, and try the classic grid: the report's buttons do this.
    let again =
        import_with(&src, &themes, &ImportOptions { family: Some(Family::Classic), media: Some(false) }).unwrap();
    assert_eq!(again.key, rep.key, "the same theme, replaced");
    assert_eq!((again.family, again.suggested), (Some(Family::Classic), Some(Family::Translated)));
    assert!(!again.media_included && !again.dir.join("media").exists());
    let f = ThemeFile::parse(&std::fs::read_to_string(again.dir.join("theme.toml")).unwrap()).unwrap();
    assert!(f.home.is_none() && f.sounds.is_empty() && f.music.is_empty());
    let lines = again.missed.join("\n");
    assert!(lines.contains("Include its videos and sounds"), "{lines}");
    assert!(again.carried.join("\n").contains("chosen by you"));
    // Rail and rows too.
    let rail = import_with(&src, &themes, &ImportOptions { family: Some(Family::Rail), media: None }).unwrap();
    assert_eq!(rail.layout, "Fullscreen");
    let f = ThemeFile::parse(&std::fs::read_to_string(rail.dir.join("theme.toml")).unwrap()).unwrap();
    assert!(f.home.as_ref().is_some_and(|h| h.rail_index().is_some()));
}

#[test]
fn media_can_be_left_out_or_added() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    row_theme(&src);
    assert_eq!(media::DEFAULT_LIMIT, 150 << 20);
    let themes = tmp.path().join("themes");
    let off = import_with(&src, &themes, &ImportOptions { family: None, media: Some(false) }).unwrap();
    assert!(!off.media_included);
    let on = import_with(&src, &themes, &ImportOptions { family: None, media: Some(true) }).unwrap();
    assert!(on.media_included);
}

#[test]
fn titillium_and_windows_fonts_are_reported_for_what_they_are() {
    let tmp = tempfile::tempdir().unwrap();
    // Playnite's own font, not bundled with the theme.
    let a = tmp.path().join("a");
    write(&a, "theme.yaml", b"Name: Plain\nMode: Fullscreen\n");
    write(&a, "Constants.xaml", dict(r##"<Color x:Key="TextColor">#FFEEEEEE</Color>"##).as_bytes());
    let rep = import(&a, &tmp.path().join("themes")).unwrap();
    let all = format!("{:?}", rep.groups);
    assert!(all.contains("Titillium Web — it comes with Playnite"), "{all}");
    assert!(!all.contains("Windows"), "{all}");
    let f = ThemeFile::parse(&std::fs::read_to_string(rep.dir.join("theme.toml")).unwrap()).unwrap();
    assert_eq!(f.fonts.regular.as_deref(), Some("builtin:titillium-web-light"));
    assert_eq!(f.fonts.title.as_deref(), Some("builtin:titillium-web-bold"));
    // A Windows font.
    let b = tmp.path().join("b");
    write(&b, "theme.yaml", b"Name: Segoe\nMode: Fullscreen\n");
    write(&b, "Constants.xaml", dict(r##"<FontFamily x:Key="FontFamily">Segoe UI</FontFamily>"##).as_bytes());
    let rep = import(&b, &tmp.path().join("themes")).unwrap();
    assert!(rep.missed.iter().any(|m| m.contains("“Segoe UI” — a Windows font")), "{:?}", rep.missed);
    let f = ThemeFile::parse(&std::fs::read_to_string(rep.dir.join("theme.toml")).unwrap()).unwrap();
    assert_eq!(f.fonts.regular.as_deref(), Some("system:sans"));
}

#[test]
fn a_theme_font_keeps_titles_in_it_unless_titillium_is_included() {
    // The report says "Titles: Titillium Web Bold" only when the theme really gets it.
    let tmp = tempfile::tempdir().unwrap();
    let a = tmp.path().join("a");
    write(&a, "theme.yaml", b"Name: Own Font\nMode: Fullscreen\n");
    write(&a, "Constants.xaml", dict(r##"<FontFamily x:Key="FontFamily">Comfy</FontFamily>"##).as_bytes());
    write(&a, "Fonts/Comfy-Regular.ttf", &fake_font("Comfy", "Regular"));
    write(&a, "Fonts/Comfy-Bold.ttf", &fake_font("Comfy", "Bold"));
    let rep = import(&a, &tmp.path().join("themes")).unwrap();
    let f = ThemeFile::parse(&std::fs::read_to_string(rep.dir.join("theme.toml")).unwrap()).unwrap();
    let says = rep.carried.iter().any(|l| l.starts_with("Titles: Titillium Web"));
    let has = f.fonts.title.as_deref() == Some("builtin:titillium-web-bold");
    assert_eq!(says, has, "{:?} / {:?}", rep.carried, f.fonts.title);
    assert_eq!(has, crate::theme::fonts::bundled::has_titillium());
    if !has {
        assert_eq!(f.fonts.title.as_deref(), Some("fonts/Comfy-Bold.ttf"));
    }
}

#[test]
fn family_names_and_shapes() {
    for f in Family::ALL {
        assert_eq!(Family::parse(f.key()), Some(f));
        assert_eq!(Family::parse(f.label()), Some(f));
        assert!(crate::theme::builtin::all().iter().any(|(k, _)| *k == f.builtin()), "{f:?}");
    }
    assert_eq!(Family::for_shape(look::ListShape::Grid), Family::Classic);
    assert_eq!(Family::for_shape(look::ListShape::Row), Family::Row);
    assert_eq!(Family::for_shape(look::ListShape::Carousel), Family::Row);
    assert_eq!(Family::parse("nope"), None);
}

#[test]
fn reimport_replaces_its_own_theme_but_never_someone_elses() {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    desktop_theme(&src);
    let themes = tmp.path().join("themes");
    // A hand-made theme already uses the name.
    write(&themes, "harbour-night/theme.toml", b"[theme]\nname = \"Mine\"\n");
    let a = import(&src, &themes).unwrap();
    assert_eq!(a.key, "harbour-night-playnite");
    let b = import(&src, &themes).unwrap();
    assert_eq!(b.key, a.key, "replaced, not duplicated");
    assert_eq!(std::fs::read_to_string(themes.join("harbour-night/theme.toml")).unwrap(), "[theme]\nname = \"Mine\"\n");
    // The same name in fullscreen mode is a different theme.
    let fs_src = tmp.path().join("fs");
    desktop_theme(&fs_src);
    write(&fs_src, "theme.yaml", b"Name: Harbour Night\nMode: Fullscreen\n");
    let c2 = import(&fs_src, &themes).unwrap();
    assert_eq!(c2.key, "harbour-night-playnite-fullscreen");
    // No scratch folders left behind.
    let leftovers: Vec<_> = std::fs::read_dir(&themes)
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
        .collect();
    assert!(leftovers.is_empty());
    // A built-in's name is never shadowed.
    let lod = tmp.path().join("lod");
    write(&lod, "theme.yaml", b"Name: Lodestar\n");
    write(&lod, "Constants.xaml", dict("").as_bytes());
    assert_eq!(import(&lod, &themes).unwrap().key, "lodestar-playnite");
}

#[test]
fn colours_the_theme_leaves_out_come_from_playnites_defaults() {
    let src = tempfile::tempdir().unwrap();
    let themes = tempfile::tempdir().unwrap();
    write(src.path(), "theme.yaml", b"Name: Only Text\n");
    write(src.path(), "Constants.xaml", dict(r#"<Color x:Key="TextColor">#FFFFEEDD</Color>"#).as_bytes());
    let rep = import(src.path(), themes.path()).unwrap();
    let f = ThemeFile::parse(&std::fs::read_to_string(rep.dir.join("theme.toml")).unwrap()).unwrap();
    assert_eq!(f.palette.text, c("#ffeedd"));
    assert_eq!(f.palette.accent, c("#55cdff"), "Playnite's default GlyphColor");
    assert!(rep.carried[0].contains("From the theme: text"), "{:?}", rep.carried);
    assert!(!rep.missed.iter().any(|m| m.contains("own screens")), "a colours-only theme has no layout to lose");
}

#[test]
fn light_themes_stay_readable() {
    let src = tempfile::tempdir().unwrap();
    let themes = tempfile::tempdir().unwrap();
    write(src.path(), "theme.yaml", b"Name: Paper\n");
    // Dark text, light popups, but the (inherited) window background is Playnite's dark blue.
    write(
        src.path(),
        "Constants.xaml",
        dict(
            r##"<Color x:Key="TextColor">#080808</Color>
            <Color x:Key="TextColorDarker">#555555</Color>
            <Color x:Key="GlyphColor">#804000</Color>
            <Color x:Key="PopupBackgroundColor">#e8e1d9</Color>
            <Color x:Key="GridItemBackgroundColor">#eae1c2</Color>
            <Color x:Key="MainColor">#ffa040</Color>
            <SolidColorBrush x:Key="PopupBackgroundBrush" Color="{DynamicResource PopupBackgroundColor}" />"##,
        )
        .as_bytes(),
    );
    let rep = import(src.path(), themes.path()).unwrap();
    let p = ThemeFile::parse(&std::fs::read_to_string(rep.dir.join("theme.toml")).unwrap()).unwrap().palette;
    assert!(!p.dark);
    assert_eq!(p.background, c("#e8e1d9"));
    for (what, x) in [("background", p.background), ("panels", p.surface), ("cards", p.surface_alt)] {
        assert!(map::contrast(p.text, x) >= 3.0, "{what} {x:?} vs text");
    }
}

#[test]
fn folders_and_packages_that_are_not_themes_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(tmp.path().join("empty")).unwrap();
    let e = import(&tmp.path().join("empty"), &tmp.path().join("themes")).unwrap_err();
    assert!(format!("{e:#}").contains("theme.yaml"), "{e:#}");
    let zip = tmp.path().join("x.pthm");
    write_zip(&zip, &[("readme.txt", b"hi", false)]);
    assert!(import(&zip, &tmp.path().join("themes")).is_err());
    std::fs::write(tmp.path().join("junk.pthm"), b"not a zip").unwrap();
    assert!(import(&tmp.path().join("junk.pthm"), &tmp.path().join("themes")).is_err());
    assert!(
        !tmp.path().join("themes").exists() || std::fs::read_dir(tmp.path().join("themes")).unwrap().next().is_none()
    );
}

#[test]
fn a_theme_one_level_down_and_a_package_with_a_folder_inside_are_found() {
    let tmp = tempfile::tempdir().unwrap();
    desktop_theme(&tmp.path().join("download/Harbour"));
    let rep = import(&tmp.path().join("download"), &tmp.path().join("themes")).unwrap();
    assert_eq!(rep.name, "Harbour Night");
    let zip = tmp.path().join("nested.pthm");
    let consts = dict(r#"<Color x:Key="GlyphColor">#FF00FF00</Color>"#);
    write_zip(
        &zip,
        &[("Nested/theme.yaml", b"Name: Nested\n", false), ("Nested/Constants.xaml", consts.as_bytes(), true)],
    );
    let rep = import(&zip, &tmp.path().join("themes")).unwrap();
    let f = ThemeFile::parse(&std::fs::read_to_string(rep.dir.join("theme.toml")).unwrap()).unwrap();
    assert_eq!(f.palette.accent, c("#00ff00"));
}

#[test]
fn xaml_paths_resolve_to_theme_files() {
    let files: Vec<(String, u64)> =
        ["Images/bg.jpg", "Images/Other/bg2.png", "Fonts/a.ttf"].iter().map(|f| (f.to_string(), 1)).collect();
    for raw in [
        "{ThemeFile 'Images/bg.jpg'}",
        "{ThemeFile Images\\bg.jpg}",
        "../Images/bg.jpg",
        "pack://siteoforigin:,,,/Themes/Fullscreen/My Theme/Images/bg.jpg",
        "pack://application:,,,/Theme;component/Images/bg.jpg",
        "images/BG.JPG",
    ] {
        assert_eq!(resolve_file(&files, raw).as_deref(), Some("Images/bg.jpg"), "{raw}");
    }
    assert_eq!(resolve_file(&files, "bg2.png").as_deref(), Some("Images/Other/bg2.png"));
    assert_eq!(resolve_file(&files, "missing.png"), None);
}

#[test]
fn manifest_lines() {
    let m =
        parse_manifest("\u{feff}Id: X_1\nName: \"Quoted Name\"\n# comment\nMode: Fullscreen\nThemeApiVersion: 2.0.0");
    assert_eq!(m[0], ("Id".into(), "X_1".into()));
    assert_eq!(m[1], ("Name".into(), "Quoted Name".into()));
    assert_eq!(m[2], ("Mode".into(), "Fullscreen".into()));
}

#[test]
fn finds_downloaded_packages_and_themes_in_wine_prefixes() {
    let home = tempfile::tempdir().unwrap();
    let h = home.path();
    fullscreen_package(&h.join("Downloads/Ember_77.pthm"));
    std::fs::write(h.join("Downloads/notes.txt"), "x").unwrap();
    let roaming = "drive_c/users/steamuser/AppData/Roaming/Playnite/Themes";
    desktop_theme(&h.join(format!(".local/share/Steam/steamapps/compatdata/123/pfx/{roaming}/Desktop/Harbour")));
    write(
        &h.join(format!(".wine/{roaming}/Desktop/Default")),
        "theme.yaml",
        b"Id: Playnite_builtin_DefaultDesktop\nName: Default\n",
    );
    let found = discover(h);
    let names: Vec<(&str, Mode)> = found.iter().map(|f| (f.name.as_str(), f.mode)).collect();
    assert_eq!(names, vec![("Ember", Mode::Fullscreen), ("Harbour Night", Mode::Desktop)]);
    assert!(discover(&h.join("nowhere")).is_empty());
}

#[test]
fn classic_imports_open_on_all_games_over_the_games_picture() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("Ember_77.pthm");
    fullscreen_package(&pkg);
    let rep =
        import_with(&pkg, &tmp.path().join("themes"), &ImportOptions { family: Some(Family::Classic), media: None })
            .unwrap();
    assert_eq!(rep.family, Some(Family::Classic));
    let f = ThemeFile::parse(&std::fs::read_to_string(rep.dir.join("theme.toml")).unwrap()).unwrap();
    // Playnite Fullscreen opens on its game list.
    assert_eq!(f.theme.start, "games");
    // Playnite's default Main has PART_ImageBackground: the selected game's picture, darkened,
    // over the theme's own picture and under the grid.
    let t = raw(&rep.dir);
    let pic = el(&t, "games", "pn_game_picture").expect("game picture");
    assert_eq!(pic["path"].as_str(), Some("{art.hero}"));
    let z = |n: &str| el(&t, "games", n).unwrap()["z"].as_float().unwrap();
    assert!(z("pn_theme_picture") < z("pn_game_picture"));
    assert!(z("pn_game_picture") < z("pn_game_picture_dim") && z("pn_game_picture_dim") < z("pn_list"));
    assert!(el(&t, "collections", "pn_game_picture").is_none());
    // The built-in Top Bar keeps opening on the collections.
    let builtin = crate::theme::builtin::all().into_iter().find(|(n, _)| *n == "Top Bar").unwrap().1;
    assert!(builtin.theme.start.is_empty());
}

#[test]
fn desktop_panels_stay_quiet_and_tiles_are_not_the_window_picture() {
    let src = tempfile::tempdir().unwrap();
    let themes = tempfile::tempdir().unwrap();
    write(src.path(), "theme.yaml", b"Name: Tiles\nThemeApiVersion: 2.6.0\n");
    write(
        src.path(),
        "Constants.xaml",
        dict(
            r##"<Color x:Key="TextColor">#FFF0F0F0</Color>
            <SolidColorBrush x:Key="TextBrush" Color="{DynamicResource TextColor}" />
            <SolidColorBrush x:Key="WindowBackgourndBrush" Color="#FF121212" />
            <SolidColorBrush x:Key="GridItemBackgroundBrush" Color="#FF1E90FF" />
            <SolidColorBrush x:Key="PopupBackgroundBrush" Color="#FF202224" />
            <ImageBrush x:Key="DetailsViewPanelBackgroundBrush" ImageSource="{ThemeFile 'Images/noise_background.png'}"
              Opacity="0.1" TileMode="Tile" Viewport="0,0,100,100" ViewportUnits="Absolute" />"##,
        )
        .as_bytes(),
    );
    write(src.path(), "Images/noise_background.png", &[0x89u8; 4096]);
    let rep = import(src.path(), themes.path()).unwrap();
    let f = ThemeFile::parse(&std::fs::read_to_string(rep.dir.join("theme.toml")).unwrap()).unwrap();
    // Panels: the popup colour, not the accent-blue cover tiles.
    assert_eq!(f.palette.background, c("#121212"));
    assert_eq!(f.palette.surface, c("#202224"));
    // A small tiled texture whose key says "Background" isn't stretched behind the window.
    assert!(f.desktop.background_picture.is_none());
    assert!(!rep.dir.join("images").exists());
}

#[test]
fn a_fullscreen_theme_is_translated_from_its_own_views() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("Ember_77.pthm");
    fullscreen_package(&pkg);
    let rep = import(&pkg, &tmp.path().join("themes")).unwrap();
    assert_eq!((rep.family, rep.suggested), (Some(Family::Translated), Some(Family::Translated)));
    assert_eq!(rep.layout, "Translated from the theme");
    let carried = rep.carried.join("\n");
    assert!(carried.contains("translated from the theme"), "{carried}");
    assert!(carried.contains("Main screen:") && carried.contains("Game details page:"), "{carried}");
    let f = ThemeFile::parse(&std::fs::read_to_string(rep.dir.join("theme.toml")).unwrap()).unwrap();
    // Playnite's main screen first: its game list where Playnite's Main puts it.
    assert_eq!(f.theme.start, "games");
    // Both translated views are scaled as one piece, like Playnite's 1920 × 1080 Viewbox.
    assert_eq!(f.games.viewbox, Some([1920.0, 1080.0]));
    assert_eq!(f.details.as_ref().unwrap().viewbox, Some([1920.0, 1080.0]));
    assert!(f.collections.viewbox.is_none(), "Lodestar's own screens stretch as before");
    assert!(rep.translatable);
    assert!(f.home.is_none());
    let list = f.games.elements.iter().find(|e| e.common().name == "pn_list").expect("list");
    assert!(matches!(list, Element::Grid { .. }));
    assert!((list.common().pos[1] - 110.0 / 1080.0).abs() < 0.002);
    // The theme's window brush (a picture here) is copied for the translated view.
    let t = raw(&rep.dir);
    let games = t["games"]["elements"].as_array().unwrap();
    assert!(games.iter().any(|e| e.get("path").and_then(|p| p.as_str()) == Some("images/view/Images_Backdrop.png")));
    assert!(rep.dir.join("images/view/Images_Backdrop.png").is_file());
    // The extracted cover focus look is on the compiled list.
    let lt = games.iter().find(|e| e.get("name").and_then(|n| n.as_str()) == Some("pn_list")).unwrap();
    assert!(lt.get("selected_border").is_some());
    // The details page is the theme's GameDetails (Playnite's default here), with hints.
    let d = f.details.as_ref().unwrap();
    assert!(d.elements.iter().any(|e| matches!(e, Element::HelpBar { .. })));
    assert!(d.elements.iter().any(|e| matches!(e, Element::Image { path, .. } if path == "{art.cover}")));
    // A layout family on request, and the translation offered again.
    let classic =
        import_with(&pkg, &tmp.path().join("themes"), &ImportOptions { family: Some(Family::Classic), media: None })
            .unwrap();
    assert_eq!(classic.key, rep.key);
    assert_eq!(classic.suggested, Some(Family::Translated));
    assert!(Family::ALL.contains(&Family::Translated));
    assert_eq!(Family::parse("translated"), Some(Family::Translated));
}
