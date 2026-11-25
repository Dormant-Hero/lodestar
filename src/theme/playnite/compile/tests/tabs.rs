//! The filter preset strip drawn the way the theme's `ItemFilterQuickPreset` template draws
//! each preset: pictures by the preset's exact name, sized as WPF sizes them, the template's
//! text look and plates, presets the template hides, and a strip that fades out on the page.

use super::*;

/// A theme whose preset buttons use `template` (a ControlTemplate's content and triggers),
/// with extra `setters` on the button's style; `panel` is the strip's items host.
fn presets(setters: &str, template: &str, panel: &str) -> String {
    format!(
        r#"<ResourceDictionary {NS}>
  <Style x:Key="ItemFilterQuickPreset" TargetType="CheckBoxEx">
    {setters}
    <Setter Property="Template"><Setter.Value>
      <ControlTemplate TargetType="CheckBoxEx">
        {template}
      </ControlTemplate>
    </Setter.Value></Setter>
  </Style>
  <Style TargetType="{{x:Type FilterPresetSelector}}">
    <Setter Property="Template"><Setter.Value>
      <ControlTemplate TargetType="{{x:Type FilterPresetSelector}}">
        {panel}
      </ControlTemplate>
    </Setter.Value></Setter>
  </Style>
</ResourceDictionary>"#
    )
}

const ROW: &str = r#"<ItemsControlEx x:Name="PART_ItemsFilterPresets">
  <ItemsControl.ItemsPanel><ItemsPanelTemplate><StackPanel Orientation="Horizontal"/></ItemsPanelTemplate></ItemsControl.ItemsPanel>
</ItemsControlEx>"#;

/// A main screen with the strip at the top (black text, as themes often write it on the
/// selector) above the game list.
fn main_with_strip() -> String {
    wrap_main(
        r#"<Grid>
  <FilterPresetSelector x:Name="PART_FilterPresetSelector" VerticalAlignment="Top" Foreground="Black" FontSize="20"/>
  <ListBoxEx x:Name="PART_ListGameItems" Height="600" VerticalAlignment="Bottom"/>
  <GameDetails x:Name="PART_ElemGameDetails"/>
</Grid>"#,
    )
}

/// The strip: the stack whose children show by the open collection.
fn strip(els: &[Table]) -> &Table {
    let tabbed = |s: &&Table| toml::to_string(*s).is_ok_and(|t| t.contains("collection.key"));
    by_type(els, "stack").into_iter().find(tabbed).unwrap_or_else(|| panic!("no strip: {els:#?}"))
}

/// The strip's button for collection `key` at rest or open.
fn button<'a>(strip: &'a Table, key: &str, open: bool) -> Option<&'a Table> {
    let rule = format!("{{collection.key}} == {key}");
    let field = if open { "visible_if" } else { "hidden_if" };
    strip
        .get("children")?
        .as_array()?
        .iter()
        .filter_map(T::as_table)
        .find(|c| c.get(field).and_then(T::as_str) == Some(rule.as_str()))
}

fn kids(t: &Table) -> Vec<&Table> {
    t.get("children").and_then(T::as_array).map(|a| a.iter().filter_map(T::as_table).collect()).unwrap_or_default()
}

/// A button that keeps the room of the picture its preset lacks and draws nothing in it:
/// that room's size, px (rounded as the theme file writes it).
fn blank_room(button: &Table) -> Option<[f32; 2]> {
    let [room] = kids(button)[..] else { return None };
    let empty = room.get("type").and_then(T::as_str) == Some("stack") && kids(room).is_empty();
    empty.then(|| pair(room, "size")).map(|s| [s[0] * 1920.0, s[1] * 1080.0])
}

fn num(t: &Table, k: &str) -> f32 {
    t.get(k).and_then(T::as_float).unwrap_or_else(|| panic!("{k}: {t:?}")) as f32
}

/// Playnite finds a preset's picture by the preset's exact name: a tab takes the picture for
/// its own label, else for Playnite's default preset of the same games ("Favorites"), never
/// one for a near name ("Recent"); without one it shows its name. A picture without a size
/// of its own fills the height its holder gives it, less its margins, as wide as its aspect
/// makes it.
#[test]
fn preset_pictures_go_by_exact_names_and_fill_their_cell() {
    let template = r#"<Grid Height="60">
  <Image x:Name="Icon" Margin="10" Source="{ThemeFileBinding Name, PathFormat='Icons/Presets/{0}.png'}"/>
  <TextBlock Text="{Binding Name}">
    <TextBlock.Style><Style TargetType="TextBlock">
      <Setter Property="Visibility" Value="Collapsed"/>
      <Style.Triggers>
        <DataTrigger Binding="{Binding ElementName=Icon, Path=Source}" Value="{x:Null}">
          <Setter Property="Visibility" Value="Visible"/>
        </DataTrigger>
      </Style.Triggers>
    </Style></TextBlock.Style>
  </TextBlock>
</Grid>"#;
    let c = compiled(
        &[
            ("CustomControls/FilterPresetSelector.xaml", presets("", template, ROW)),
            ("Views/Main.xaml", main_with_strip()),
        ],
        &[
            ("Icons/Presets/All Games.png", 100),
            ("Icons/Presets/All.png", 100),
            ("Icons/Presets/Favorites.png", 100),
            ("Icons/Presets/Recent.png", 100),
        ],
    );
    let els = c.main.expect("main").elements;
    let all = kids(button(strip(&els), "all", false).expect("all games"));
    assert_eq!(all.len(), 1, "{all:#?}");
    assert_eq!(all[0].get("type").and_then(T::as_str), Some("image"));
    // 60 px less 2 × 10 px margin, twice as wide (the test pictures are 400 × 200).
    let size = pair(all[0], "size");
    assert!(near(size[1], 40.0 / 1080.0) && near(size[0], 80.0 / 1920.0), "{size:?}");
    assert_eq!(all[0].get("path").and_then(T::as_str), Some("images/view/Icons_Presets_All_Games.png"));
    let favourites = kids(button(strip(&els), "favorites", false).expect("favourites"));
    assert_eq!(favourites[0].get("type").and_then(T::as_str), Some("image"), "{favourites:#?}");
    let recent = kids(button(strip(&els), "recent", false).expect("recent"));
    assert_eq!(recent[0].get("text").and_then(T::as_str), Some("Continue Playing"), "{recent:#?}");
    assert_eq!(c.images.len(), 2, "only the pictures shown are copied: {:?}", c.images);
}

/// The name takes the template's own text look (its style's colour and size beat the black
/// text the selector asks for), the open preset its trigger's look, and the plate behind it
/// the template's fill and corners.
#[test]
fn presets_take_the_templates_text_and_plates() {
    let template = r##"<Border x:Name="Plate" Background="#333333" CornerRadius="10" Padding="12,4">
  <TextBlock x:Name="Label" Text="{Binding Name, Converter={StaticResource StringToUpperCaseConverter}}">
    <TextBlock.Style><Style TargetType="TextBlock">
      <Setter Property="Foreground" Value="#EEEEEE"/>
      <Setter Property="FontSize" Value="30"/>
    </Style></TextBlock.Style>
  </TextBlock>
</Border>
<ControlTemplate.Triggers>
  <Trigger Property="IsFocused" Value="True"><Setter TargetName="Plate" Property="Background" Value="Red"/></Trigger>
  <Trigger Property="IsChecked" Value="True">
    <Setter TargetName="Plate" Property="Background" Value="#3A7BFF"/>
    <Setter TargetName="Label" Property="FontWeight" Value="Bold"/>
  </Trigger>
</ControlTemplate.Triggers>"##;
    let c = compiled(
        &[
            ("CustomControls/FilterPresetSelector.xaml", presets("", template, ROW)),
            ("Views/Main.xaml", main_with_strip()),
        ],
        &[],
    );
    let els = c.main.expect("main").elements;
    let s = strip(&els);
    let (rest, open) = (button(s, "recent", false).expect("rest"), button(s, "recent", true).expect("open"));
    assert_eq!(rest.get("background").and_then(T::as_str), Some("#333333"), "{rest:?}");
    assert_eq!(open.get("background").and_then(T::as_str), Some("#3a7bff"), "{open:?}");
    assert!(near(num(rest, "corner_radius"), 10.0 / 1080.0));
    assert!(near(pair(rest, "padding")[0], 12.0 / 1080.0), "{rest:?}");
    let label = kids(rest)[0];
    let style = label.get("style").and_then(T::as_table).expect("style");
    assert_eq!(style.get("color").and_then(T::as_str), Some("#eeeeee"));
    assert!(near(num(style, "font_size"), 30.0 / 1080.0));
    assert_eq!(style.get("uppercase").and_then(T::as_bool), Some(true));
    assert!(style.get("font").is_none(), "the resting name isn't bold: {style:?}");
    let open_style = kids(open)[0].get("style").and_then(T::as_table).expect("style");
    assert_eq!(open_style.get("font").and_then(T::as_str), Some("bold"));
}

/// A template that shows only the first presets (`AlternationIndex` 0 and 1 of a strip that
/// numbers its items) has no button for the others.
#[test]
fn presets_the_template_collapses_have_no_tab() {
    let setters = r#"<Setter Property="Visibility" Value="Collapsed"/>"#;
    let template = r#"<TextBlock Text="{Binding Name}"/>
<ControlTemplate.Triggers>
  <Trigger Property="ListBox.AlternationIndex" Value="0"><Setter Property="Visibility" Value="Visible"/></Trigger>
  <Trigger Property="ListBox.AlternationIndex" Value="1"><Setter Property="Visibility" Value="Visible"/></Trigger>
</ControlTemplate.Triggers>"#;
    let panel = ROW.replace(r#"x:Name="PART_ItemsFilterPresets""#, r#"x:Name="PART_ItemsFilterPresets" AlternationCount="{Binding Items.Count, RelativeSource={RelativeSource Self}}""#);
    let c = compiled(
        &[
            ("CustomControls/FilterPresetSelector.xaml", presets(setters, template, &panel)),
            ("Views/Main.xaml", main_with_strip()),
        ],
        &[],
    );
    let els = c.main.expect("main").elements;
    let s = strip(&els);
    assert!(button(s, "all", false).is_some() && button(s, "recent", true).is_some());
    assert!(button(s, "favorites", false).is_none() && button(s, "installed", true).is_none(), "{s:#?}");
}

/// A preset button that fades out while the game's page is open leaves the page without a
/// strip; the main screen keeps it.
#[test]
fn a_strip_fading_out_on_the_page_is_not_drawn_there() {
    let template = r#"<TextBlock Text="{Binding Name}"/>
<ControlTemplate.Triggers>
  <DataTrigger Binding="{Binding ElementName=PART_ElemGameDetails, Path=Visibility}" Value="Visible">
    <DataTrigger.EnterActions><BeginStoryboard><Storyboard>
      <DoubleAnimation Storyboard.TargetProperty="Opacity" Duration="0:0:0.1" To="0"/>
    </Storyboard></BeginStoryboard></DataTrigger.EnterActions>
  </DataTrigger>
</ControlTemplate.Triggers>"#;
    let details = wrap_details(r#"<TextBlock Text="{Binding DisplayName}" FontSize="50" VerticalAlignment="Bottom"/>"#);
    let c = compiled(
        &[
            ("CustomControls/FilterPresetSelector.xaml", presets("", template, ROW)),
            ("Views/Main.xaml", main_with_strip()),
            ("Views/GameDetails.xaml", details),
        ],
        &[],
    );
    let page = c.details.expect("details").elements;
    assert!(text_of(&page, "{game.name}").is_some());
    let tabbed = |e: &&Table| toml::to_string(*e).is_ok_and(|t| t.contains("collection.key"));
    assert!(!page.iter().any(|e| tabbed(&e)), "{page:#?}");
    strip(&c.main.expect("main").elements);
}

/// A template that marks the open preset only while it has the focus: Lodestar's own cue
/// (the open name bold in the accent colour) keeps it apart.
#[test]
fn an_open_preset_drawn_like_the_others_gets_lodestars_cue() {
    let template = r#"<TextBlock x:Name="Label" Text="{Binding Name}" Foreground="White"/>
<ControlTemplate.Triggers>
  <Trigger Property="IsFocused" Value="True"><Setter TargetName="Label" Property="FontWeight" Value="Bold"/></Trigger>
</ControlTemplate.Triggers>"#;
    let c = compiled(
        &[
            ("CustomControls/FilterPresetSelector.xaml", presets("", template, ROW)),
            ("Views/Main.xaml", main_with_strip()),
        ],
        &[],
    );
    let els = c.main.expect("main").elements;
    let s = strip(&els);
    let font = |open: bool| {
        let b = button(s, "installed", open).expect("button");
        kids(b)[0]
            .get("style")
            .and_then(T::as_table)
            .and_then(|st| st.get("font"))
            .and_then(T::as_str)
            .map(str::to_string)
    };
    assert_eq!(font(false), None);
    assert_eq!(font(true).as_deref(), Some("bold"));
}

/// A preset strip that stacks its presets down the screen and shows the theme's picture per
/// preset name instead of the name (an icon rail). A preset without a picture shows no name
/// either (the template never shows one), only the picture's empty room.
#[test]
fn filter_presets_take_the_themes_pictures_and_direction() {
    let template = r#"<StackPanel>
  <TextBlock Text="{Binding Name}" Visibility="Collapsed"/>
  <Image x:Name="Icon" Width="48" Height="48" Opacity="0.4"
         Source="{ThemeFileBinding Name, PathFormat='Icons/Presets/{0}.png'}"/>
</StackPanel>
<ControlTemplate.Triggers>
  <Trigger Property="IsChecked" Value="True"><Setter Property="Opacity" Value="1" TargetName="Icon"/></Trigger>
</ControlTemplate.Triggers>"#;
    let panel = ROW.replace("Horizontal", "Vertical");
    let main = wrap_main(
        r#"<Grid>
  <ListBoxEx x:Name="PART_ListGameItems" Height="300" VerticalAlignment="Bottom"/>
  <FilterPresetSelector x:Name="PART_FilterPresetSelector" HorizontalAlignment="Left" VerticalAlignment="Center" Margin="20,0,0,0"/>
</Grid>"#,
    );
    let c = compiled(
        &[("CustomControls/FilterPresetSelector.xaml", presets("", template, &panel)), ("Views/Main.xaml", main)],
        &[("Icons/Presets/All Games.png", 100), ("Icons/Presets/Favourites.png", 100)],
    );
    let els = c.main.expect("main").elements;
    let s = strip(&els);
    assert_eq!(s.get("direction").and_then(T::as_str), Some("vertical"));
    let parts: Vec<&Table> = kids(s).into_iter().flat_map(kids).collect();
    let pictures: Vec<&Table> =
        parts.iter().copied().filter(|k| k.get("type").and_then(T::as_str) == Some("image")).collect();
    // All Games and Favourites as pictures (resting and open), the other two empty rooms.
    assert_eq!(pictures.len(), 4, "{parts:#?}");
    assert_eq!(c.images.len(), 2, "{:?}", c.images);
    assert!(near(num(pictures[0], "opacity"), 0.4) && pictures[1].get("opacity").is_none(), "{pictures:#?}");
    let installed = button(s, "installed", false).expect("installed");
    let room = blank_room(installed).unwrap_or_else(|| panic!("{installed:#?}"));
    assert!(room.iter().all(|v| (v - 48.0).abs() < 0.5), "{room:?}");
    // Only the open collection that is none of the tabs is named.
    let names: Vec<&str> = parts.iter().filter_map(|k| k.get("text").and_then(T::as_str)).collect();
    assert_eq!(names, ["{collection.name}"]);
}

/// An icon column whose names show only while the player is in it (the strip holding the
/// focus), with a glyph standing in for a missing picture: a preset without a picture shows
/// no name, only the room of the glyph Lodestar can't draw.
#[test]
fn an_icon_column_shows_no_name_where_a_picture_is_missing() {
    let template = r#"<StackPanel Orientation="Horizontal" Height="40">
  <Grid>
    <Image x:Name="FilterIcon" Width="30" Height="30" Source="{ThemeFileBinding Name, PathFormat='FilterIcons/{0}.png'}"/>
    <TextBlock Text="&#xE71C;" FontFamily="Segoe MDL2 Assets" FontSize="25">
      <TextBlock.Style><Style TargetType="TextBlock">
        <Setter Property="Visibility" Value="Collapsed"/>
        <Style.Triggers>
          <DataTrigger Binding="{Binding ElementName=FilterIcon, Path=Source}" Value="{x:Null}">
            <Setter Property="Visibility" Value="Visible"/>
          </DataTrigger>
        </Style.Triggers>
      </Style></TextBlock.Style>
    </TextBlock>
  </Grid>
  <Grid Margin="25,0,10,0">
    <Grid.Style><Style TargetType="Grid">
      <Setter Property="Visibility" Value="Collapsed"/>
      <Style.Triggers>
        <DataTrigger Binding="{Binding ElementName=FilterPresets, Path=IsKeyboardFocusWithin}" Value="True">
          <Setter Property="Visibility" Value="Visible"/>
        </DataTrigger>
      </Style.Triggers>
    </Style></Grid.Style>
    <TextBlock Text="{Binding Name}"/>
  </Grid>
</StackPanel>"#;
    let panel = ROW.replace("Horizontal", "Vertical");
    let main = wrap_main(
        r#"<Grid>
  <ListBoxEx x:Name="PART_ListGameItems" Height="300" VerticalAlignment="Bottom"/>
  <Grid x:Name="FilterPresets" HorizontalAlignment="Left">
    <FilterPresetSelector x:Name="PART_FilterPresetSelector" VerticalAlignment="Center"/>
  </Grid>
</Grid>"#,
    );
    let c = compiled(
        &[("CustomControls/FilterPresetSelector.xaml", presets("", template, &panel)), ("Views/Main.xaml", main)],
        &[("FilterIcons/All.png", 100), ("FilterIcons/Recently Played.png", 100), ("FilterIcons/Favorites.png", 100)],
    );
    let els = c.main.expect("main").elements;
    let s = strip(&els);
    for open in [false, true] {
        let installed = button(s, "installed", open).expect("installed");
        let room = blank_room(installed).unwrap_or_else(|| panic!("{installed:#?}"));
        assert!(room.iter().all(|v| (v - 30.0).abs() < 0.5), "{room:?}");
        let all = kids(button(s, "all", open).expect("all"));
        assert_eq!(all.len(), 1, "{all:#?}");
        assert_eq!(all[0].get("type").and_then(T::as_str), Some("image"));
    }
}

/// A selector as wide as the screen whose template centres its items control: the presets sit
/// in the middle, as WPF lays them out, not at the selector's left edge.
#[test]
fn a_full_width_preset_strip_centres_where_its_template_does() {
    let strip_of = |align: &str| {
        let presets = format!(
            r#"<ResourceDictionary {NS}>
  <Style TargetType="{{x:Type FilterPresetSelector}}">
    <Setter Property="Width" Value="1920"/>
    <Setter Property="Template"><Setter.Value>
      <ControlTemplate TargetType="{{x:Type FilterPresetSelector}}">
        <ItemsControlEx x:Name="PART_ItemsFilterPresets" {align}>
          <ItemsControl.ItemsPanel><ItemsPanelTemplate><StackPanel Orientation="Horizontal"/></ItemsPanelTemplate></ItemsControl.ItemsPanel>
        </ItemsControlEx>
      </ControlTemplate>
    </Setter.Value></Setter>
  </Style>
</ResourceDictionary>"#
        );
        let main = wrap_main(
            r#"<Grid>
  <ListBoxEx x:Name="PART_ListGameItems" Height="300" VerticalAlignment="Bottom"/>
  <FilterPresetSelector x:Name="PART_FilterPresetSelector" VerticalAlignment="Top" Height="60"/>
</Grid>"#,
        );
        let c = compiled(&[("CustomControls/FilterPresetSelector.xaml", presets), ("Views/Main.xaml", main)], &[]);
        let els = c.main.expect("main").elements;
        let strip =
            els.into_iter().find(|e| e.get("direction").and_then(T::as_str) == Some("horizontal")).expect("strip");
        let pair = |k: &str| {
            let a = strip.get(k).and_then(T::as_array).expect(k);
            a[0].as_float().expect(k) as f32
        };
        (pair("pos"), pair("origin"))
    };
    let (x, origin) = strip_of(r#"HorizontalAlignment="Center""#);
    assert!((x - 0.5).abs() < 0.01 && origin == 0.5, "{x} {origin}");
    let (x, origin) = strip_of("");
    assert!(x < 0.05 && origin == 0.0, "{x} {origin}");
}
