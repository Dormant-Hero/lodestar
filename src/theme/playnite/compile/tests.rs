//! View compiler tests: Playnite's own default views (embedded, MIT) and hand-written XAML
//! that imitates how themes are built (no theme files).

use super::super::Mode;
use super::super::extract::look_of;
use super::super::layout::Rough;
use super::super::tree::tests::NS;
use super::super::tree::{Texts, Tree};
use super::bind::{bind_art, bind_text, parse_binding};
use super::build::icon_symbol;
use super::emit::tile_panel;
use super::visibility::duration_ms;
use super::*;

mod details;
mod list;
mod rest;
mod tabs;

thread_local! {
    /// Flowing panels as `stack` containers (`stack::enabled`). Off by default: the placement
    /// tests check absolute positions, so they write plain elements.
    pub(super) static STACKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn compiled(files: &[(&str, String)], assets: &[(&str, u64)]) -> Compiled {
    let texts = Texts::with_theme(Mode::Fullscreen, files.iter().map(|(a, b)| (a.to_string(), b.clone())).collect());
    let list: Vec<(String, u64)> = assets.iter().map(|(a, b)| (a.to_string(), *b)).collect();
    let tree = Tree::build(&texts, &list);
    let look = look_of(&tree, None);
    compile_tree(
        &tree,
        &look,
        &ViewStrings::with(&[("LOCThemeHello", "Hello there")]),
        &|_| Some([400.0, 200.0]),
        &mut Rough,
    )
}

fn by_type<'a>(els: &'a [Table], ty: &str) -> Vec<&'a Table> {
    els.iter().filter(|e| e.get("type").and_then(T::as_str) == Some(ty)).collect()
}

fn pair(t: &Table, k: &str) -> [f32; 2] {
    let a = t.get(k).and_then(T::as_array).expect(k);
    [a[0].as_float().unwrap() as f32, a[1].as_float().unwrap() as f32]
}

fn text_of<'a>(els: &'a [Table], text: &str) -> Option<&'a Table> {
    els.iter().find(|e| e.get("text").and_then(T::as_str) == Some(text))
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.002
}

fn wrap_main(body: &str) -> String {
    format!(
        r#"<ResourceDictionary {NS}>
  <Style TargetType="{{x:Type Main}}">
    <Setter Property="Template"><Setter.Value>
      <ControlTemplate TargetType="{{x:Type Main}}">
        {body}
      </ControlTemplate>
    </Setter.Value></Setter>
  </Style>
</ResourceDictionary>"#
    )
}

#[test]
fn playnite_default_main_translates() {
    let c = compiled(&[], &[]);
    let main = c.main.expect("main");
    assert!(c.main_problem.is_none(), "{:?} {:?}", c.main_problem, main.stats);
    assert!(!main.own);
    let els = &main.elements;
    // The list fills the middle row inside its 25 px margins: 110 px top bar, 100 px bottom bar.
    let list = els.iter().find(|e| e.get("name").and_then(T::as_str) == Some("pn_list")).expect("list");
    assert_eq!(list.get("type").and_then(T::as_str), Some("grid"));
    let p = pair(list, "pos");
    let s = pair(list, "size");
    assert!(near(p[0], 25.0 / 1920.0) && near(p[1], 110.0 / 1080.0), "{p:?}");
    assert!(near(s[0], 1870.0 / 1920.0) && near(s[1], 870.0 / 1080.0), "{s:?}");
    // The two bars are TopBarBackgroundBrush (an angled gradient).
    let bars: Vec<&Table> = by_type(els, "rect")
        .into_iter()
        .filter(|r| near(pair(r, "size")[0], 1.0) && pair(r, "size")[1] < 0.11)
        .collect();
    assert_eq!(bars.len(), 2, "{bars:#?}");
    assert!(near(pair(bars[0], "size")[1], 110.0 / 1080.0));
    assert!(near(pair(bars[1], "pos")[1], 980.0 / 1080.0));
    // Clock, battery and the help bar in the bottom bar.
    assert_eq!(by_type(els, "datetime").len(), 1);
    assert!(els.iter().any(|e| e.get("text").and_then(T::as_str) == Some("{system.battery}")));
    let help = by_type(els, "helpbar");
    assert_eq!(help.len(), 1);
    assert!(pair(help[0], "pos")[1] > 0.9);
    // The game's picture behind everything, and "No games found" isn't shown (there are games).
    let pic = by_type(els, "image").into_iter().find(|i| i.get("path").and_then(T::as_str) == Some("{art.hero}"));
    assert!(pic.is_some());
    assert!(text_of(els, "No games found").is_none());
    // Every element parses as a Lodestar element.
    for e in els {
        let toml = toml::to_string(e).unwrap();
        let _: crate::theme::model::Element = toml::from_str(&toml).unwrap_or_else(|err| panic!("{err}\n{toml}"));
    }
}

#[test]
fn playnite_default_details_translate() {
    let c = compiled(&[], &[]);
    let d = c.details.expect("details");
    let els = &d.elements;
    // Cover in the 350 px column, right-aligned; the name in the 6* column next to it.
    let cover = by_type(els, "image")
        .into_iter()
        .find(|i| i.get("path").and_then(T::as_str) == Some("{art.cover}"))
        .expect("cover");
    let p = pair(cover, "pos");
    assert!(p[1] > 0.42 && p[1] < 0.45, "{p:?}");
    let name = text_of(els, "{game.name}").expect("name");
    let np = pair(name, "pos");
    assert!(near(np[0], ((1920.0 - 350.0) / 8.0 + 350.0 + 20.0) / 1920.0), "{np:?}");
    // The labels come from Playnite's English strings, the values from the game.
    assert!(text_of(els, "Time Played").is_some());
    assert!(text_of(els, "{game.playtime}").is_some());
    assert!(text_of(els, "{game.lastplayed}").is_some());
    // Play / Install follow the game's state.
    let play = text_of(els, "Play").expect("play");
    assert_eq!(play.get("visible_if").and_then(T::as_str), Some("{game.installed}"));
    let install = text_of(els, "Install").expect("install");
    assert_eq!(install.get("hidden_if").and_then(T::as_str), Some("{game.installed}"));
    // The description scrolls in what is left of the page.
    let desc = text_of(els, "{game.description}").expect("description");
    assert_eq!(desc.get("scroll").and_then(T::as_bool), Some(true));
    let dp = pair(desc, "pos");
    let ds = pair(desc, "size");
    assert!(dp[1] + ds[1] <= 1.0 + 1e-3, "{dp:?} {ds:?}");
}

#[test]
fn a_theme_main_with_triggers_pictures_and_a_row() {
    let main = wrap_main(
        r##"<Grid x:Name="PART_MainHost" Background="#FF101010">
          <Grid.RowDefinitions><RowDefinition Height="*"/><RowDefinition Height="300"/></Grid.RowDefinitions>
          <Image Source="{ThemeFile 'Images/logo.png'}" Width="200" HorizontalAlignment="Left" VerticalAlignment="Top" Margin="40,30,0,0"/>
          <StackPanel Margin="100,0,0,40" VerticalAlignment="Bottom" HorizontalAlignment="Left">
            <TextBlock Text="{Binding SelectedGame.DisplayName, Converter={StaticResource StringToUpperCaseConverter}}" FontSize="48" Foreground="#FFFFAA00"/>
            <TextBlock Text="{DynamicResource LOCThemeHello}" FontSize="20"/>
            <TextBlock Text="{Binding SelectedGame.ReleaseYear, StringFormat='{}Released {0}'}" FontSize="20"/>
            <Border x:Name="NotInstalled" Background="#80FF0000" Height="30" Width="120">
              <Border.Style>
                <Style TargetType="Border">
                  <Setter Property="Visibility" Value="Collapsed"/>
                  <Style.Triggers>
                    <DataTrigger Binding="{Binding SelectedGame.IsInstalled}" Value="False">
                      <Setter Property="Visibility" Value="Visible"/>
                    </DataTrigger>
                  </Style.Triggers>
                </Style>
              </Border.Style>
            </Border>
            <Border x:Name="Helper" Background="#FF00FF00" Height="30">
              <Border.Style>
                <Style TargetType="Border">
                  <Style.Triggers>
                    <DataTrigger Binding="{Binding ElementName=PART_ListGameItems, Path=SelectedIndex}" Value="0">
                      <Setter Property="Visibility" Value="Collapsed"/>
                    </DataTrigger>
                  </Style.Triggers>
                </Style>
              </Border.Style>
            </Border>
            <TextBlock Text="{PluginSettings Plugin=SuccessStory, Path=Count}"/>
          </StackPanel>
          <ListBoxEx x:Name="PART_ListGameItems" Grid.Row="1" Margin="20,0,20,20"/>
          <ContentControl x:Name="SuccessStory_PluginCompact" Grid.Row="0" Width="300" Height="200" HorizontalAlignment="Right"/>
          <TextBlock x:Name="PART_TextClock" FontSize="30" HorizontalAlignment="Right" VerticalAlignment="Top" Margin="0,20,40,0"/>
        </Grid>"##,
    );
    let c = compiled(&[("Views/Main.xaml", main)], &[("Images/logo.png", 5000)]);
    let m = c.main.expect("main");
    assert!(m.own);
    let els = &m.elements;
    // A 300 px row of covers: a carousel across its cell.
    let list = els.iter().find(|e| e.get("name").and_then(T::as_str) == Some("pn_list")).unwrap();
    assert_eq!(list.get("type").and_then(T::as_str), Some("carousel"));
    assert!(near(pair(list, "pos")[1], 780.0 / 1080.0));
    assert!(near(pair(list, "size")[1], 280.0 / 1080.0));
    // The picture is copied into the theme and placed where the theme puts it (200 × 100 px).
    let logo = by_type(els, "image")
        .into_iter()
        .find(|i| i.get("path").and_then(T::as_str).is_some_and(|p| p.starts_with("images/view/")))
        .unwrap();
    assert!(near(pair(logo, "pos")[0], 40.0 / 1920.0) && near(pair(logo, "size")[1], 100.0 / 1080.0));
    assert_eq!(c.images, vec![("Images/logo.png".to_string(), "images/view/Images_logo.png".to_string())]);
    // Game texts with converters and formats; the theme's own strings.
    let name = text_of(els, "{game.name}").unwrap();
    let st = name.get("style").and_then(T::as_table).unwrap();
    assert_eq!(st.get("uppercase").and_then(T::as_bool), Some(true));
    assert_eq!(st.get("color").and_then(T::as_str), Some("#ffaa00"));
    assert!(near(st.get("font_size").and_then(T::as_float).unwrap() as f32, 48.0 / 1080.0));
    assert!(text_of(els, "Hello there").is_some());
    assert!(text_of(els, "Released {game.year}").is_some());
    // IsInstalled = False → shown only for games that aren't installed.
    let red =
        by_type(els, "rect").into_iter().find(|r| r.get("color").and_then(T::as_str) == Some("#ff000080")).unwrap();
    assert_eq!(red.get("hidden_if").and_then(T::as_str), Some("{game.installed}"));
    // The helper keeps its resting state (shown) and is counted as dropped.
    assert!(by_type(els, "rect").iter().any(|r| r.get("color").and_then(T::as_str) == Some("#00ff00")));
    assert!(m.stats.triggers_dropped >= 1 && m.stats.triggers_mapped >= 1);
    // The add-on panel and the add-on text are dropped.
    assert!(m.stats.dropped.contains_key("add-on panel"), "{:?}", m.stats.dropped);
    assert_eq!(by_type(els, "datetime").len(), 1);
    for e in els {
        let toml = toml::to_string(e).unwrap();
        let _: crate::theme::model::Element = toml::from_str(&toml).unwrap_or_else(|err| panic!("{err}\n{toml}"));
    }
}

#[test]
fn a_hidden_list_or_an_add_on_screen_is_not_used() {
    let hidden = wrap_main(
        r##"<Grid>
          <ListBoxEx x:Name="PART_ListGameItems" Visibility="Collapsed"/>
          <ListBox x:Name="Mirror" Height="300" VerticalAlignment="Bottom"/>
        </Grid>"##,
    );
    let c = compiled(&[("Views/Main.xaml", hidden)], &[]);
    // Said in plain words after "The theme's own main screen".
    assert_eq!(c.main_problem.as_deref(), Some("needs Playnite add-ons"));
    // Mostly a mirrored list and helper controls: not enough of it is Lodestar's to draw.
    let mirror = wrap_main(
        r##"<Grid>
          <ListBox x:Name="Mirror" Margin="0,0,0,200"/>
          <ListBoxEx x:Name="PART_ListGameItems" Height="200" VerticalAlignment="Bottom"/>
        </Grid>"##,
    );
    let c = compiled(&[("Views/Main.xaml", mirror)], &[]);
    let m = c.main.unwrap();
    assert!(m.stats.coverage() < MIN_COVERAGE, "{:?}", m.stats);
    assert!(c.main_problem.is_some());
    // An add-on's panel shows nothing in a Playnite without that add-on: it doesn't count.
    let addon = wrap_main(
        r##"<Grid>
          <ContentControl x:Name="AnikiHelper_Hub" Margin="0,0,0,200"/>
          <ListBoxEx x:Name="PART_ListGameItems" Height="200" VerticalAlignment="Bottom"/>
        </Grid>"##,
    );
    let c = compiled(&[("Views/Main.xaml", addon)], &[]);
    assert!(c.main_problem.is_none(), "{:?}", c.main.unwrap().stats);
}

#[test]
fn fading_elements_get_an_appear_transition_and_masks_become_gradients() {
    let main = wrap_main(
        r##"<Grid>
          <Border Background="Black" Height="200" VerticalAlignment="Top">
            <Border.OpacityMask>
              <LinearGradientBrush StartPoint="0,0" EndPoint="0,1">
                <GradientStop Color="#FF000000" Offset="0"/>
                <GradientStop Color="#00000000" Offset="1"/>
              </LinearGradientBrush>
            </Border.OpacityMask>
          </Border>
          <TextBlock Text="{Binding SelectedGame.DisplayName}" Opacity="0" HorizontalAlignment="Center" VerticalAlignment="Center">
            <TextBlock.Triggers>
              <EventTrigger RoutedEvent="Binding.TargetUpdated">
                <BeginStoryboard><Storyboard>
                  <DoubleAnimation Storyboard.TargetProperty="Opacity" From="0" To="1" Duration="0:0:0.4" BeginTime="0:0:0.1"/>
                </Storyboard></BeginStoryboard>
              </EventTrigger>
            </TextBlock.Triggers>
          </TextBlock>
          <ListBoxEx x:Name="PART_ListGameItems" Height="400" VerticalAlignment="Bottom"/>
        </Grid>"##,
    );
    let c = compiled(&[("Views/Main.xaml", main)], &[]);
    let els = c.main.unwrap().elements;
    let name = text_of(&els, "{game.name}").unwrap();
    let appear = name.get("transitions").and_then(|t| t.get("appear")).and_then(T::as_table).unwrap();
    assert_eq!(appear.get("duration_ms").and_then(T::as_integer), Some(400));
    assert_eq!(appear.get("delay_ms").and_then(T::as_integer), Some(100));
    assert_eq!(appear.get("replay").and_then(T::as_str), Some("select"));
    // A centred per-game text spans its slot, centred on the same point.
    let p = pair(name, "pos");
    let s = pair(name, "size");
    assert!(near(p[0] + s[0] / 2.0, 0.5), "{p:?} {s:?}");
    let shade = by_type(&els, "rect").into_iter().find(|r| near(pair(r, "size")[1], 200.0 / 1080.0)).unwrap();
    let stops = shade.get("gradient").and_then(T::as_array).unwrap();
    assert_eq!(stops[0].as_str(), Some("#000000"));
    assert_eq!(stops[1].as_str(), Some("#00000000"));
}

#[test]
fn bindings_and_converters() {
    let b = parse_binding("SelectedGame.Playtime, Converter={StaticResource PlayTimeToStringConverter}, Mode=OneWay");
    assert_eq!(b.path, "SelectedGame.Playtime");
    assert_eq!(b.converter.as_deref(), Some("PlayTimeToStringConverter"));
    assert_eq!(bind_text(&b, true), Some(("{game.playtime}".into(), false)));
    let b = parse_binding("Path=Genres[0].Name");
    assert_eq!(bind_text(&b, false), Some(("{game.genre}".into(), false)));
    let b = parse_binding("Items.Count, ElementName=PART_ListGameItems, StringFormat='{}{0} games'");
    assert_eq!(bind_text(&b, true), Some(("{list.count} games".into(), false)));
    // Main's view model, not the game: no data.
    assert_eq!(bind_text(&parse_binding("MainMenuVisible"), true), None);
    assert_eq!(bind_art(&parse_binding("SelectedGame.CoverImageObjectCached")), Some("{art.cover}"));
    assert_eq!(bind_art(&parse_binding("SelectedGame.Platforms[0].Icon")), None);
    assert_eq!(duration_ms("0:0:0.35"), Some(350));
    // Where a RelativeSource points.
    assert!(parse_binding("Tag, RelativeSource={RelativeSource Self}").on_self());
    assert!(parse_binding("IsFocused, RelativeSource={RelativeSource Mode=TemplatedParent}").on_control());
    let ancestor = parse_binding("DataContext.Time, RelativeSource={RelativeSource AncestorType={x:Type Main}}");
    assert!(ancestor.relative && !ancestor.on_control());
    assert!(!parse_binding("Tag, ElementName=SelfTest").on_self());
}

#[test]
#[ignore]
fn dump_default() {
    let c = compiled(&[], &[]);
    for e in &c.details.unwrap().elements {
        eprintln!("{}", toml::to_string(e).unwrap().replace('\n', " | "));
    }
}

#[test]
fn per_game_texts_flow_in_a_stack() {
    STACKS.with(|f| f.set(true));
    let c = compiled(&[], &[]);
    STACKS.with(|f| f.set(false));
    let els = c.details.unwrap().elements;
    let stacks = by_type(&els, "stack");
    assert_eq!(stacks.len(), 1, "{els:#?}");
    let st = stacks[0];
    // Playnite's details DockPanel: name, the facts row, the buttons, the description.
    let kids = st.get("children").and_then(T::as_array).unwrap();
    let kinds: Vec<&str> = kids.iter().map(|k| k.get("type").and_then(T::as_str).unwrap()).collect();
    assert_eq!(kinds, ["text", "stack", "stack", "text"], "{kids:#?}");
    assert_eq!(kids[0].get("text").and_then(T::as_str), Some("{game.name}"));
    assert_eq!(kids[1].get("direction").and_then(T::as_str), Some("horizontal"));
    // The facts row keeps its WPF margin (0,20,20,20 → fractions of the height).
    let m = kids[1].get("margin").and_then(T::as_array).unwrap();
    assert!(near(m[1].as_float().unwrap() as f32, 20.0 / 1080.0));
    let desc = &kids[3];
    assert_eq!(desc.get("scroll").and_then(T::as_bool), Some(true));
    assert!(pair(desc.as_table().unwrap(), "size")[1] > 0.1);
    // The name and the facts are no longer drawn on their own.
    assert!(text_of(&els, "{game.name}").is_none());
    // The stack is a valid element of the theme model.
    let toml = toml::to_string(st).unwrap();
    let _: crate::theme::model::Element = toml::from_str(&toml).unwrap();
}

#[test]
fn details_slide_ins_rest_where_they_end_and_bound_image_brushes_are_art() {
    // A details page whose facts slide in from off-screen when the view opens (a Margin
    // animation on "PART_ElemGameDetails is Visible"), and a cover drawn as a Border's
    // ImageBrush bound to the game.
    let details = format!(
        r#"<ResourceDictionary {NS}>
  <Style TargetType="{{x:Type GameDetails}}">
    <Setter Property="Template"><Setter.Value>
      <ControlTemplate TargetType="{{x:Type GameDetails}}">
        <Grid>
          <Border Width="150" Height="225" HorizontalAlignment="Left" VerticalAlignment="Top" Margin="100,50,0,0"
                  CornerRadius="12" BorderThickness="4" BorderBrush="White">
            <Border.Background>
              <ImageBrush ImageSource="{{Binding Game.CoverImage, Converter={{StaticResource ImageStringToImageConverter}}}}"/>
            </Border.Background>
          </Border>
          <StackPanel HorizontalAlignment="Left" VerticalAlignment="Top" Margin="-3000,400,0,0">
            <StackPanel.Style>
              <Style>
                <Style.Triggers>
                  <DataTrigger Binding="{{Binding ElementName=PART_ElemGameDetails, Path=Visibility}}" Value="Visible">
                    <DataTrigger.EnterActions>
                      <BeginStoryboard><Storyboard>
                        <ThicknessAnimation Storyboard.TargetProperty="Margin" Duration="0:0:0.2" From="-3000,400,0,0" To="300,400,0,0"/>
                      </Storyboard></BeginStoryboard>
                    </DataTrigger.EnterActions>
                  </DataTrigger>
                </Style.Triggers>
              </Style>
            </StackPanel.Style>
            <TextBlock Text="{{Binding DisplayName}}"/>
          </StackPanel>
        </Grid>
      </ControlTemplate>
    </Setter.Value></Setter>
  </Style>
</ResourceDictionary>"#
    );
    let c = compiled(&[("Views/GameDetails.xaml", details)], &[]);
    let els = c.details.expect("details").elements;
    let name = text_of(&els, "{game.name}").expect("name");
    let p = pair(name, "pos");
    assert!(near(p[0], 300.0 / 1920.0) && near(p[1], 400.0 / 1080.0), "{p:?}");
    let cover = by_type(&els, "image")
        .into_iter()
        .find(|i| i.get("path").and_then(T::as_str) == Some("{art.cover}"))
        .expect("cover");
    let cp = pair(cover, "pos");
    assert!(near(cp[0], 100.0 / 1920.0) && near(cp[1], 50.0 / 1080.0), "{cp:?}");
    assert!(cover.get("corner_radius").is_some());
    // Its frame is drawn over it.
    assert!(by_type(&els, "rect").iter().any(|r| r.get("border_color").is_some() && near(pair(r, "pos")[0], cp[0])));
}

fn list_of(els: &[Table]) -> &Table {
    els.iter().find(|e| e.get("name").and_then(T::as_str) == Some("pn_list")).expect("list")
}

#[test]
fn element_name_bindings_follow_the_other_elements_resting_value() {
    // Playnite's own idiom for the layout switch: a CheckBox holding a setting, triggers on it.
    let main = wrap_main(
        r##"<Grid>
          <CheckBoxEx x:Name="ChangeViewButton" IsChecked="{Settings Fullscreen.HorizontalLayout, Mode=TwoWay}" Width="0" Height="0"/>
          <TextBlock Text="{Binding SelectedGame.DisplayName}" VerticalAlignment="Top" FontSize="40">
            <TextBlock.Style><Style TargetType="TextBlock">
              <Setter Property="Visibility" Value="Collapsed"/>
              <Style.Triggers>
                <DataTrigger Binding="{Binding ElementName=ChangeViewButton, Path=IsChecked}" Value="True">
                  <Setter Property="Visibility" Value="Visible"/>
                </DataTrigger>
              </Style.Triggers>
            </Style></TextBlock.Style>
          </TextBlock>
          <ListBoxEx x:Name="PART_ListGameItems" VerticalAlignment="Bottom">
            <ListBox.Style><Style TargetType="ListBoxEx">
              <Style.Triggers>
                <DataTrigger Binding="{Binding ElementName=ChangeViewButton, Path=IsChecked}" Value="True">
                  <Setter Property="Height" Value="320"/>
                </DataTrigger>
                <DataTrigger Binding="{Binding ElementName=ChangeViewButton, Path=IsChecked}" Value="False">
                  <Setter Property="Height" Value="900"/>
                </DataTrigger>
              </Style.Triggers>
            </Style></ListBox.Style>
          </ListBoxEx>
          <Border x:Name="Shade" Background="#FF000000" Opacity="0"/>
          <Border Background="#FF000000" Opacity="{Binding Opacity, ElementName=Shade}"/>
        </Grid>"##,
    );
    let c = compiled(&[("Views/Main.xaml", main)], &[]);
    assert!(c.main_problem.is_none(), "{:?}", c.main_problem);
    let m = c.main.unwrap();
    // The horizontal branch (Playnite's default): the title shows, the list is one 320 px row.
    assert!(text_of(&m.elements, "{game.name}").is_some());
    let list = list_of(&m.elements);
    assert_eq!(list.get("type").and_then(T::as_str), Some("carousel"));
    assert!(near(pair(list, "size")[1], 320.0 / 1080.0), "{list:?}");
    // A layer whose opacity follows a transparent one isn't drawn over everything.
    assert!(!by_type(&m.elements, "rect").iter().any(|r| r.get("color").and_then(T::as_str) == Some("#000000")));
}

#[test]
fn screens_the_theme_takes_away_dont_cover_the_list() {
    let main = wrap_main(
        r##"<Grid>
          <ListBoxEx x:Name="PART_ListGameItems" Height="400" VerticalAlignment="Bottom"/>
          <Grid x:Name="Intro" Background="Black">
            <TextBlock Text="Welcome" FontSize="60" HorizontalAlignment="Center" VerticalAlignment="Center"/>
            <Grid.Style><Style TargetType="Grid">
              <Setter Property="Opacity" Value="1"/>
              <Style.Triggers>
                <EventTrigger RoutedEvent="Window.Loaded">
                  <BeginStoryboard><Storyboard>
                    <DoubleAnimation Storyboard.TargetProperty="Opacity" From="1" To="1" Duration="0:0:0"/>
                    <DoubleAnimation Storyboard.TargetProperty="Opacity" From="1" To="0" BeginTime="0:0:5" Duration="0:0:1"/>
                  </Storyboard></BeginStoryboard>
                </EventTrigger>
              </Style.Triggers>
            </Style></Grid.Style>
          </Grid>
        </Grid>"##,
    );
    let c = compiled(&[("Views/Main.xaml", main)], &[]);
    assert!(c.main_problem.is_none(), "{:?}", c.main_problem);
    assert!(text_of(&c.main.unwrap().elements, "Welcome").is_none());
    // A panel that fades out and back in (on the selection, say) stays.
    let back = wrap_main(
        r##"<Grid>
          <ListBoxEx x:Name="PART_ListGameItems" Height="400" VerticalAlignment="Bottom"/>
          <TextBlock x:Name="Info" Text="Info" FontSize="30" VerticalAlignment="Top">
            <TextBlock.Triggers>
              <EventTrigger RoutedEvent="Binding.TargetUpdated">
                <BeginStoryboard><Storyboard>
                  <DoubleAnimation Storyboard.TargetProperty="Opacity" To="0" Duration="0:0:0.1"/>
                  <DoubleAnimation Storyboard.TargetProperty="Opacity" To="1" BeginTime="0:0:0.1" Duration="0:0:0.2"/>
                </Storyboard></BeginStoryboard>
              </EventTrigger>
            </TextBlock.Triggers>
          </TextBlock>
        </Grid>"##,
    );
    let c = compiled(&[("Views/Main.xaml", back)], &[]);
    assert!(text_of(&c.main.unwrap().elements, "Info").is_some());
}

#[test]
fn add_ons_lodestar_provides_count_as_there() {
    let main = wrap_main(
        r##"<Grid>
          <ListBoxEx x:Name="PART_ListGameItems" Height="300" VerticalAlignment="Bottom"/>
          <TextBlock Text="{Binding SelectedGame.Name}" VerticalAlignment="Top" FontSize="40"
                     Tag="{PluginSettings Plugin=ExtraMetadataLoader, Path=IsLogoAvailable, FallbackValue=False}">
            <TextBlock.Style><Style TargetType="TextBlock">
              <Style.Triggers>
                <DataTrigger Binding="{Binding Tag, RelativeSource={RelativeSource Self}}" Value="True">
                  <Setter Property="Visibility" Value="Collapsed"/>
                </DataTrigger>
              </Style.Triggers>
            </Style></TextBlock.Style>
          </TextBlock>
          <TextBlock Text="Install Theme Options" FontSize="30" VerticalAlignment="Center">
            <TextBlock.Style><Style TargetType="TextBlock">
              <Setter Property="Visibility" Value="Collapsed"/>
              <Style.Triggers>
                <DataTrigger Binding="{PluginStatus Plugin=ThemeOptions_904cbf3b-573f-48f8-9642-0a09d05c64ef, Status=Installed}" Value="False">
                  <Setter Property="Visibility" Value="Visible"/>
                </DataTrigger>
              </Style.Triggers>
            </Style></TextBlock.Style>
          </TextBlock>
        </Grid>"##,
    );
    let c = compiled(&[("Views/Main.xaml", main)], &[]);
    let els = c.main.unwrap().elements;
    // The name stands in for a missing logo only.
    let name = text_of(&els, "{game.name}").expect("name");
    assert_eq!(name.get("hidden_if").and_then(T::as_str), Some("{art.logo}"));
    assert!(text_of(&els, "Install Theme Options").is_none());
}

#[test]
fn a_list_behind_a_toggle_is_shown_open() {
    // At rest the theme hides its list until "more games" is checked (helper rows show
    // instead): Lodestar takes the view with the toggle as the user flips it.
    let main = wrap_main(
        r##"<Grid>
          <CheckBoxEx x:Name="MoreGames" IsChecked="False" Width="0" Height="0"/>
          <ListBoxEx x:Name="PART_ListGameItems" Height="400" VerticalAlignment="Bottom">
            <ListBox.Style><Style TargetType="ListBoxEx">
              <Style.Triggers>
                <DataTrigger Binding="{Binding ElementName=MoreGames, Path=IsChecked}" Value="False">
                  <Setter Property="Visibility" Value="Collapsed"/>
                </DataTrigger>
              </Style.Triggers>
            </Style></ListBox.Style>
          </ListBoxEx>
        </Grid>"##,
    );
    let c = compiled(&[("Views/Main.xaml", main)], &[]);
    assert!(c.main_problem.is_none(), "{:?}", c.main_problem);
    assert!(c.main.unwrap().stats.list);
}

#[test]
fn a_condition_that_never_holds_decides_a_multi_trigger() {
    // The video add-on isn't there: the fade-out never runs, whatever the controller does.
    let main = wrap_main(
        r##"<Grid>
          <ListBoxEx x:Name="PART_ListGameItems" Height="300" VerticalAlignment="Bottom"/>
          <TextBlock Text="Backdrop" FontSize="30" VerticalAlignment="Top">
            <TextBlock.Style><Style TargetType="TextBlock">
              <Style.Triggers>
                <MultiDataTrigger>
                  <MultiDataTrigger.Conditions>
                    <Condition Binding="{Binding ElementName=PART_ListGameItems, Path=SelectedIndex}" Value="0"/>
                    <Condition Binding="{PluginSettings Plugin=ExtraMetadataLoader, Path=IsVideoPlaying}" Value="True"/>
                  </MultiDataTrigger.Conditions>
                  <Setter Property="Visibility" Value="Collapsed"/>
                </MultiDataTrigger>
              </Style.Triggers>
            </Style></TextBlock.Style>
          </TextBlock>
        </Grid>"##,
    );
    let c = compiled(&[("Views/Main.xaml", main)], &[]);
    let m = c.main.unwrap();
    assert!(text_of(&m.elements, "Backdrop").is_some());
    assert_eq!(m.stats.triggers_dropped, 0, "{:?}", m.stats);
}

#[test]
fn the_filter_preset_selector_becomes_tabs_of_lodestars_collections() {
    let main = wrap_main(
        r##"<Grid>
          <FilterPresetSelector x:Name="PART_FilterPresetSelector" VerticalAlignment="Top" Height="60" FontSize="24"/>
          <ListBoxEx x:Name="PART_ListGameItems" Margin="0,80,0,0"/>
        </Grid>"##,
    );
    let c = compiled(&[("Views/Main.xaml", main)], &[]);
    let m = c.main.unwrap();
    let tabs = by_type(&m.elements, "stack")
        .into_iter()
        .find(|s| s.get("direction").and_then(T::as_str) == Some("horizontal"))
        .expect("tabs");
    let kids = tabs.get("children").and_then(T::as_array).unwrap();
    let texts: Vec<(&str, Option<&str>, Option<&str>)> = kids
        .iter()
        .filter_map(T::as_table)
        .map(|k| {
            (
                k.get("text").and_then(T::as_str).unwrap_or(""),
                k.get("visible_if").and_then(T::as_str),
                k.get("hidden_if").and_then(T::as_str),
            )
        })
        .collect();
    assert!(texts.contains(&("All Games", None, Some("{collection.key} == all"))), "{texts:?}");
    assert!(texts.contains(&("All Games", Some("{collection.key} == all"), None)), "{texts:?}");
    assert!(texts.contains(&("Favourites", Some("{collection.key} == favorites"), None)), "{texts:?}");
    // Any other open collection: its own tab.
    let other = texts.iter().find(|t| t.0 == "{collection.name}").expect("other");
    assert!(other.1.is_some_and(|v| v.contains("{collection.key} != all && ")), "{other:?}");
    // Nothing beside it: room up to the screen's edge, so its text keeps its size.
    let reach = (pair(tabs, "pos")[0] + tabs.get("max_width").and_then(T::as_float).unwrap() as f32) * W;
    assert!((reach - W).abs() < 0.5, "{reach}");
    let toml = toml::to_string(tabs).unwrap();
    let _: crate::theme::model::Element = toml::from_str(&toml).unwrap_or_else(|err| panic!("{err}\n{toml}"));
}

/// The tab strip and the theme's LB / RB prompts in `els`: (strip's left edge, the most it
/// may reach, prompts' [left, right]) in px.
fn tabs_and_shoulders(els: &[Table]) -> (f32, f32, Vec<[f32; 2]>) {
    let tabs = by_type(els, "stack")
        .into_iter()
        .find(|s| s.get("direction").and_then(T::as_str) == Some("horizontal"))
        .expect("tabs");
    assert_eq!(pair(tabs, "origin")[0], 0.0, "left-aligned strip");
    let x = pair(tabs, "pos")[0] * W;
    let max = tabs.get("max_width").and_then(T::as_float).expect("max_width") as f32 * W;
    let prompts = by_type(els, "helpbar")
        .into_iter()
        .filter(|e| e.get("glyphs").and_then(T::as_bool) == Some(true))
        .map(|e| [pair(e, "pos")[0] * W, (pair(e, "pos")[0] + pair(e, "size")[0]) * W])
        .collect();
    (x, x + max, prompts)
}

/// Lodestar writes the tabs' names where Playnite themes show small pictures, so the names
/// can be wider than the room the theme gives them: the strip keeps inside its dock (a
/// Border with a MaxWidth, which cuts off what doesn't fit in WPF) and clear of the RB
/// prompt after it.
#[test]
fn the_tab_strip_keeps_inside_its_dock_and_clear_of_the_shoulder_prompts() {
    let main = wrap_main(
        r##"<Grid>
          <ListBoxEx x:Name="PART_ListGameItems" Height="600" VerticalAlignment="Top"/>
          <Grid VerticalAlignment="Bottom" Height="120">
            <Grid.ColumnDefinitions>
              <ColumnDefinition Width="*"/><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/>
              <ColumnDefinition Width="Auto"/><ColumnDefinition Width="*"/>
            </Grid.ColumnDefinitions>
            <ContentControl Grid.Column="1" Width="60" Height="60" ContentTemplate="{DynamicResource ButtonPromptLB}"/>
            <Border Grid.Column="2" MaxWidth="600" Background="#202020">
              <Grid>
                <Grid.ColumnDefinitions><ColumnDefinition Width="Auto"/><ColumnDefinition Width="Auto"/></Grid.ColumnDefinitions>
                <Rectangle Width="100" Height="40" Fill="Red"/>
                <FilterPresetSelector x:Name="PART_FilterPresetSelector" Grid.Column="1" FontSize="30"/>
              </Grid>
            </Border>
            <ContentControl Grid.Column="3" Width="60" Height="60" ContentTemplate="{DynamicResource ButtonPromptRB}"/>
          </Grid>
        </Grid>"##,
    );
    let els = compiled(&[("Views/Main.xaml", main)], &[]).main.unwrap().elements;
    let dock = by_type(&els, "rect").into_iter().find(|e| e.get("color").and_then(T::as_str) == Some("#202020"));
    let dock = dock.expect("dock");
    let (dock_x, dock_w) = (pair(dock, "pos")[0] * W, pair(dock, "size")[0] * W);
    assert!((dock_w - 600.0).abs() < 1.0, "{dock_w}");
    let (x, reach, prompts) = tabs_and_shoulders(&els);
    assert!((x - (dock_x + 100.0)).abs() < 1.5, "after the red box: {x} {dock_x}");
    assert!(reach <= dock_x + dock_w + 0.5 && reach > dock_x + dock_w - 5.0, "{reach} {dock_x} {dock_w}");
    let rb = prompts.iter().find(|p| p[0] > x).expect("rb");
    assert!(reach <= rb[0], "{reach} {rb:?}");
}

/// Prompts laid over the ends of the strip's own box (the strip filling a row the prompts
/// sit in): the strip runs between them.
#[test]
fn the_tab_strip_runs_between_prompts_over_its_ends() {
    let main = wrap_main(
        r##"<Grid>
          <ListBoxEx x:Name="PART_ListGameItems" Height="600" VerticalAlignment="Bottom"/>
          <Grid HorizontalAlignment="Left" VerticalAlignment="Top" Width="700" Height="60" Margin="100,20,0,0">
            <ContentControl ContentTemplate="{DynamicResource ButtonPromptLB}" Width="50" Height="35"
                            HorizontalAlignment="Left" VerticalAlignment="Top"/>
            <FilterPresetSelector x:Name="PART_FilterPresetSelector" Margin="30,0,30,0" FontSize="24"
                                  VerticalAlignment="Top"/>
            <ContentControl ContentTemplate="{DynamicResource ButtonPromptRB}" Width="50" Height="35"
                            HorizontalAlignment="Right" VerticalAlignment="Top"/>
          </Grid>
        </Grid>"##,
    );
    let els = compiled(&[("Views/Main.xaml", main)], &[]).main.unwrap().elements;
    let (x, reach, prompts) = tabs_and_shoulders(&els);
    assert_eq!(prompts.len(), 2);
    assert!((x - 150.0).abs() < 1.0, "after LB: {x}");
    assert!((reach - 750.0).abs() < 1.0, "up to RB: {reach}");
}

#[test]
fn the_list_is_shaped_like_playnites_tile_panel() {
    // Horizontal layout (the default): items are width ÷ 7.5 wide, as many rows as fit.
    let row = tile_panel(Rect::new(0.0, 0.0, 1800.0, 400.0), 1.5);
    assert_eq!(row.rows, 1);
    assert!((row.item[1] - 360.0).abs() < 0.5, "{row:?}");
    let two = tile_panel(Rect::new(0.0, 0.0, 1800.0, 800.0), 1.5);
    assert_eq!((two.rows, two.columns), (2, 7));
    // A box lower than one cover: the covers shrink to it.
    let low = tile_panel(Rect::new(0.0, 0.0, 1800.0, 200.0), 1.5);
    assert_eq!(low.rows, 1);
    assert!((low.item[1] - 200.0).abs() < 0.5);
    // Landscape tiles: two rows in the same box.
    assert_eq!(tile_panel(Rect::new(0.0, 0.0, 1800.0, 400.0), 0.6).rows, 2);
}

#[test]
fn later_list_items_symbols_and_sounds() {
    let main = wrap_main(
        r##"<Grid>
          <ListBoxEx x:Name="PART_ListGameItems" Height="300" VerticalAlignment="Bottom"/>
          <StackPanel Orientation="Horizontal" VerticalAlignment="Top">
            <TextBlock Text="{Binding SelectedGame.Genres[0]}" FontSize="20"/>
            <TextBlock Text="h" FontFamily="Marlett" FontSize="20">
              <TextBlock.Style><Style TargetType="TextBlock">
                <Style.Triggers>
                  <DataTrigger Binding="{Binding SelectedGame.Genres[1]}" Value="{x:Null}">
                    <Setter Property="Visibility" Value="Collapsed"/>
                  </DataTrigger>
                </Style.Triggers>
              </Style></TextBlock.Style>
            </TextBlock>
            <TextBlock Text="{Binding SelectedGame.Genres[1]}" FontSize="20"/>
          </StackPanel>
          <TextBlock Text="r" FontFamily="Marlett" FontSize="20" HorizontalAlignment="Right" VerticalAlignment="Top"/>
          <MediaElement Source="{ThemeFile 'Audio/chime.mp3'}"/>
        </Grid>"##,
    );
    let c = compiled(&[("Views/Main.xaml", main)], &[("Audio/chime.mp3", 1000)]);
    let els = c.main.unwrap().elements;
    let genres: Vec<&Table> =
        els.iter().filter(|e| e.get("text").and_then(T::as_str).is_some_and(|t| t.contains("{game.genre"))).collect();
    assert_eq!(genres.len(), 1, "{genres:?}");
    assert!(text_of(&els, "•").is_none(), "the separator before a missing second genre");
    assert!(text_of(&els, "✕").is_some());
    assert!(!els.iter().any(|e| e.contains_key("video")));
}

#[test]
fn lodestars_hints_give_way_to_the_themes_own_bottom_row() {
    let help: Table =
        toml::from_str("type = \"helpbar\"\npos = [0.06, 0.95]\norigin = [0.0, 0.5]\nsize = [0.85, 0.04]\n").unwrap();
    let band = element_box(&help).unwrap();
    assert!((band.y - 1004.4).abs() < 0.5 && (band.w - 1632.0).abs() < 0.5, "{band:?}");
    let row: Table = toml::from_str(
        "type = \"text\"\ntext = \"Back\"\npos = [0.1, 0.94]\nsize = [0.05, 0.0]\n[style]\nfont_size = 0.02\n",
    )
    .unwrap();
    let top: Table = toml::from_str(
        "type = \"text\"\ntext = \"Name\"\npos = [0.1, 0.1]\nsize = [0.2, 0.0]\n[style]\nfont_size = 0.04\n",
    )
    .unwrap();
    let description: Table = toml::from_str(
        "type = \"text\"\ntext = \"{game.description}\"\nwrap = true\npos = [0.3, 0.66]\nsize = [0.6, 0.32]\n",
    )
    .unwrap();
    assert!(texts_in(&[top.clone(), row], band));
    assert!(!texts_in(&[top, description], band));
}

#[test]
fn notes_about_playnites_list_layout_settings_stay_hidden() {
    let main = wrap_main(
        r##"<Grid>
          <ListBoxEx x:Name="PART_ListGameItems" Height="300" VerticalAlignment="Bottom"/>
          <TextBlock Text="Disable horizontal scrolling" FontSize="40" VerticalAlignment="Top">
            <TextBlock.Style><Style TargetType="TextBlock">
              <Setter Property="Visibility" Value="Collapsed"/>
              <Style.Triggers>
                <DataTrigger Binding="{Settings Fullscreen.HorizontalLayout}" Value="True">
                  <Setter Property="Visibility" Value="Visible"/>
                </DataTrigger>
              </Style.Triggers>
            </Style></TextBlock.Style>
          </TextBlock>
          <TextBlock Text="12:00" FontSize="40" HorizontalAlignment="Right" VerticalAlignment="Top">
            <TextBlock.Style><Style TargetType="TextBlock">
              <Setter Property="Visibility" Value="Collapsed"/>
              <Style.Triggers>
                <DataTrigger Binding="{Settings Fullscreen.ShowClock}" Value="True">
                  <Setter Property="Visibility" Value="Visible"/>
                </DataTrigger>
              </Style.Triggers>
            </Style></TextBlock.Style>
          </TextBlock>
        </Grid>"##,
    );
    let els = compiled(&[("Views/Main.xaml", main)], &[]).main.unwrap().elements;
    assert!(text_of(&els, "Disable horizontal scrolling").is_none());
    assert!(text_of(&els, "12:00").is_some());
}

fn wrap_details(body: &str) -> String {
    wrap_main(body).replace("x:Type Main}", "x:Type GameDetails}")
}

#[test]
fn an_options_button_showing_only_an_icon_is_not_a_second_play_button() {
    let details = wrap_details(
        r##"<StackPanel Orientation="Horizontal" VerticalAlignment="Bottom" HorizontalAlignment="Left" Margin="100,0,0,100">
          <ButtonEx x:Name="PART_ButtonContext" Width="300" Height="60"/>
          <ButtonEx x:Name="PART_ButtonOptions" Width="90" Height="60">
            <Button.Content>
              <TextBlock Text="&#xe712;" FontFamily="Segoe Fluent Icons, Segoe MDL2 Assets" FontSize="24"/>
            </Button.Content>
          </ButtonEx>
          <ButtonEx x:Name="PART_ButtonOptions" Width="90" Height="60">
            <Button.Content><TextBlock Text="&#xe700;" FontFamily="Segoe MDL2 Assets"/></Button.Content>
          </ButtonEx>
        </StackPanel>"##,
    );
    let c = compiled(&[("Views/GameDetails.xaml", details)], &[]);
    let els = c.details.expect("details").elements;
    let texts: Vec<&str> = els.iter().filter_map(|e| e.get("text").and_then(T::as_str)).collect();
    assert_eq!(texts.iter().filter(|t| **t == "Play").count(), 1, "{texts:?}");
    assert!(texts.contains(&"…") && texts.contains(&"☰"), "{texts:?}");
    assert_eq!(icon_symbol("\u{e10c}"), "…");
}

#[test]
fn button_prompt_templates_become_the_users_button_glyphs() {
    let main = wrap_main(
        r##"<Grid>
          <ListBoxEx x:Name="PART_ListGameItems" Height="600" VerticalAlignment="Top"/>
          <StackPanel Orientation="Horizontal" VerticalAlignment="Bottom" Margin="56,0,0,30">
            <ContentControl ContentTemplate="{DynamicResource ButtonPromptA}" Height="30" Margin="0,0,10,0"/>
            <TextBlock Text="Select" FontSize="26"/>
            <ContentControl ContentTemplate="{DynamicResource ButtonPromptLB}" Height="34" Margin="34,0,0,0"/>
            <ContentControl ContentTemplate="{DynamicResource ButtonPromptBack}" Height="34"/>
            <ContentControl ContentTemplate="{DynamicResource ButtonPromptLT}" Height="34"/>
            <TextBlock Text="Tabs" FontSize="26"/>
          </StackPanel>
        </Grid>"##,
    );
    let els = compiled(&[("Views/Main.xaml", main)], &[]).main.unwrap().elements;
    let prompts: Vec<(&str, f32)> = by_type(&els, "helpbar")
        .into_iter()
        .filter(|e| e.get("glyphs").and_then(T::as_bool) == Some(true))
        .map(|e| (e["buttons"].as_array().unwrap()[0].as_str().unwrap(), pair(e, "size")[1] * 1080.0))
        .collect();
    // A, LB and Back (Select); a trigger has no Lodestar button and is left out.
    assert_eq!(prompts.iter().map(|p| p.0).collect::<Vec<_>>(), ["a", "lb", "select"], "{els:#?}");
    assert!((prompts[0].1 - 30.0).abs() < 0.5 && (prompts[1].1 - 34.0).abs() < 0.5);
    assert!(text_of(&els, "Select").is_some());
    for e in &els {
        let toml = toml::to_string(e).unwrap();
        let _: crate::theme::model::Element = toml::from_str(&toml).unwrap_or_else(|err| panic!("{err}\n{toml}"));
    }
}

#[test]
fn a_label_whose_value_lodestar_lacks_is_left_out_too() {
    let main = wrap_main(
        r##"<Grid>
          <ListBoxEx x:Name="PART_ListGameItems" Height="600" VerticalAlignment="Bottom"/>
          <StackPanel Orientation="Horizontal" VerticalAlignment="Top">
            <TextBlock Text="Sorting:" FontSize="26"/>
            <TextBlock Text="{Binding AppSettings.Fullscreen.ViewSettings.SortingOrder, Converter={StaticResource SortingOrderToStringConverter}}" FontSize="26"/>
            <TextBlock Text="Released:" FontSize="26" Margin="30,0,0,0"/>
            <TextBlock Text="{Binding SelectedGame.ReleaseYear}" FontSize="26"/>
          </StackPanel>
        </Grid>"##,
    );
    let els = compiled(&[("Views/Main.xaml", main)], &[]).main.unwrap().elements;
    assert!(text_of(&els, "Sorting:").is_none());
    assert!(text_of(&els, "Released:").is_some() && text_of(&els, "{game.year}").is_some());
}

#[test]
fn a_per_game_text_may_grow_up_to_the_next_thing_beside_it() {
    let main = wrap_main(
        r##"<Grid>
          <ListBoxEx x:Name="PART_ListGameItems" Height="600" VerticalAlignment="Bottom"/>
          <TextBlock Text="{Binding SelectedGame.ReleaseYear}" FontSize="26" HorizontalAlignment="Left" VerticalAlignment="Top" Margin="100,100,0,0"/>
          <TextBlock Text="Next" FontSize="26" HorizontalAlignment="Left" VerticalAlignment="Top" Margin="600,100,0,0"/>
          <TextBlock x:Name="PART_TextClock" FontSize="30" HorizontalAlignment="Right" VerticalAlignment="Top" Margin="0,300,40,0"/>
        </Grid>"##,
    );
    let els = compiled(&[("Views/Main.xaml", main)], &[]).main.unwrap().elements;
    let year = text_of(&els, "{game.year}").unwrap();
    assert!(near(pair(year, "pos")[0], 100.0 / 1920.0));
    assert!(near(pair(year, "size")[0], 500.0 / 1920.0), "{year:?}");
    // Nothing beside the clock: it may grow to the screen's edge (right-aligned, leftwards).
    let clock = by_type(&els, "datetime")[0];
    assert!(pair(clock, "size")[0] > 0.5, "{clock:?}");
}

#[test]
fn a_column_of_texts_in_a_row_is_as_wide_as_its_longest_value() {
    STACKS.with(|f| f.set(true));
    let details = wrap_details(
        r##"<StackPanel Orientation="Horizontal" HorizontalAlignment="Left" VerticalAlignment="Top" Margin="100,100,0,0">
          <StackPanel>
            <TextBlock Text="Last Played" FontSize="22"/>
            <TextBlock Text="{Binding Game.LastActivity}" FontSize="22" HorizontalAlignment="Center"/>
          </StackPanel>
          <StackPanel Width="200" Margin="40,0,0,0">
            <TextBlock Text="Platform" FontSize="22"/>
            <TextBlock Text="{Binding Game.Platforms}" FontSize="22"/>
          </StackPanel>
        </StackPanel>"##,
    );
    let c = compiled(&[("Views/GameDetails.xaml", details)], &[]);
    STACKS.with(|f| f.set(false));
    let els = c.details.unwrap().elements;
    // The page's row (Playnite's own main screen is drawn under the page).
    for s in by_type(&els, "stack") {
        eprintln!("STACK {}", toml::to_string(s).unwrap());
    }
    let row = by_type(&els, "stack").into_iter().find(|s| near(pair(s, "pos")[0], 100.0 / 1920.0)).expect("row");
    let cols = row["children"].as_array().unwrap();
    let col = cols[0].as_table().unwrap();
    // No width of its own: as wide as its texts, which span it; the centred value stays centred.
    assert_eq!(pair(col, "size")[0], 0.0);
    assert_eq!(col.get("align").and_then(T::as_str), Some("stretch"));
    let kids = col["children"].as_array().unwrap();
    assert!(kids.iter().all(|k| pair(k.as_table().unwrap(), "size")[0] == 0.0), "{kids:#?}");
    assert_eq!(kids[1]["style"]["align"].as_str(), Some("center"));
    // A column with a Width keeps it.
    let fixed = cols[1].as_table().unwrap();
    assert!(near(pair(fixed, "size")[0], 200.0 / 1920.0), "{fixed:?}");
}

fn rects_colored<'a>(els: &'a [Table], color: &str) -> Vec<&'a Table> {
    by_type(els, "rect").into_iter().filter(|r| r.get("color").and_then(T::as_str) == Some(color)).collect()
}

#[test]
fn the_details_page_opens_with_the_focus_on_its_play_button() {
    // Two black veils that fade in while their panel holds the focus: only the one around
    // the Play button shows, at the opacity its fade ends at.
    let details = wrap_details(
        r##"<Grid>
          <Grid x:Name="Actions" VerticalAlignment="Bottom" Height="200">
            <Border Background="#000000">
              <Border.Style><Style TargetType="Border">
                <Setter Property="Opacity" Value="0"/>
                <Style.Triggers>
                  <DataTrigger Binding="{Binding ElementName=Actions, Path=IsKeyboardFocusWithin}" Value="True">
                    <DataTrigger.EnterActions><BeginStoryboard><Storyboard>
                      <DoubleAnimation Storyboard.TargetProperty="Opacity" From="0" To="0.5" Duration="0:0:0.2"/>
                    </Storyboard></BeginStoryboard></DataTrigger.EnterActions>
                  </DataTrigger>
                </Style.Triggers>
              </Style></Border.Style>
            </Border>
            <ButtonEx x:Name="PART_ButtonContext" Width="300" Height="60"/>
          </Grid>
          <Grid x:Name="Media" VerticalAlignment="Top" Height="300">
            <Border Background="#000001">
              <Border.Style><Style TargetType="Border">
                <Setter Property="Opacity" Value="0"/>
                <Style.Triggers>
                  <DataTrigger Binding="{Binding ElementName=Media, Path=IsKeyboardFocusWithin}" Value="True">
                    <DataTrigger.EnterActions><BeginStoryboard><Storyboard>
                      <DoubleAnimation Storyboard.TargetProperty="Opacity" From="0" To="1" Duration="0:0:0.2"/>
                    </Storyboard></BeginStoryboard></DataTrigger.EnterActions>
                  </DataTrigger>
                </Style.Triggers>
              </Style></Border.Style>
            </Border>
          </Grid>
        </Grid>"##,
    );
    let els = compiled(&[("Views/GameDetails.xaml", details)], &[]).details.expect("details").elements;
    let veil = rects_colored(&els, "#000000");
    assert_eq!(veil.len(), 1, "{els:#?}");
    assert!((veil[0].get("opacity").and_then(T::as_float).unwrap_or(1.0) - 0.5).abs() < 1e-3);
    assert!(rects_colored(&els, "#000001").is_empty(), "the media veil never fades in");
}

#[test]
fn the_details_page_sits_in_the_main_screen_as_playnite_shows_it() {
    // The main screen's picture brightens and its title fades out as the page opens; the
    // list stays behind the page.
    let main = wrap_main(
        r##"<Grid>
          <FadeImage x:Name="PART_ImageBackground" Opacity="0.4">
            <FadeImage.Style><Style TargetType="FadeImage"><Style.Triggers>
              <DataTrigger Binding="{Binding ElementName=PART_ElemGameDetails, Path=Visibility}" Value="Visible">
                <DataTrigger.EnterActions><BeginStoryboard><Storyboard>
                  <DoubleAnimation Storyboard.TargetProperty="Opacity" Duration="0:0:1" To="1"/>
                </Storyboard></BeginStoryboard></DataTrigger.EnterActions>
              </DataTrigger>
            </Style.Triggers></Style></FadeImage.Style>
          </FadeImage>
          <TextBlock Text="Library" FontSize="40" VerticalAlignment="Top">
            <TextBlock.Style><Style TargetType="TextBlock"><Style.Triggers>
              <DataTrigger Binding="{Binding ElementName=PART_ElemGameDetails, Path=Visibility}" Value="Visible">
                <DataTrigger.EnterActions><BeginStoryboard><Storyboard>
                  <DoubleAnimation Storyboard.TargetProperty="Opacity" Duration="0:0:0.1" To="0"/>
                </Storyboard></BeginStoryboard></DataTrigger.EnterActions>
              </DataTrigger>
            </Style.Triggers></Style></TextBlock.Style>
          </TextBlock>
          <ListBoxEx x:Name="PART_ListGameItems" Height="400" VerticalAlignment="Bottom"/>
          <GameDetails x:Name="PART_ElemGameDetails"/>
        </Grid>"##,
    );
    let details = wrap_details(
        r##"<StackPanel VerticalAlignment="Bottom" Margin="100,0,0,100">
          <TextBlock Text="{Binding DisplayName}" FontSize="50"/>
          <ButtonEx x:Name="PART_ButtonContext" Width="300" Height="60"/>
        </StackPanel>"##,
    );
    let c = compiled(&[("Views/Main.xaml", main.clone()), ("Views/GameDetails.xaml", details)], &[]);
    let els = c.details.expect("details").elements;
    let hero = by_type(&els, "image")
        .into_iter()
        .find(|i| i.get("path").and_then(T::as_str) == Some("{art.hero}"))
        .expect("the main screen's picture");
    assert!(hero.get("opacity").is_none(), "{hero:?}");
    assert!(text_of(&els, "Library").is_none());
    assert!(text_of(&els, "{game.name}").is_some());
    assert!(!els.iter().any(|e| e.get("name").and_then(T::as_str) == Some("pn_list")));
    // On the main screen the picture keeps its own opacity and the title shows.
    let main_els = c.main.expect("main").elements;
    assert!(text_of(&main_els, "Library").is_some());

    // A page that paints its own opaque background hides the main screen under it.
    let opaque = wrap_details(
        r##"<Grid Background="#101010">
          <TextBlock Text="{Binding DisplayName}" FontSize="50"/>
        </Grid>"##,
    );
    let els = compiled(&[("Views/Main.xaml", main), ("Views/GameDetails.xaml", opaque)], &[])
        .details
        .expect("details")
        .elements;
    assert!(by_type(&els, "image").is_empty(), "{els:#?}");
    assert!(text_of(&els, "{game.name}").is_some());
}

#[test]
fn a_buttons_face_comes_from_its_template_in_its_focus_state() {
    let details = wrap_details(
        r##"<Grid>
          <Grid.Resources>
            <Style x:Key="Pill" TargetType="ButtonEx">
              <Setter Property="Background" Value="#40808080"/>
              <Setter Property="Template"><Setter.Value>
                <ControlTemplate TargetType="Button">
                  <Grid>
                    <Border x:Name="Face" Background="{TemplateBinding Background}" CornerRadius="30"/>
                    <Border x:Name="Ring" BorderBrush="#FF00FF00" BorderThickness="3" CornerRadius="34" Visibility="Hidden"/>
                    <ContentPresenter/>
                  </Grid>
                  <ControlTemplate.Triggers>
                    <Trigger Property="IsFocused" Value="True">
                      <Setter Property="Background" Value="#F2F2F2" TargetName="Face"/>
                      <Setter Property="Visibility" Value="Visible" TargetName="Ring"/>
                      <Setter Property="Foreground" Value="#111111"/>
                    </Trigger>
                  </ControlTemplate.Triggers>
                </ControlTemplate>
              </Setter.Value></Setter>
            </Style>
          </Grid.Resources>
          <StackPanel Orientation="Horizontal" VerticalAlignment="Bottom" Margin="100,0,0,100">
            <ButtonEx x:Name="PART_ButtonContext" Style="{StaticResource Pill}" Width="170" Height="60"/>
            <ButtonEx x:Name="PART_ButtonOptions" Style="{StaticResource Pill}" Width="90" Height="60">
              <TextBlock FontSize="40">&#x2022;&#x2022;&#x2022;</TextBlock>
            </ButtonEx>
          </StackPanel>
        </Grid>"##,
    );
    let els = compiled(&[("Views/GameDetails.xaml", details)], &[]).details.expect("details").elements;
    // The Play button has the focus: white face, the ring, dark label.
    let face = rects_colored(&els, "#f2f2f2");
    assert_eq!(face.len(), 1, "{els:#?}");
    assert_eq!(face[0].get("border_color").and_then(T::as_str), Some("#00ff00"));
    assert!(face[0].get("corner_radius").is_some());
    let install = text_of(&els, "Install").expect("install");
    assert_eq!(install["style"].get("color").and_then(T::as_str), Some("#111111"));
    // The options button hasn't: its resting face and the label written inside it, whole.
    assert_eq!(rects_colored(&els, "#80808040").len(), 1, "{els:#?}");
    let dots = text_of(&els, "•••").expect("dots");
    assert!((dots["style"]["font_size"].as_float().unwrap() * 1080.0 - 40.0).abs() < 0.5);
}

#[test]
fn lists_of_the_games_tags_add_on_lists_and_separators() {
    let details = wrap_details(
        r##"<StackPanel VerticalAlignment="Top" Margin="100,100,0,0">
          <ItemsControl ItemsSource="{Binding Game.Tags}">
            <ItemsControl.ItemTemplate><DataTemplate>
              <Border Padding="12,6"><TextBlock Text="{Binding Name}" FontSize="18"/></Border>
            </DataTemplate></ItemsControl.ItemTemplate>
          </ItemsControl>
          <StackPanel Orientation="Horizontal">
            <TextBlock Text="{PluginSettings Plugin=HowLongToBeat, Path=MainStory}" FontSize="30"/>
            <TextBlock Text="/" FontSize="30"/>
            <TextBlock Text="{PluginSettings Plugin=HowLongToBeat, Path=Completionist}" FontSize="30"/>
          </StackPanel>
          <Image x:Name="GameLogo" Source="{Binding IconObject}" Width="64" Height="64"/>
          <ButtonEx x:Name="PART_ButtonContext" Width="200" Height="50"/>
          <ListView Width="1920" Height="600" ItemsSource="{PluginSettings Plugin=ScreenshotsVisualizer, Path=ListScreenshots}"/>
        </StackPanel>"##,
    );
    let c = compiled(&[("Views/GameDetails.xaml", details)], &[]);
    assert!(c.details_used(), "an add-on's empty list takes no room: {:?}", c.details.as_ref().map(|d| &d.stats));
    let els = c.details.expect("details").elements;
    let tags = text_of(&els, "{game.tags}").expect("tags");
    assert!((tags["style"]["font_size"].as_float().unwrap() * 1080.0 - 18.0).abs() < 0.5);
    assert!(text_of(&els, "/").is_none());
    // The Image called "GameLogo" shows the game's icon (Playnite's main screen under the page
    // adds its picture).
    let pictures: Vec<&str> = by_type(&els, "image").iter().filter_map(|i| i.get("path").and_then(T::as_str)).collect();
    assert!(pictures.contains(&"{art.icon}") && !pictures.contains(&"{art.logo}"), "{pictures:?}");
}

#[test]
fn a_theme_details_page_goes_with_a_layout_family_only_in_place_of_its_main_screen() {
    use super::super::fill::Family;
    let main = wrap_main(r#"<Grid><TextBlock Text="No list here"/></Grid>"#);
    let page = |panel_opacity: &str| {
        wrap_details(&format!(
            r#"<Grid Opacity="{panel_opacity}">
              <TextBlock Text="{{Binding DisplayName}}" FontSize="50"/>
              <ButtonEx x:Name="PART_ButtonContext" Width="200" Height="50" VerticalAlignment="Bottom"/>
            </Grid>"#
        ))
    };
    let c = compiled(&[("Views/Main.xaml", main.clone()), ("Views/GameDetails.xaml", page("1"))], &[]);
    assert!(!c.main_ok());
    assert!(c.details_with(Family::Row) && c.details_with(Family::Translated));
    let c = compiled(&[("Views/GameDetails.xaml", page("1"))], &[]);
    assert!(c.main_ok());
    assert!(c.details_with(Family::Translated) && !c.details_with(Family::Row));
    // A page whose panel only an add-on fades in shows no Play button: Lodestar's page instead.
    let c = compiled(&[("Views/Main.xaml", main), ("Views/GameDetails.xaml", page("0"))], &[]);
    assert!(!c.details_used());
}
