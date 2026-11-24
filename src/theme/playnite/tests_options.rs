//! Theme options and banners through a whole import (hand-made fixture theme, no third-party
//! theme files).

use super::*;
use crate::theme::Color;

const NS: &str = r#"xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation" xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml" xmlns:sys="clr-namespace:System;assembly=mscorlib""#;

fn dict(body: &str) -> String {
    format!("<ResourceDictionary {NS}>\n{body}\n</ResourceDictionary>")
}

fn write(dir: &Path, rel: &str, data: &[u8]) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, data).unwrap();
}

fn png(rgb: [u8; 3]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(10, 2, image::Rgba([rgb[0], rgb[1], rgb[2], 255]));
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

/// A desktop theme with options: an accent colour, a banner switch (the usual Tag + trigger
/// pattern), a darken slider nothing reads and a label.
fn theme(dir: &Path) {
    write(dir, "theme.yaml", b"Id: Opt_1\nName: Options Test\nAuthor: Tester\nVersion: 1.0\nMode: Desktop\n");
    write(
        dir,
        "Constants.xaml",
        dict(
            r##"<Color x:Key="GlyphColor">#FF2A9D8F</Color>
            <SolidColorBrush x:Key="GlyphBrush" Color="{DynamicResource GlyphColor}" />
            <sys:Boolean x:Key="ShowBanners">True</sys:Boolean>
            <sys:Double x:Key="UnusedDarken">0.4</sys:Double>
            <sys:String x:Key="LOCPlayGame">Launch</sys:String>"##,
        )
        .as_bytes(),
    );
    write(
        dir,
        "DerivedStyles/GridViewItemTemplate.xaml",
        dict(
            r#"<DataTemplate x:Key="GridViewItemTemplate"><StackPanel>
            <ContentControl x:Name="ThemeExtras_Banner" Tag="{DynamicResource ShowBanners}"
              Visibility="{Binding Tag, RelativeSource={RelativeSource Self}, Converter={StaticResource BooleanToVisibilityConverter}}" />
            <Image Name="PART_ImageCover" /></StackPanel></DataTemplate>"#,
        )
        .as_bytes(),
    );
    write(
        dir,
        "thememodifier.yaml",
        b"Constants:\n    - \"Colours\"\n    - GlyphBrush: 'Accent'\n    - \"Covers\"\n    - ShowBanners: 'Show banners'\n    - UnusedDarken(0.0,1.0): 'Darken'\n    - LOCPlayGame: 'Play label'\n",
    );
    write(
        dir,
        "themeExtras.yaml",
        b"ThemeId: Opt_1\nBannersByPluginIdPath: Images/Banners/PluginId\nBannersBySpecIdPath: Images/Banners/PlatformSpecId\nDefaultBannerPath: Images/Banners/Unknown.png\n",
    );
    write(dir, "Images/Banners/PluginId/cb91dfc9-b977-43bf-8e70-55f46e410fab.png", &png([20, 60, 200]));
    write(dir, "Images/Banners/PlatformSpecId/nintendo_64.png", &png([200, 20, 20]));
    write(dir, "Images/Banners/Unknown.png", &png([90, 90, 90]));
    write(dir, "Images/Background.jpg", &png([1, 2, 3]));
}

fn accent(dir: &Path) -> Color {
    ThemeFile::parse(&std::fs::read_to_string(dir.join("theme.toml")).unwrap()).unwrap().palette.accent
}

#[test]
fn options_are_offered_applied_and_kept() {
    let src = tempfile::tempdir().unwrap();
    let themes = tempfile::tempdir().unwrap();
    theme(src.path());
    let rep = import(src.path(), themes.path()).unwrap();
    let dir = rep.dir.clone();
    // The accent and the banner switch change what Lodestar draws; the slider and the label don't.
    let spec = options::read(&dir).expect("options");
    let eff: Vec<&str> = spec.effective().map(|o| o.key.as_str()).collect();
    assert_eq!(eff, ["GlyphBrush", "ShowBanners"]);
    assert_eq!(spec.others(), 2);
    assert_eq!(rep.options, 2);
    assert!(rep.groups.iter().any(|g| g.title == "Theme options"), "{:?}", rep.groups);
    // The theme's own files are kept to apply options later (no package needed).
    assert!(dir.join(options::SNAPSHOT).join("Constants.xaml").is_file());
    assert!(dir.join(options::SNAPSHOT).join("Images/Banners/Unknown.png").is_file());
    assert_eq!(accent(&dir), Color([0x2a, 0x9d, 0x8f, 255]));
    // Banners for Steam, the Nintendo 64 and everything else.
    let b = extras::Banners::load(&dir).unwrap();
    assert!(b.shown);
    assert_eq!(b.image("steam", None).as_deref(), Some("images/banners/source/steam.png"));
    assert_eq!(b.image("emulator", Some("Nintendo 64")).as_deref(), Some("images/banners/platform/Nintendo 64.png"));
    assert_eq!(b.image("lutris", None).as_deref(), Some("images/banners/default.png"));

    // Apply options without the original folder; an install record survives.
    std::fs::write(dir.join(crate::theme::catalog::RECORD), "x = 1\n").unwrap();
    let original_source = rep.source.clone();
    drop(src);
    let values: BTreeMap<String, String> =
        [("GlyphBrush".to_string(), "#FFFF0000".to_string()), ("ShowBanners".to_string(), "False".to_string())].into();
    let rep2 = apply_options(themes.path(), &rep.key, &values).unwrap();
    assert_eq!(rep2.key, rep.key);
    assert_eq!(rep2.source, original_source);
    assert_eq!(accent(&dir), Color([255, 0, 0, 255]));
    assert!(!extras::Banners::load(&dir).unwrap().shown);
    assert!(dir.join(crate::theme::catalog::RECORD).is_file());
    let spec = options::read(&dir).unwrap();
    assert_eq!(spec.values, values);
    // The switch keeps its effect from the other side too (marked again after the quick apply).
    assert_eq!(spec.effective().count(), 2);
    assert!(!refresh_option_effects(themes.path(), &rep.key).unwrap());
    assert_eq!(options::read(&dir).unwrap().effective().count(), 2);

    // Another import of the same theme keeps the chosen values; resetting clears them.
    let src = tempfile::tempdir().unwrap();
    theme(src.path());
    let rep3 = import(src.path(), themes.path()).unwrap();
    assert_eq!(rep3.option_values, values);
    assert_eq!(accent(&dir), Color([255, 0, 0, 255]));
    apply_options(themes.path(), &rep.key, &BTreeMap::new()).unwrap();
    assert_eq!(accent(&dir), Color([0x2a, 0x9d, 0x8f, 255]));
    assert!(options::read(&dir).unwrap().values.is_empty());
    // Values that aren't options, or equal the theme's own, are dropped.
    let junk: BTreeMap<String, String> =
        [("Nope".to_string(), "1".to_string()), ("ShowBanners".to_string(), "True".to_string())].into();
    let r = apply_options(themes.path(), &rep.key, &junk).unwrap();
    assert!(r.option_values.is_empty());
}

#[test]
fn themes_without_options_keep_no_copy() {
    let src = tempfile::tempdir().unwrap();
    let themes = tempfile::tempdir().unwrap();
    theme(src.path());
    std::fs::remove_file(src.path().join("thememodifier.yaml")).unwrap();
    let rep = import(src.path(), themes.path()).unwrap();
    assert!(!rep.dir.join(options::SNAPSHOT).exists());
    assert!(options::read(&rep.dir).is_none());
    assert_eq!(rep.options, 0);
    assert!(apply_options(themes.path(), &rep.key, &BTreeMap::new()).is_ok(), "falls back to the original folder");
    drop(src);
    assert!(apply_options(themes.path(), &rep.key, &BTreeMap::new()).is_err());
}

#[test]
fn static_tag_triggers_follow_the_theme_constants() {
    use super::tree::{Texts, Tree};
    let xaml = dict(
        r#"<sys:Boolean x:Key="Round">False</sys:Boolean>
        <Style x:Key="S" TargetType="Border">
          <Setter Property="CornerRadius" Value="0" />
          <Style.Triggers>
            <DataTrigger Binding="{Binding RelativeSource={RelativeSource Self}, Path=Tag}" Value="True">
              <Setter Property="CornerRadius" Value="8" />
            </DataTrigger>
            <DataTrigger Binding="{Binding Source={StaticResource Round}}" Value="False">
              <Setter Property="Opacity" Value="0.5" />
            </DataTrigger>
            <DataTrigger Binding="{Binding IsInstalled}" Value="False">
              <Setter Property="Opacity" Value="0.1" />
            </DataTrigger>
          </Style.Triggers>
        </Style>
        <DataTemplate x:Key="T"><Grid>
          <Border x:Name="Mask" Tag="{DynamicResource Round}" />
          <Border x:Name="A" Style="{StaticResource S}" Tag="{DynamicResource Round}" />
          <Border x:Name="B" Style="{StaticResource S}" Tag="True" />
          <TextBlock x:Name="C">
            <TextBlock.Style><Style TargetType="TextBlock">
              <Setter Property="Visibility" Value="Visible" />
              <Style.Triggers><DataTrigger Binding="{Binding ElementName=Mask, Path=Tag}" Value="False">
                <Setter Property="Visibility" Value="Collapsed" /></DataTrigger></Style.Triggers>
            </Style></TextBlock.Style>
          </TextBlock>
        </Grid></DataTemplate>"#,
    );
    let texts = Texts::with_theme(Mode::Desktop, vec![("Constants.xaml".into(), xaml)]);
    let tree = Tree::build(&texts, &[]);
    let tpl = tree.resource("T").unwrap();
    let get = |name: &str, prop: &str| tree.get(tree.named(tpl, name).unwrap(), prop);
    assert_eq!(get("A", "CornerRadius").and_then(|v| v.max_side()), Some(0.0));
    assert_eq!(get("B", "CornerRadius").and_then(|v| v.max_side()), Some(8.0));
    // The constant-bound trigger applies; the game-bound one (runtime) doesn't.
    assert_eq!(get("A", "Opacity").and_then(|v| v.num()), Some(0.5));
    assert_eq!(get("C", "Visibility").and_then(|v| v.text().map(str::to_string)).as_deref(), Some("Collapsed"));
}
