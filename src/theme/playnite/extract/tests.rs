//! Hand-written XAML fixtures that imitate the structures Playnite themes use (no theme code).

use super::*;
use crate::theme::playnite::tree::tests::dict;

fn c(hex: &str) -> Color {
    Color::parse(hex).unwrap()
}

fn look(mode: Mode, files: &[(&str, &str)], sizes: &[(&str, u64)], modifier: Option<&str>) -> PlayniteLook {
    let texts = Texts::with_theme(mode, files.iter().map(|(a, b)| (a.to_string(), dict(b))).collect());
    let mut list: Vec<(String, u64)> = files.iter().map(|(f, b)| (f.to_string(), b.len() as u64)).collect();
    list.extend(sizes.iter().map(|(f, s)| (f.to_string(), *s)));
    let tree = Tree::build(&texts, &list);
    look_of(&tree, modifier)
}

#[test]
fn playnite_default_fullscreen_look() {
    let l = look(Mode::Fullscreen, &[], &[], None);
    // ListGameItemStyle: a 5 px SelectionBrush frame 5 px outside the cover, radius 3, an
    // orange glow (blur 20), focus scale 1.06; a 3 px ControlBackgroundBrush frame on every item.
    let f = l.item.focus_frame.clone().unwrap();
    assert_eq!((f.width, f.radius, f.outset), (5.0, 3.0, 5.0));
    assert!(matches!(f.brush, Some(LookBrush::Linear { .. })));
    let g = l.item.focus_glow.clone().unwrap();
    assert_eq!((g.color, g.blur, g.depth), (c("#ec6200"), 20.0, 0.0));
    assert_eq!(l.item.focus_scale, Some(1.06));
    assert_eq!(l.item.frame.as_ref().map(|f| (f.width, f.radius)), Some((3.0, 3.0)));
    // ListGameItemTemplate: its title strip follows "show game titles" and its darkening
    // "darken games that aren't installed", both off in Playnite's defaults.
    assert_eq!(l.item.caption, None);
    assert_eq!(l.item.uninstalled_scrim, None);
    // Main: 110 px top bar, 100 px bottom bar (both TopBarBackgroundBrush, angled), radial
    // background, picture over all rows, clock + battery, a cover grid.
    let top = l.main.top_bar.clone().unwrap();
    assert_eq!(top.height, 110.0);
    let Some(LookBrush::Linear { start, end, stops, .. }) = top.background else { panic!("{:?}", l.main.top_bar) };
    assert_eq!((start, end, stops.len()), ([-0.5, 0.5], [1.0, 0.5], 2));
    assert_eq!(l.main.bottom_bar.as_ref().map(|b| b.height), Some(100.0));
    assert!(matches!(l.main.background, Some(LookBrush::Radial { center: [0.5, 0.2], .. })));
    let pic = l.main.picture.clone().unwrap();
    assert!(pic.full);
    assert_eq!((pic.halign, pic.valign), (Some(Align::Center), Some(Align::Start)));
    assert!(l.main.clock && l.main.battery && l.main.battery_percent && l.main.filter_presets);
    assert_eq!(l.main.list.shape, ListShape::Grid);
    assert_eq!(l.main.list.margin, Some([25.0, 0.0, 25.0, 0.0]));
    let area = l.main.list.area.unwrap();
    assert!((area[1] - 110.0 / 1080.0).abs() < 1e-4, "{area:?}");
    assert!((area[3] - 870.0 / 1080.0).abs() < 1e-4, "{area:?}");
    // GameDetails: overlay gradient, cover left of the text, bold Titillium title.
    assert!(matches!(l.details.overlay, Some(LookBrush::Linear { .. })));
    assert_eq!(l.details.cover_side, Some(Side::Left));
    let t = l.details.title_font.clone().unwrap();
    assert_eq!((t.family.as_deref(), t.weight), (Some("Titillium Web"), 700));
    assert_eq!(l.details.title_size, Some(38.0));
    assert!(l.details.title_shadow.is_some());
    assert_eq!(l.details.description_size, Some(16.0));
    assert!(!l.details.logo);
    let body = l.fonts.body.clone().unwrap();
    assert_eq!(
        (body.family.as_deref(), body.weight, body.key.as_deref()),
        (Some("Titillium Web"), 300, Some("FontTitilliumWebLight"))
    );
    assert_eq!((l.font_sizes.small, l.font_sizes.normal), (Some(18.0), Some(22.0)));
    assert!(l.media.videos.is_empty() && l.media.music.is_empty());
}

const ROW_MAIN: &str = r##"
<LinearGradientBrush x:Key="BarBrush" StartPoint="0,0" EndPoint="0,1">
  <GradientStop Color="#FF102040" Offset="0"/>
  <GradientStop Color="#80102040" Offset="0.6"/>
  <GradientStop Color="#00102040" Offset="1"/>
</LinearGradientBrush>
<Style TargetType="{x:Type Main}">
  <Setter Property="Template">
    <Setter.Value>
      <ControlTemplate TargetType="{x:Type Main}">
        <Grid x:Name="PART_MainHost">
          <Grid x:Name="PART_ViewHost" Background="#FF000814">
            <Grid.RowDefinitions>
              <RowDefinition Height="90" />
              <RowDefinition Height="*" />
              <RowDefinition Height="Auto" />
            </Grid.RowDefinitions>
            <FadeImage x:Name="PART_ImageBackground" Grid.RowSpan="3" Stretch="UniformToFill" Opacity="0.7">
              <FadeImage.OpacityMask>
                <LinearGradientBrush StartPoint="0.5,0" EndPoint="0.5,1">
                  <GradientStop Color="Black" Offset="0.4"/>
                  <GradientStop Color="Transparent" Offset="1"/>
                </LinearGradientBrush>
              </FadeImage.OpacityMask>
              <FadeImage.Effect><BlurEffect Radius="12" /></FadeImage.Effect>
            </FadeImage>
            <Border Grid.RowSpan="3" Background="#66000000" />
            <DockPanel Grid.Row="0" Background="{DynamicResource BarBrush}">
              <TextBlock x:Name="PART_TextClock" DockPanel.Dock="Right" />
              <ContentControl x:Name="PART_ElemBatteryStatus" Visibility="Collapsed" />
            </DockPanel>
            <ListBoxEx x:Name="PART_ListGameItems" Grid.Row="1" Height="420" Margin="60,20,0,0"
                       VerticalAlignment="Bottom">
              <ListBoxEx.ItemsPanel>
                <ItemsPanelTemplate><VirtualizingStackPanel Orientation="Horizontal" /></ItemsPanelTemplate>
              </ListBoxEx.ItemsPanel>
            </ListBoxEx>
            <StackPanel Grid.Row="2" Height="80" Orientation="Horizontal">
              <ButtonEx x:Name="PART_ButtonPlay" />
            </StackPanel>
            <ExtraMetadataLoader_LogoLoaderControl />
          </Grid>
        </Grid>
      </ControlTemplate>
    </Setter.Value>
  </Setter>
</Style>"##;

const ROW_ITEM: &str = r##"
<SolidColorBrush x:Key="FocusRing" Color="#FFFFFFFF" />
<CornerRadius x:Key="CoverRound">12</CornerRadius>
<Style x:Key="ListGameItemStyle" TargetType="ListBoxItem">
  <Setter Property="Template">
    <Setter.Value>
      <ControlTemplate TargetType="ListBoxItem">
        <ControlTemplate.Resources>
          <Storyboard x:Key="Grow">
            <DoubleAnimation Storyboard.TargetName="Zoom" Storyboard.TargetProperty="ScaleX" To="1.0" />
            <DoubleAnimation Storyboard.TargetName="Zoom" Storyboard.TargetProperty="ScaleY" To="1.0" />
            <DoubleAnimation Storyboard.TargetName="Caption" Storyboard.TargetProperty="Opacity" To="1" />
          </Storyboard>
        </ControlTemplate.Resources>
        <Grid Margin="8,0,8,0" Width="300" Height="400" Opacity="0.8">
          <Grid.RenderTransform><ScaleTransform x:Name="Zoom" ScaleX="0.9" ScaleY="0.9" /></Grid.RenderTransform>
          <ContentPresenter />
          <Border x:Name="Ring" BorderThickness="4" BorderBrush="{DynamicResource FocusRing}"
                  CornerRadius="{DynamicResource CoverRound}" Margin="-4" Visibility="Collapsed">
            <Border.Effect><DropShadowEffect ShadowDepth="0" BlurRadius="25" Color="#FF55CCFF" Opacity="0.8"/></Border.Effect>
          </Border>
          <Border x:Name="Caption" Opacity="0" VerticalAlignment="Bottom" Background="#CC000000" Height="60">
            <TextBlock Text="{Binding DisplayName}" FontSize="{DynamicResource FontSizeSmall}" TextAlignment="Left" />
          </Border>
        </Grid>
        <ControlTemplate.Triggers>
          <Trigger Property="IsSelected" Value="True">
            <Setter Property="Visibility" Value="Visible" TargetName="Ring" />
            <Trigger.EnterActions><BeginStoryboard Storyboard="{StaticResource Grow}" /></Trigger.EnterActions>
          </Trigger>
        </ControlTemplate.Triggers>
      </ControlTemplate>
    </Setter.Value>
  </Setter>
</Style>
<Style x:Key="ListGameItemTemplate" TargetType="{x:Type GameListItem}">
  <Setter Property="Template">
    <Setter.Value>
      <ControlTemplate TargetType="{x:Type GameListItem}">
        <Grid>
          <Border CornerRadius="{StaticResource CoverRound}" ClipToBounds="True">
            <Image x:Name="PART_ImageCover" Stretch="UniformToFill" />
          </Border>
          <Border x:Name="Dim" Background="#FF000000" Opacity="0.8" Visibility="Collapsed" />
        </Grid>
        <ControlTemplate.Triggers>
          <DataTrigger Binding="{Binding IsInstalled}" Value="False">
            <Setter Property="Visibility" Value="Visible" TargetName="Dim" />
          </DataTrigger>
        </ControlTemplate.Triggers>
      </ControlTemplate>
    </Setter.Value>
  </Setter>
</Style>"##;

const DETAILS: &str = r##"
<Style TargetType="{x:Type GameDetails}">
  <Setter Property="Template">
    <Setter.Value>
      <ControlTemplate TargetType="{x:Type GameDetails}">
        <Grid x:Name="PART_ViewHost" Background="#FF000000">
          <Grid.ColumnDefinitions><ColumnDefinition Width="*" /><ColumnDefinition Width="400" /></Grid.ColumnDefinitions>
          <FadeImage x:Name="PART_ImageBackground" Grid.ColumnSpan="2" />
          <StackPanel Grid.Column="0">
            <ContentControl x:Name="ExtraMetadataLoader_LogoLoaderControl" />
            <TextBlock Text="{Binding DisplayName}" FontFamily="Fonts/#Comfortaa" FontWeight="SemiBold" FontSize="44" />
            <TextBlock x:Name="PART_TextDescription" FontSize="20" />
          </StackPanel>
          <Image x:Name="PART_ImageCover" Grid.Column="1" />
        </Grid>
      </ControlTemplate>
    </Setter.Value>
  </Setter>
</Style>"##;

const MEDIA: &str = r##"
<Style x:Key="Unused" TargetType="Grid">
  <Setter Property="Template">
    <Setter.Value>
      <ControlTemplate>
        <Grid>
          <MediaElement Source="{ThemeFile 'Videos/Loop.mp4'}" MediaElementBehaviors.Repeat="True" IsMuted="True" Stretch="UniformToFill" />
          <MediaElement Source="{ThemeFile 'Videos/Intro.mp4'}" />
          <MediaElement>
            <MediaElement.Style>
              <Style TargetType="MediaElement">
                <Style.Triggers>
                  <DataTrigger Binding="{Binding IsOpen}" Value="True">
                    <Setter Property="Source" Value="{ThemeFile 'audio/Whoosh.wav'}" />
                  </DataTrigger>
                </Style.Triggers>
              </Style>
            </MediaElement.Style>
          </MediaElement>
          <TextBlock Text="{PluginSettings Plugin=SuccessStory, Path=Count}" />
          <TextBlock Text="{PluginSettings Plugin=SuccessStory, Path=Total}" Tag="{PluginStatus Plugin=ThemeExtras, Status=Installed}" />
          <ContentControl x:Name="ThemeExtras_BannerControl" />
        </Grid>
      </ControlTemplate>
    </Setter.Value>
  </Setter>
</Style>
<sys:Boolean x:Key="FadeBackground">False</sys:Boolean>
<sys:Double x:Key="DarkenAmount">0.35</sys:Double>"##;

const MODIFIER: &str = "Constants:\n    - \"Background\"\n    - FadeBackground: 'Fade background image'\n    - DarkenAmount(0.0,1.0): 'Darken amount'\n    - \"Other\"\n    - Missing: 'Not defined anywhere'\n";

#[test]
fn a_single_row_theme_over_the_default() {
    let l = look(
        Mode::Fullscreen,
        &[
            ("Views/Main.xaml", ROW_MAIN),
            ("DerivedStyles/ListGameItemStyle.xaml", ROW_ITEM),
            ("Views/GameDetails.xaml", DETAILS),
            ("Views/Extras.xaml", MEDIA),
        ],
        &[
            ("audio/background.mp3", 4000),
            ("audio/navigation.wav", 300),
            ("audio/activation.wav", 0),
            ("audio/Whoosh.wav", 50),
            ("Videos/Loop.mp4", 9000),
            ("Videos/Intro.mp4", 100),
            ("Videos/Spare.mp4", 70),
        ],
        Some(MODIFIER),
    );
    let main = &l.main;
    assert_eq!(main.list.shape, ListShape::Row, "{:?}", main.list.evidence);
    assert_eq!(main.list.height, Some(420.0));
    assert_eq!(main.list.valign, Some(Align::End));
    // Top bar: a fixed 90 px row holding the clock, a 3-stop vertical gradient.
    let top = main.top_bar.clone().unwrap();
    assert_eq!(top.height, 90.0);
    let bar = top.background.unwrap();
    assert_eq!(bar.stops().len(), 3);
    assert_eq!(bar.angle(), Some(90.0));
    assert_eq!(main.bottom_bar.as_ref().map(|b| b.height), Some(80.0));
    assert!(main.clock && !main.battery && !main.filter_presets);
    assert_eq!(main.background, Some(LookBrush::solid(c("#000814"))));
    let pic = main.picture.clone().unwrap();
    assert!(pic.full);
    assert_eq!((pic.stretch, pic.opacity, pic.blur), (Some(Stretch::UniformToFill), Some(0.7), Some(12.0)));
    let mask = pic.mask.unwrap();
    assert!(mask.edge_alpha[0] > 0.99 && mask.edge_alpha[2] < 0.01, "{:?}", mask.edge_alpha);
    assert_eq!(pic.darken, Some(c("#00000066")));
    // List area: 420 px high at the bottom of the band between the 90 px and 80 px bars.
    let a = main.list.area.unwrap();
    assert!((a[0] - 60.0 / 1920.0).abs() < 1e-4 && (a[1] - 580.0 / 1080.0).abs() < 1e-4, "{a:?}");
    assert!((a[3] - 420.0 / 1080.0).abs() < 1e-4 && (a[2] - 1860.0 / 1920.0).abs() < 1e-4, "{a:?}");
    // The item: the selection ring (white, radius from a resource, glow), scale relative to 0.9.
    let it = &l.item;
    let f = it.focus_frame.clone().unwrap();
    assert_eq!((f.width, f.radius, f.outset), (4.0, 12.0, 4.0));
    assert_eq!(f.brush, Some(LookBrush::solid(c("#ffffff"))));
    let g = it.focus_glow.clone().unwrap();
    assert_eq!((g.color, g.blur, g.opacity), (c("#55ccff"), 25.0, 0.8));
    assert!((it.focus_scale.unwrap() - 1.0 / 0.9).abs() < 1e-4);
    assert_eq!(it.unfocused_opacity, Some(0.8));
    assert_eq!(it.size, Some([300.0, 400.0]));
    assert_eq!(it.spacing, Some([8.0, 0.0, 8.0, 0.0]));
    assert_eq!(it.cover_radius, Some(12.0));
    assert_eq!(it.uninstalled_scrim, Some(c("#000000cc")));
    assert_eq!(it.frame, None);
    let cap = it.caption.clone().unwrap();
    assert_eq!(cap.shown, Shown::Focus);
    assert_eq!(cap.align, Some(Align::Start));
    assert_eq!(cap.valign, Some(Align::End));
    assert_eq!(cap.background, Some(LookBrush::solid(c("#000000cc"))));
    // Details: cover on the right, the theme's own title font, logo hook.
    let d = &l.details;
    assert_eq!(d.cover_side, Some(Side::Right));
    let t = d.title_font.clone().unwrap();
    assert_eq!((t.value.as_deref(), t.family.as_deref(), t.weight), (Some("Fonts/#Comfortaa"), Some("Comfortaa"), 600));
    assert_eq!(d.title_size, Some(44.0));
    assert_eq!(d.description_size, Some(20.0));
    assert!(d.logo);
    assert_eq!(d.overlay, None);
    // Media: the looping video first, the intro, a sound from the XAML; empty files skipped.
    let m = &l.media;
    assert_eq!(m.music, vec!["audio/background.mp3".to_string()]);
    let roles: Vec<&SoundRole> = m.sounds.iter().map(|s| &s.role).collect();
    assert_eq!(roles, vec![&SoundRole::Navigation, &SoundRole::Other("Whoosh".into())]);
    assert_eq!(m.videos.len(), 2);
    assert_eq!((m.videos[0].file.as_str(), m.videos[0].role), ("Videos/Loop.mp4", VideoRole::Background));
    assert!(m.videos[0].repeat && m.videos[0].muted);
    assert_eq!(m.videos[0].view, "Extras");
    assert_eq!(m.videos[1].role, VideoRole::Intro);
    assert_eq!(m.unused_videos, vec!["Videos/Spare.mp4".to_string()]);
    assert_eq!(m.bytes, 4000 + 300 + 50 + 9000 + 100);
    // Plugins and options.
    assert_eq!(l.plugins.names.first(), Some(&("SuccessStory".to_string(), 2)));
    assert!(l.plugins.names.iter().any(|(n, _)| n == "ThemeExtras"));
    assert!(l.plugins.banners);
    assert_eq!(l.plugins.uses, 3);
    assert_eq!(l.options.len(), 3);
    assert_eq!(l.options[0].section, "Background");
    assert_eq!(l.options[0].kind, OptionKind::Switch { default: Some(false) });
    assert_eq!(l.options[1].kind, OptionKind::Range { min: 0.0, max: 1.0, default: Some(0.35) });
    assert_eq!(l.options[2].kind, OptionKind::Switch { default: None });
    assert_eq!(l.options[2].section, "Other");
    assert!(l.coverage.media_elements >= 3 && l.coverage.storyboards >= 1);
}

#[test]
fn a_carousel_theme() {
    let main = r##"
<Style TargetType="{x:Type Main}">
  <Setter Property="Template">
    <Setter.Value>
      <ControlTemplate TargetType="{x:Type Main}">
        <Grid x:Name="PART_ViewHost">
          <ListBoxEx x:Name="PART_ListGameItems" Opacity="0" />
          <ItemsControl x:Name="DisplayCarousel" ItemsSource="{Binding Items, ElementName=PART_ListGameItems}">
            <ItemsControl.RenderTransform>
              <TranslateTransform X="{Binding SelectedIndex, ElementName=PART_ListGameItems, Converter={StaticResource MathConverter}, ConverterParameter=x*-300}" />
            </ItemsControl.RenderTransform>
          </ItemsControl>
        </Grid>
      </ControlTemplate>
    </Setter.Value>
  </Setter>
</Style>"##;
    let l = look(Mode::Fullscreen, &[("Views/Main.xaml", main)], &[], None);
    assert_eq!(l.main.list.shape, ListShape::Carousel, "{:?}", l.main.list.evidence);
    assert_eq!(l.main.top_bar, None);
    assert!(!l.main.clock);
}

/// A details page that first tints its whole picture faintly, then lays a dark panel over the
/// text's side: the panel is the overlay (it keeps the text readable), not the tint.
#[test]
fn details_overlay_skips_a_faint_tint() {
    let details = r##"
<Style TargetType="{x:Type GameDetails}">
  <Setter Property="Template">
    <Setter.Value>
      <ControlTemplate TargetType="{x:Type GameDetails}">
        <Grid x:Name="PART_ViewHost">
          <FadeImage x:Name="PART_ImageBackground" />
          <Border Background="#22050A0E" />
          <Border>
            <Border.Background>
              <LinearGradientBrush StartPoint="0,0" EndPoint="1,0">
                <GradientStop Color="#D9060B10" Offset="0" />
                <GradientStop Color="#10060B10" Offset="1" />
              </LinearGradientBrush>
            </Border.Background>
          </Border>
        </Grid>
      </ControlTemplate>
    </Setter.Value>
  </Setter>
</Style>"##;
    let l = look(Mode::Fullscreen, &[("Views/GameDetails.xaml", details)], &[], None);
    assert!(matches!(l.details.overlay, Some(LookBrush::Linear { .. })), "{:?}", l.details.overlay);
    // Only the faint tint: it is still the overlay.
    let tint = details.replace("#D9060B10", "#10060B10");
    let tint = tint.split("<Border>").next().unwrap().to_string()
        + "</Grid></ControlTemplate></Setter.Value></Setter></Style>";
    let l = look(Mode::Fullscreen, &[("Views/GameDetails.xaml", &tint)], &[], None);
    assert_eq!(l.details.overlay.and_then(|b| b.average()), Some(c("#050a0e22")));
}

/// A view that starts at Opacity 0 and fades in once shown is not hidden; a list that Playnite's
/// "horizontal layout" setting makes one cover high (through a hidden toggle) is one row.
#[test]
fn fade_in_views_and_a_horizontal_layout_row() {
    let main = r##"
<Style TargetType="{x:Type Main}">
  <Setter Property="Template">
    <Setter.Value>
      <ControlTemplate TargetType="{x:Type Main}">
        <Grid x:Name="PART_ViewHost">
          <Grid.Style>
            <Style>
              <Setter Property="ItemsControl.Opacity" Value="0" />
              <Style.Triggers>
                <DataTrigger Binding="{Binding ElementName=PART_ListGameItems, Path=Visibility}" Value="Visible">
                  <DataTrigger.EnterActions>
                    <BeginStoryboard><Storyboard>
                      <DoubleAnimation Storyboard.TargetProperty="Opacity" From="0" To="1" Duration="0:0:0.5" />
                    </Storyboard></BeginStoryboard>
                  </DataTrigger.EnterActions>
                </DataTrigger>
              </Style.Triggers>
            </Style>
          </Grid.Style>
          <Grid.RowDefinitions>
            <RowDefinition Height="Auto" />
            <RowDefinition Height="*" />
          </Grid.RowDefinitions>
          <Grid Grid.Row="0" Height="90">
            <CheckBoxEx x:Name="LayoutToggle" Visibility="Collapsed" IsChecked="{Settings Fullscreen.HorizontalLayout, Mode=TwoWay}" />
            <TextBlock x:Name="PART_TextClock" />
          </Grid>
          <Grid Grid.Row="1">
            <ListBoxEx x:Name="PART_ListGameItems">
              <ListBox.Style>
                <Style>
                  <Setter Property="ItemsControl.Height" Value="Auto" />
                  <Style.Triggers>
                    <DataTrigger Binding="{Binding ElementName=LayoutToggle, Path=IsChecked}" Value="True">
                      <Setter Property="ItemsControl.Height" Value="420" />
                    </DataTrigger>
                  </Style.Triggers>
                </Style>
              </ListBox.Style>
            </ListBoxEx>
          </Grid>
          <Border x:Name="Hidden" Opacity="0"><TextBlock x:Name="PART_ElemBatteryStatus" /></Border>
        </Grid>
      </ControlTemplate>
    </Setter.Value>
  </Setter>
</Style>"##;
    let l = look(Mode::Fullscreen, &[("Views/Main.xaml", main)], &[], None);
    assert!(l.main.clock);
    assert!(!l.main.battery, "Opacity 0 with nothing fading it in stays hidden");
    assert_eq!(l.main.top_bar.as_ref().map(|b| b.height), Some(90.0));
    assert_eq!(l.main.list.shape, ListShape::Row, "{:?}", l.main.list.evidence);
    // Taller than two covers in the horizontal layout: Playnite's grid.
    let tall = main.replace("Value=\"420\"", "Value=\"900\"");
    let l = look(Mode::Fullscreen, &[("Views/Main.xaml", &tall)], &[], None);
    assert_eq!(l.main.list.shape, ListShape::Grid, "{:?}", l.main.list.evidence);
}

/// A title that a trigger on Playnite's "show game titles" setting makes visible, over a
/// separate strip layer the same trigger shows: decided at Playnite's default (titles off).
#[test]
fn a_caption_shown_by_the_titles_setting() {
    let shown_when_titles = |value: &str| {
        let item = ITEM_WITH_TITLES_TRIGGER.replace("Value=\"True\"", &format!("Value=\"{value}\""));
        look(Mode::Fullscreen, &[("DerivedStyles/ListGameItemStyle.xaml", &item)], &[], None).item.caption
    };
    assert_eq!(shown_when_titles("True"), None);
    // A theme that shows its own title while Playnite's are off.
    let cap = shown_when_titles("False").expect("caption");
    assert_eq!(cap.shown, Shown::Setting);
    assert_eq!(cap.background, Some(LookBrush::solid(c("#10101080"))));
    assert_eq!(cap.font_size, Some(18.0));
}

const ITEM_WITH_TITLES_TRIGGER: &str = r##"
<Style x:Key="ListGameItemStyle" TargetType="ListBoxItem">
  <Setter Property="Template">
    <Setter.Value>
      <ControlTemplate TargetType="ListBoxItem">
        <Grid>
          <ContentPresenter />
          <Border x:Name="Strip" Background="#FF101010" Opacity="0.3" Height="120" VerticalAlignment="Bottom" Visibility="Collapsed" />
          <StackPanel VerticalAlignment="Bottom">
            <TextBlock x:Name="Title" Visibility="Collapsed" Text="{Binding DisplayName}" FontSize="{DynamicResource FontSizeSmall}" />
          </StackPanel>
        </Grid>
        <ControlTemplate.Triggers>
          <DataTrigger Binding="{Settings Fullscreen.ShowGameTitles}" Value="True">
            <Setter Property="Opacity" Value="0.5" TargetName="Strip" />
            <Setter Property="Visibility" Value="Visible" TargetName="Strip" />
            <Setter Property="Visibility" Value="Visible" TargetName="Title" />
          </DataTrigger>
        </ControlTemplate.Triggers>
      </ControlTemplate>
    </Setter.Value>
  </Setter>
</Style>"##;

#[test]
fn playnite_default_desktop_look_and_a_rounded_cover_theme() {
    let d = look(Mode::Desktop, &[], &[], None).desktop;
    assert_eq!(d.control_radius, Some(1.0));
    // The names follow "show names under covers", which Lodestar's own switch stands for; the
    // darkening of games that aren't installed is off in Playnite's defaults.
    assert_eq!(d.cover_title, CoverTitle::Under);
    let sel = d.grid.focus_frame.clone().unwrap();
    assert_eq!(sel.width, 3.0);
    assert_eq!(sel.brush.and_then(|b| b.average()), Some(c("#55cdff")));
    assert_eq!(d.grid.hover_scrim, Some(c("#000000aa")));
    assert_eq!(d.grid.uninstalled_scrim, None);
    assert_eq!(d.grid.cover_radius, None);
    // Its item's GridItemBackgroundBrush is cleared while "show grid item background" is off.
    assert_eq!(d.grid.background, None);
    let Some(LookBrush::Linear { start, end, .. }) = d.window_background.clone() else {
        panic!("{:?}", d.window_background)
    };
    assert_eq!((start, end), ([0.5, -0.2], [0.4, 1.0]));
    assert!(d.panel_separator.is_some());
    let pic = d.details_picture.clone().unwrap();
    assert!(pic.mask.as_ref().is_some_and(|m| m.edge_alpha[2] < 0.5), "{pic:?}");
    let l = look(Mode::Desktop, &[], &[], None);
    assert_eq!(l.font_sizes.largest, Some(29.0));
    assert_eq!(l.fonts.body.as_ref().and_then(|f| f.family.as_deref()), Some("Trebuchet MS"));

    let theme = look(
        Mode::Desktop,
        &[
            (
                "Constants.xaml",
                r##"<CornerRadius x:Key="GridViewCoverCornerRadius">8</CornerRadius>
                <CornerRadius x:Key="ControlCornerRadius">6</CornerRadius>
                <FontFamily x:Key="FontFamily">Segoe UI</FontFamily>
                <ImageBrush x:Key="WindowBackgourndBrush" ImageSource="{ThemeFile 'Images/BackgroundGradient.png'}" Stretch="UniformToFill" />"##,
            ),
            (
                "DerivedStyles/GridViewItemTemplate.xaml",
                r##"<Style x:Key="GridViewItemTemplate" TargetType="{x:Type GameListItem}">
                  <Setter Property="Template"><Setter.Value>
                    <ControlTemplate TargetType="{x:Type GameListItem}">
                      <Grid>
                        <Border CornerRadius="{DynamicResource GridViewCoverCornerRadius}">
                          <Image Name="PART_ImageCover" />
                        </Border>
                        <TextBlock Text="{Binding Name}" VerticalAlignment="Bottom" Background="#99000000" />
                      </Grid>
                    </ControlTemplate>
                  </Setter.Value></Setter>
                </Style>"##,
            ),
        ],
        &[("Images/BackgroundGradient.png", 1000)],
        None,
    );
    let d = theme.desktop;
    assert_eq!(d.grid.cover_radius, Some(8.0));
    assert_eq!(d.control_radius, Some(6.0));
    assert_eq!(d.cover_title, CoverTitle::Overlay);
    assert!(matches!(
        &d.background_picture,
        Some(LookBrush::Image { file: Some(f), stretch: Stretch::UniformToFill, .. }) if f == "Images/BackgroundGradient.png"
    ));
    assert_eq!(theme.fonts.body.and_then(|f| f.value), Some("Segoe UI".into()));
}

/// Opt-in corpus check (`LODESTAR_PLAYNITE_CORPUS=<folder of .pthm packages / theme folders>
/// cargo test --release -- --ignored playnite_corpus`): every theme reads without a panic in
/// under 2 s. Theme files are never part of the repository.
#[test]
#[ignore]
fn playnite_corpus() {
    let Ok(dir) = std::env::var("LODESTAR_PLAYNITE_CORPUS") else {
        return;
    };
    let mut n = 0;
    for e in std::fs::read_dir(&dir).unwrap().flatten() {
        let p = e.path();
        let Ok(src) = Source::open(&p) else { continue };
        let t0 = std::time::Instant::now();
        for mode in [Mode::Desktop, Mode::Fullscreen] {
            let x = extract(&src, mode);
            assert!(x.look.coverage.nodes > 0, "{}", p.display());
        }
        let took = t0.elapsed();
        assert!(took.as_secs_f32() < 2.0, "{} took {took:?}", p.display());
        n += 1;
    }
    eprintln!("{n} themes read");
}

#[test]
fn the_play_button_window_and_panels_of_a_desktop_theme() {
    // The style's Background where the template draws it.
    let own = r##"<Style x:Key="PlayButton" TargetType="{x:Type Button}">
        <Setter Property="Background" Value="#FF238636"/>
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{x:Type Button}">
            <Grid><Border Margin="4,4,-4,-4" Background="#FF1F6FEB"/><Border Background="{TemplateBinding Background}"/></Grid>
          </ControlTemplate>
        </Setter.Value></Setter>
      </Style>"##;
    let d = look(Mode::Desktop, &[("DerivedStyles/PlayButton.xaml", own)], &[], None).desktop;
    assert_eq!(d.play_button.and_then(|b| b.average()), Some(Color([0x23, 0x86, 0x36, 255])));
    // A template that paints its own frame, at rest as its IsMouseOver = False trigger says.
    let frame = r##"<Style x:Key="PlayButton" TargetType="{x:Type Button}">
        <Setter Property="Background" Value="White"/>
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{x:Type Button}">
            <Grid><Border x:Name="Frame" Background="#FF101010"/><ContentPresenter/></Grid>
            <ControlTemplate.Triggers>
              <Trigger Property="IsMouseOver" Value="False">
                <Setter TargetName="Frame" Property="Background" Value="#FF4392B0"/>
              </Trigger>
            </ControlTemplate.Triggers>
          </ControlTemplate>
        </Setter.Value></Setter>
      </Style>"##;
    let d = look(Mode::Desktop, &[("DerivedStyles/PlayButton.xaml", frame)], &[], None).desktop;
    assert_eq!(d.play_button.and_then(|b| b.average()), Some(Color([0x43, 0x92, 0xb0, 255])));
    // Playnite's own Play button isn't the theme's colour.
    assert!(look(Mode::Desktop, &[], &[], None).desktop.play_button.is_none());
    // The window's own type style; a sidebar whose first child is a thin stripe isn't painted
    // with it.
    let window = r##"<Style TargetType="WindowBase"><Setter Property="Background" Value="#FFEBEBEB"/></Style>
      <Style TargetType="{x:Type Sidebar}">
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{x:Type Sidebar}">
            <Grid Background="{x:Null}"><Border Width="2" HorizontalAlignment="Left" Background="#FFFFA000"/></Grid>
          </ControlTemplate>
        </Setter.Value></Setter>
      </Style>"##;
    let d = look(Mode::Desktop, &[("CustomControls/WindowBase.xaml", window)], &[], None).desktop;
    assert_eq!(d.window_background.and_then(|b| b.average()), Some(Color([0xeb, 0xeb, 0xeb, 255])));
    assert!(d.sidebar.is_none(), "{:?}", d.sidebar);
}

/// A Library view template around `body` (its root Grid's children).
fn library(body: &str) -> String {
    format!(
        r##"<Style TargetType="{{x:Type Library}}">
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{{x:Type Library}}">
            <Grid>
              <Grid.RowDefinitions><RowDefinition Height="Auto" /><RowDefinition Height="*" /></Grid.RowDefinitions>
              {body}
            </Grid>
          </ControlTemplate>
        </Setter.Value></Setter>
      </Style>"##
    )
}

#[test]
fn game_art_behind_the_desktop_window() {
    // Playnite's own Library view keeps PART_ImageBackground behind everything: shown in the
    // details view, darkened by the user's 70 % (towards black), nothing over it.
    let d = look(Mode::Desktop, &[], &[], None).desktop;
    let art = d.window_art.expect("Playnite's Library shows the game art");
    assert_eq!(art.picture.darken.map(|c| c.0[3]), Some(179));
    assert!(art.masks.is_empty() && art.layers.is_empty() && !art.below_top_bar);
    // A theme's own layer over the art is drawn over it; its ImageDarkeningBrush isn't
    // (Playnite sets that one from the user's setting).
    let layered = library(
        r##"<FadeImage x:Name="PART_ImageBackground" Grid.RowSpan="2" ImageDarkeningBrush="#FF000000" />
            <Rectangle Grid.RowSpan="2" Opacity="0.5">
              <Rectangle.Fill>
                <LinearGradientBrush StartPoint="0,0" EndPoint="1,0">
                  <GradientStop Color="#FF000000" Offset="0" /><GradientStop Color="#00000000" Offset="1" />
                </LinearGradientBrush>
              </Rectangle.Fill>
            </Rectangle>
            <Grid Grid.RowSpan="2"><Border Background="#FF000000" Opacity="0.45" /></Grid>
            <TopPanel Grid.Row="0" />
            <DockPanel Grid.Row="1"><Border /></DockPanel>
            <Border Grid.RowSpan="2" Background="#FF000000" />"##,
    );
    let d = look(Mode::Desktop, &[("Views/Library.xaml", &layered)], &[], None).desktop;
    let art = d.window_art.expect("a see-through layer keeps the art");
    assert_eq!(art.picture.darken.map(|c| c.0[3]), Some(179));
    assert_eq!(art.layers.len(), 2, "only the layers before the views: {:?}", art.layers);
    assert!(matches!(art.layers[0], LookBrush::Linear { .. }));
    assert_eq!(art.layers[0].stops()[0].0.0[3], 128, "the element's Opacity is folded in");
    assert_eq!(art.layers[1].average(), Some(Color([0, 0, 0, 115])), "a group of layers counts");
    // An opaque layer right over it hides it.
    let covered = library(
        r##"<FadeImage x:Name="PART_ImageBackground" Grid.RowSpan="2" />
            <Border Grid.RowSpan="2" Background="#FF101010" />
            <DockPanel Grid.Row="1"><Border /></DockPanel>"##,
    );
    assert!(look(Mode::Desktop, &[("Views/Library.xaml", &covered)], &[], None).desktop.window_art.is_none());
    // Hidden by the theme: none.
    let hidden = layered.replace("ImageDarkeningBrush=\"#FF000000\"", "Visibility=\"Collapsed\"");
    assert!(look(Mode::Desktop, &[("Views/Library.xaml", &hidden)], &[], None).desktop.window_art.is_none());
    // Covered by an opaque game list in the details view: none.
    let list = r##"<Style TargetType="{x:Type LibraryDetailsView}">
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{x:Type LibraryDetailsView}">
            <Grid Background="#FF101010"><ListBox /></Grid>
          </ControlTemplate>
        </Setter.Value></Setter>
      </Style>"##;
    let d = look(Mode::Desktop, &[("Views/LibraryDetailsView.xaml", list)], &[], None).desktop;
    assert!(d.window_art.is_none());
    assert_eq!(d.game_list.and_then(|b| b.average()), Some(c("#101010")));
}

#[test]
fn game_art_masks_place_and_a_picture_over_it() {
    // The art in the row under the top panel, faded by its own mask and its wrapper's; the
    // theme's vignette picture drawn after it lies over it.
    let body = library(
        r##"<Grid Grid.Row="1">
              <Grid.OpacityMask>
                <LinearGradientBrush StartPoint="0,0" EndPoint="1,0">
                  <GradientStop Color="#00000000" Offset="0" /><GradientStop Color="#FF000000" Offset="0.2" />
                </LinearGradientBrush>
              </Grid.OpacityMask>
              <FadeImage x:Name="PART_ImageBackground">
                <FadeImage.ImageOpacityMask>
                  <RadialGradientBrush Center="0.5,0" RadiusX="1" RadiusY="0.7">
                    <GradientStop Color="#FF000000" Offset="0.4" /><GradientStop Color="#00000000" Offset="1" />
                  </RadialGradientBrush>
                </FadeImage.ImageOpacityMask>
              </FadeImage>
            </Grid>
            <Image Grid.RowSpan="2" Source="{ThemeFile 'Images/vignette.png'}" Stretch="Fill" />
            <TopPanel Grid.Row="0" />
            <DockPanel Grid.Row="1"><Border /></DockPanel>"##,
    );
    let d = look(Mode::Desktop, &[("Views/Library.xaml", &body)], &[("Images/vignette.png", 40_000)], None).desktop;
    let art = d.window_art.expect("art");
    assert_eq!(art.masks.len(), 2);
    assert!(matches!(art.masks[0], LookBrush::Radial { .. }), "its own mask first");
    assert!(art.below_top_bar);
    assert!(art.picture_over_art);
    assert!(matches!(d.background_picture, Some(LookBrush::Image { .. })));
    // Before the art, the picture lies under it; spanning the top panel's row, the art is
    // behind the top bar too.
    let under = body.replace(r#"<Grid Grid.Row="1">"#, r#"<Grid Grid.RowSpan="2">"#);
    let under = under.replacen(
        r##"<Image Grid.RowSpan="2" Source="{ThemeFile 'Images/vignette.png'}" Stretch="Fill" />"##,
        "",
        1,
    );
    let under = under.replacen(
        "<Grid Grid.RowSpan=\"2\">",
        r##"<Image Grid.RowSpan="2" Source="{ThemeFile 'Images/vignette.png'}" Stretch="Fill" /><Grid Grid.RowSpan="2">"##,
        1,
    );
    let d = look(Mode::Desktop, &[("Views/Library.xaml", &under)], &[("Images/vignette.png", 40_000)], None).desktop;
    let art = d.window_art.expect("art");
    assert!(!art.below_top_bar && !art.picture_over_art);
}

#[test]
fn game_list_column_and_facts_box_fills() {
    // Playnite's stock list is ControlBackgroundBrush (transparent in Playnite's colours): no
    // fill; a theme's colour for it is the list's.
    assert!(look(Mode::Desktop, &[], &[], None).desktop.list_fill.is_none());
    let tint = r##"<SolidColorBrush x:Key="ControlBackgroundBrush" Color="#72121D17" />"##;
    let d = look(Mode::Desktop, &[("Constants.xaml", tint)], &[], None).desktop;
    assert_eq!(d.list_fill.and_then(|b| b.average()), Some(Color([0x12, 0x1d, 0x17, 0x72])));
    // … unless the theme's ListBox template never draws its Background.
    let clear_list = r##"<Style TargetType="{x:Type ListBox}">
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{x:Type ListBox}">
            <Border Background="Transparent"><ItemsPresenter /></Border>
          </ControlTemplate>
        </Setter.Value></Setter>
      </Style>"##;
    let files = [("Constants.xaml", tint), ("DefaultControls/ListBox.xaml", clear_list)];
    assert!(look(Mode::Desktop, &files, &[], None).desktop.list_fill.is_none());
    // A panel wrapped around the list a few levels down is the list column's fill, not the
    // view's (which stays see-through, so the art behind it still shows).
    let view = r##"<Style TargetType="{x:Type LibraryDetailsView}">
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{x:Type LibraryDetailsView}">
            <DockPanel>
              <Grid DockPanel.Dock="Left">
                <Border Background="#FF24282F"><Grid><ExtendedListBox x:Name="PART_ListGames" /></Grid></Border>
              </Grid>
              <DetailsViewGameOverview x:Name="PART_ControlGameView" />
            </DockPanel>
          </ControlTemplate>
        </Setter.Value></Setter>
      </Style>"##;
    let d = look(Mode::Desktop, &[("Views/LibraryDetailsView.xaml", view)], &[], None).desktop;
    assert_eq!(d.list_fill.and_then(|b| b.average()), Some(c("#24282F")));
    assert!(d.game_list.is_none() && d.window_art.is_some());

    // The facts' own box (not the panel that also holds the description).
    let page = r##"<Style TargetType="{x:Type DetailsViewGameOverview}">
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{x:Type DetailsViewGameOverview}">
            <Grid Background="#FF101010">
              <StackPanel x:Name="PART_ElemDescription"><HtmlTextView x:Name="PART_HtmlDescription" /></StackPanel>
              <Border Background="#FF202020" BorderBrush="#FFC8A040" BorderThickness="1" CornerRadius="6">
                <Grid><Label x:Name="PART_ElemPlayTime" /><TextBlock x:Name="PART_TextPlayTime" /></Grid>
              </Border>
            </Grid>
          </ControlTemplate>
        </Setter.Value></Setter>
      </Style>"##;
    let d = look(Mode::Desktop, &[("Views/DetailsViewGameOverview.xaml", page)], &[], None).desktop;
    let b = d.facts_box.expect("the facts' box");
    assert_eq!(b.fill.average(), Some(c("#202020")));
    assert_eq!(b.border.map(|(brush, w)| (brush.average(), w)), Some((Some(c("#C8A040")), 1.0)));
    assert_eq!(b.radius, Some(6.0));
    // Playnite's own page draws no box.
    assert!(look(Mode::Desktop, &[], &[], None).desktop.facts_box.is_none());
}

#[test]
fn game_list_rows_from_the_item_style() {
    // Playnite's own rows: a 5 px GlyphBrush bar and GlyphBrush text when selected, no fill;
    // installed games in TextBrush, the others in TextBrushDarker.
    let r = look(Mode::Desktop, &[], &[], None).desktop.rows.expect("rows");
    assert_eq!(r.selected_bar.map(|(b, w)| (b.average(), w)), Some((Some(c("#55cdff")), 5.0)));
    assert!(r.selected_fill.is_none());
    assert_eq!(r.selected_text.and_then(|b| b.average()), Some(c("#55cdff")));
    assert_eq!(r.text.and_then(|b| b.average()), Some(c("#f2f2f2")));
    assert_eq!(r.text_uninstalled.and_then(|b| b.average()), Some(c("#a3a3a3")));
    // A theme's rounded plate faded in on selection, and a thin bar on the left edge.
    let style = r##"<Style x:Key="DetailsViewItemStyle" TargetType="{x:Type ListBoxItem}">
        <Setter Property="Foreground" Value="#FF909090" />
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{x:Type ListBoxItem}">
            <Grid>
              <Border x:Name="Plate" Background="#FF303A50" CornerRadius="4" Opacity="0" />
              <Border x:Name="Bar" Width="3" HorizontalAlignment="Left" Background="#FFE0B040" Visibility="Collapsed" />
              <ContentPresenter />
            </Grid>
            <ControlTemplate.Triggers>
              <Trigger Property="IsSelected" Value="True">
                <Setter TargetName="Bar" Property="Visibility" Value="Visible" />
                <Setter Property="Foreground" Value="#FFFFFFFF" />
                <Trigger.EnterActions><BeginStoryboard><Storyboard>
                  <DoubleAnimation Storyboard.TargetName="Plate" Storyboard.TargetProperty="Opacity" To="1" />
                </Storyboard></BeginStoryboard></Trigger.EnterActions>
              </Trigger>
            </ControlTemplate.Triggers>
          </ControlTemplate>
        </Setter.Value></Setter>
      </Style>"##;
    let r = look(Mode::Desktop, &[("DerivedStyles/DetailsViewItemStyle.xaml", style)], &[], None).desktop.rows.unwrap();
    assert_eq!(r.selected_fill.and_then(|b| b.average()), Some(c("#303A50")));
    assert_eq!(r.radius, Some(4.0));
    assert_eq!(r.selected_bar.map(|(b, w)| (b.average(), w)), Some((Some(c("#E0B040")), 3.0)));
    assert_eq!(r.selected_text.and_then(|b| b.average()), Some(c("#ffffff")));
    assert_eq!(r.text.and_then(|b| b.average()), Some(c("#909090")), "no IsInstalled trigger: the style's");
}

#[test]
fn game_list_rows_merge_every_selected_trigger() {
    // The plate is revealed by scaling it up in the template's trigger; the selected text and a
    // later installed colour come from the style's triggers, which outrank the template's.
    let style = r##"<Style x:Key="DetailsViewItemStyle" TargetType="{x:Type ListBoxItem}">
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{x:Type ListBoxItem}">
            <Grid>
              <Rectangle x:Name="Plate" Fill="#FF2050A0">
                <Rectangle.RenderTransform><ScaleTransform x:Name="PlateScale" ScaleX="0" /></Rectangle.RenderTransform>
              </Rectangle>
              <ContentPresenter />
            </Grid>
            <ControlTemplate.Triggers>
              <DataTrigger Binding="{Binding IsInstalled}" Value="True">
                <Setter Property="Foreground" Value="#FF808080" />
              </DataTrigger>
              <Trigger Property="IsSelected" Value="True">
                <Setter Property="Foreground" Value="#FFFF0000" />
                <Trigger.EnterActions><BeginStoryboard><Storyboard>
                  <DoubleAnimation Storyboard.TargetName="PlateScale" Storyboard.TargetProperty="(ScaleTransform.ScaleX)" To="1" />
                </Storyboard></BeginStoryboard></Trigger.EnterActions>
              </Trigger>
            </ControlTemplate.Triggers>
          </ControlTemplate>
        </Setter.Value></Setter>
        <Style.Triggers>
          <DataTrigger Binding="{Binding IsInstalled}" Value="True">
            <Setter Property="Foreground" Value="#FFE0E0E0" />
          </DataTrigger>
          <Trigger Property="Selector.IsSelected" Value="True">
            <Setter Property="Foreground" Value="#FF101010" />
          </Trigger>
        </Style.Triggers>
      </Style>"##;
    let r = look(Mode::Desktop, &[("DerivedStyles/DetailsViewItemStyle.xaml", style)], &[], None).desktop.rows.unwrap();
    assert_eq!(r.selected_fill.and_then(|b| b.average()), Some(c("#2050A0")));
    assert_eq!(r.selected_text.and_then(|b| b.average()), Some(c("#101010")));
    assert_eq!(r.text.and_then(|b| b.average()), Some(c("#E0E0E0")));
}

#[test]
fn grid_tiles_and_the_views_play_button() {
    // An item template that only recolours a selection frame in a trigger has no tile.
    let frame_only = r##"<Style x:Key="GridViewItemStyle" TargetType="{x:Type ListBoxItem}">
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{x:Type ListBoxItem}">
            <Border x:Name="Sel" BorderThickness="2"><ContentPresenter /></Border>
            <ControlTemplate.Triggers>
              <Trigger Property="IsSelected" Value="True">
                <Setter TargetName="Sel" Property="BorderBrush" Value="{DynamicResource GridItemBackgroundBrush}" />
              </Trigger>
            </ControlTemplate.Triggers>
          </ControlTemplate>
        </Setter.Value></Setter>
      </Style>"##;
    let d = look(Mode::Desktop, &[("DerivedStyles/GridViewItemStyle.xaml", frame_only)], &[], None).desktop;
    assert!(d.grid.background.is_none(), "{:?}", d.grid.background);
    // Playnite's own item clears its GridItemBackgroundBrush while "show grid item
    // background" is off (its default).
    assert!(look(Mode::Desktop, &[], &[], None).desktop.grid.background.is_none());
    // A tile around the cover and the name; a strip around the name alone is the name's.
    let template = r##"<Style x:Key="GridViewItemTemplate" TargetType="{x:Type GameListItem}">
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{x:Type GameListItem}">
            <Border Background="#FF202020">
              <StackPanel>
                <Image x:Name="PART_ImageCover" />
                <Border Background="#FF404040"><TextBlock Text="{Binding DisplayName}" /></Border>
              </StackPanel>
            </Border>
          </ControlTemplate>
        </Setter.Value></Setter>
      </Style>"##;
    let d = look(Mode::Desktop, &[("DerivedStyles/GridViewItemTemplate.xaml", template)], &[], None).desktop;
    assert_eq!(d.grid.background.and_then(|b| b.average()), Some(c("#202020")));
    assert_eq!(d.caption_fill.and_then(|b| b.average()), Some(c("#404040")));

    // The details view names its own Play button style; that one is the Play colour.
    let styles = r##"<Style x:Key="PlayButton" TargetType="{x:Type Button}">
        <Setter Property="Background" Value="#FFC2C2C2"/>
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{x:Type Button}"><Border Background="{TemplateBinding Background}"/></ControlTemplate>
        </Setter.Value></Setter>
      </Style>
      <Style x:Key="PlayButtonViews" TargetType="{x:Type Button}" BasedOn="{StaticResource PlayButton}">
        <Setter Property="Background" Value="#FF007738"/>
      </Style>"##;
    let view = r##"<Style TargetType="{x:Type DetailsViewGameOverview}">
        <Setter Property="Template"><Setter.Value>
          <ControlTemplate TargetType="{x:Type DetailsViewGameOverview}">
            <StackPanel><Button x:Name="PART_ButtonPlayAction" Style="{DynamicResource PlayButtonViews}" /></StackPanel>
          </ControlTemplate>
        </Setter.Value></Setter>
      </Style>"##;
    let d = look(
        Mode::Desktop,
        &[("DerivedStyles/PlayButton.xaml", styles), ("Views/DetailsViewGameOverview.xaml", view)],
        &[],
        None,
    )
    .desktop;
    assert_eq!(d.play_button.and_then(|b| b.average()), Some(c("#007738")));
}

/// What darkens a game that isn't installed: a trigger whose other conditions can hold, on a
/// layer over the whole cover.
#[test]
fn the_uninstalled_scrim_needs_every_condition_and_the_whole_cover() {
    let scrim_of = |condition: &str, layer: &str| {
        let item = format!(
            r##"<Style x:Key="ListGameItemTemplate" TargetType="ContentControl">
  <Setter Property="Template">
    <Setter.Value>
      <ControlTemplate TargetType="ContentControl">
        <Grid>
          <Image Name="PART_ImageCover" />
          <Border x:Name="Shade" Background="#80102030" Visibility="Collapsed" {layer} />
        </Grid>
        <ControlTemplate.Triggers>
          <MultiDataTrigger>
            <MultiDataTrigger.Conditions>
              <Condition Binding="{{Binding IsInstalled}}" Value="False" />
              {condition}
            </MultiDataTrigger.Conditions>
            <Setter Property="Visibility" Value="Visible" TargetName="Shade" />
          </MultiDataTrigger>
        </ControlTemplate.Triggers>
      </ControlTemplate>
    </Setter.Value>
  </Setter>
</Style>"##
        );
        look(Mode::Fullscreen, &[("DerivedStyles/ListGameItemTemplate.xaml", &item)], &[], None).item.uninstalled_scrim
    };
    let favourite = r#"<Condition Binding="{Binding Favorite}" Value="True" />"#;
    let darken_setting = r#"<Condition Binding="{Settings Fullscreen.DarkenUninstalledGamesGrid}" Value="True" />"#;
    assert_eq!(scrim_of(favourite, ""), Some(c("#10203080")), "a condition that changes per game can hold");
    assert_eq!(scrim_of(darken_setting, ""), None, "the setting is off by default");
    assert_eq!(scrim_of(favourite, r#"Height="60" VerticalAlignment="Bottom""#), None, "a band, not a scrim");
}

/// A title bound to "show game titles" (off by default) that the selected item shows anyway.
#[test]
fn a_setting_hidden_caption_shown_on_the_selected_item() {
    let item = r##"<Style x:Key="ListGameItemStyle" TargetType="ListBoxItem">
  <Setter Property="Template">
    <Setter.Value>
      <ControlTemplate TargetType="ListBoxItem">
        <Grid>
          <ContentPresenter />
          <TextBlock x:Name="Title" Text="{Binding DisplayName}" Visibility="{Settings Fullscreen.ShowGameTitles}" />
        </Grid>
        <ControlTemplate.Triggers>
          <Trigger Property="IsSelected" Value="True">
            <Setter Property="Visibility" Value="Visible" TargetName="Title" />
          </Trigger>
        </ControlTemplate.Triggers>
      </ControlTemplate>
    </Setter.Value>
  </Setter>
</Style>"##;
    let shown = |item: &str| {
        let l = look(Mode::Fullscreen, &[("DerivedStyles/ListGameItemStyle.xaml", item)], &[], None);
        l.item.caption.map(|c| c.shown)
    };
    assert_eq!(shown(item), Some(Shown::Focus));
    let without_trigger = item.replace(r#"Property="IsSelected""#, r#"Property="IsEnabled""#);
    assert_eq!(shown(&without_trigger), None);
}

/// A name shown only for games without a cover (through a proxy element that follows
/// `CoverImage`) isn't the item's caption.
#[test]
fn a_name_for_games_without_a_cover_is_not_a_caption() {
    let item = r##"<Style x:Key="ListGameItemTemplate" TargetType="ContentControl">
  <Setter Property="Template">
    <Setter.Value>
      <ControlTemplate TargetType="ContentControl">
        <Grid>
          <Border x:Name="Proxy" Opacity="0" Visibility="{Binding CoverImage, Converter={StaticResource StringNullOrEmptyToVisibilityConverter}}" />
          <Image Name="PART_ImageCover" />
          <Grid Visibility="{Binding ElementName=Proxy, Path=IsVisible, Converter={StaticResource InvertedBooleanToVisibilityConverter}}">
            <TextBlock Text="{Binding DisplayName}" VerticalAlignment="Center" />
          </Grid>
        </Grid>
      </ControlTemplate>
    </Setter.Value>
  </Setter>
</Style>"##;
    let caption = |item: &str| {
        look(Mode::Fullscreen, &[("DerivedStyles/ListGameItemTemplate.xaml", item)], &[], None).item.caption
    };
    assert_eq!(caption(item), None);
    let always = item.replace(r#"ElementName=Proxy"#, r#"ElementName=Elsewhere"#);
    assert_eq!(caption(&always).map(|c| c.shown), Some(Shown::Always));
    // Elements that follow each other's visibility, never the cover's: still the caption.
    let circular = item.replace(
        "Visibility=\"{Binding CoverImage, Converter={StaticResource StringNullOrEmptyToVisibilityConverter}}\"",
        "Visibility=\"{Binding ElementName=Proxy, Path=IsVisible}\"",
    );
    assert_ne!(circular, item);
    assert_eq!(caption(&circular).map(|c| c.shown), Some(Shown::Always));
    // The other way round, the name shows only with a cover: it is the caption.
    let with_cover = item.replace("InvertedBooleanToVisibilityConverter", "BooleanToVisibilityConverter");
    assert_eq!(caption(&with_cover).map(|c| c.shown), Some(Shown::Always));
    // The same, bound to the cover directly.
    let direct = |converter: &str| {
        let binding = format!("{{Binding CoverImage, Converter={{StaticResource {converter}}}}}");
        item.replace("{Binding ElementName=Proxy, Path=IsVisible, Converter={StaticResource InvertedBooleanToVisibilityConverter}}", &binding)
    };
    assert_eq!(caption(&direct("InvertedStringNullOrEmptyToVisibilityConverter")), None);
    assert_eq!(caption(&direct("StringNullOrEmptyToVisibilityConverter")).map(|c| c.shown), Some(Shown::Always));
}

/// Names that follow "show names under covers" are kept: Lodestar's own "Show game names"
/// switch shows or hides them.
#[test]
fn desktop_names_follow_lodestars_names_switch() {
    let item = r##"<Style x:Key="GridViewItemTemplate" TargetType="{x:Type GameListItem}">
  <Setter Property="Template">
    <Setter.Value>
      <ControlTemplate TargetType="{x:Type GameListItem}">
        <Grid>
          <Image Name="PART_ImageCover" />
          <Border VerticalAlignment="Bottom" Background="#C0000000" Visibility="{Settings ShowNamesUnderCovers, Converter={StaticResource BooleanToVisibilityConverter}}">
            <TextBlock Text="{Binding Name}" />
          </Border>
        </Grid>
      </ControlTemplate>
    </Setter.Value>
  </Setter>
</Style>"##;
    let d = look(Mode::Desktop, &[("DerivedStyles/GridViewItemTemplate.xaml", item)], &[], None).desktop;
    assert_eq!(d.cover_title, CoverTitle::Overlay);
}
