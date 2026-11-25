//! The game list's place and shape on the main screen: rows the theme lays out itself.

use super::*;

/// A row bound to PART_ListGameItems' items, its items a 200 px wide cover at half size that
/// grows to full size from a shared top line when selected.
fn mirrored_row(list_opacity: &str) -> String {
    wrap_main(&format!(
        r#"<Grid>
  <Grid.Resources>
    <Style x:Key="RowItem" TargetType="ListBoxItem">
      <Setter Property="Template"><Setter.Value>
        <ControlTemplate TargetType="ListBoxItem">
          <Grid Margin="0,10,0,0" RenderTransformOrigin="0.5,0">
            <Grid.RenderTransform><ScaleTransform x:Name="Zoom" ScaleX="0.5" ScaleY="0.5"/></Grid.RenderTransform>
            <Border x:Name="Round" CornerRadius="20" Background="Black"/>
            <Grid>
              <Grid.OpacityMask><VisualBrush Visual="{{Binding ElementName=Round}}"/></Grid.OpacityMask>
              <Image Width="200" Source="{{Binding CoverImageObjectCached}}" Stretch="UniformToFill"/>
            </Grid>
          </Grid>
          <ControlTemplate.Triggers>
            <Trigger Property="IsSelected" Value="False"><Setter Property="Margin" Value="-20,0,-20,0"/></Trigger>
            <Trigger Property="IsSelected" Value="True">
              <Setter Property="Margin" Value="10,0,10,0"/>
              <Trigger.EnterActions><BeginStoryboard><Storyboard>
                <DoubleAnimation Storyboard.TargetName="Zoom" Storyboard.TargetProperty="ScaleX" To="1" Duration="0:0:0.1"/>
              </Storyboard></BeginStoryboard></Trigger.EnterActions>
            </Trigger>
          </ControlTemplate.Triggers>
        </ControlTemplate>
      </Setter.Value></Setter>
    </Style>
  </Grid.Resources>
  <ListBox x:Name="Row" VerticalAlignment="Top" Margin="40,100,0,0" Height="320"
           ItemsSource="{{Binding ElementName=PART_ListGameItems, Path=ItemsSource}}"
           ItemContainerStyle="{{StaticResource RowItem}}">
    <ListBox.ItemsPanel><ItemsPanelTemplate><StackPanel Orientation="Horizontal"/></ItemsPanelTemplate></ListBox.ItemsPanel>
  </ListBox>
  <ListBoxEx x:Name="PART_ListGameItems" Opacity="{list_opacity}" Height="400" VerticalAlignment="Bottom"/>
</Grid>"#
    ))
}

#[test]
fn a_row_mirroring_a_hidden_game_list_becomes_the_game_list() {
    let c = compiled(&[("Views/Main.xaml", mirrored_row("0"))], &[]);
    let main = c.main.expect("main");
    assert!(c.main_problem.is_none(), "{:?}", c.main_problem);
    let lists: Vec<&Table> =
        main.elements.iter().filter(|e| e.get("name").and_then(T::as_str) == Some("pn_list")).collect();
    assert_eq!(lists.len(), 1);
    let row = lists[0];
    assert_eq!(row.get("type").and_then(T::as_str), Some("carousel"));
    // Drawn at half its 200 × 300 layout size, 160 px apart (the unselected margins pull the
    // 200 px slots together), the selected one twice as big with 30 px more room each side.
    let item = pair(row, "item_size");
    assert!(near(item[0], 100.0 / 1920.0) && near(item[1], 150.0 / 1080.0), "{item:?}");
    assert_eq!(row.get("selected_scale").and_then(T::as_float), Some(2.0));
    let margins = pair(row, "selected_margins");
    assert!(near(margins[0], 30.0 / 1920.0), "{margins:?}");
    let p = pair(row, "pos");
    assert!(near(p[0], 40.0 / 1920.0) && near(p[1], 110.0 / 1080.0), "{p:?}");
    let visible = row.get("visible_items").and_then(T::as_float).unwrap_or(0.0) as f32;
    assert!((visible - 1880.0 / 160.0).abs() < 0.01, "{visible}");
    // Its first item is selected where the row starts: 10 px margin + half of 200 px.
    let at = row.get("selected_position").and_then(T::as_float).unwrap_or(0.0) as f32;
    assert!((at - 110.0 / 1880.0).abs() < 0.001, "{at}");
    // Tops stay in line; the rounded mask gives rounded covers.
    assert_eq!(row.get("item_align").and_then(T::as_float), Some(0.0));
    assert_eq!(main.list_item.and_then(|l| l.cover_radius), Some(10.0));
}

#[test]
fn a_mirror_stays_a_helper_while_the_game_list_itself_shows() {
    let c = compiled(&[("Views/Main.xaml", mirrored_row("1"))], &[]);
    let main = c.main.expect("main");
    let lists: Vec<&Table> =
        main.elements.iter().filter(|e| e.get("name").and_then(T::as_str) == Some("pn_list")).collect();
    assert_eq!(lists.len(), 1);
    assert!(pair(lists[0], "pos")[1] > 0.5, "the real list at the bottom: {:?}", lists[0]);
    assert!(main.stats.dropped.contains_key("list driven by the theme's helpers"));
}

/// The view switches its list holder into a state (`Tag`) with one of its own triggers; the
/// holder's style places itself by that state and slides in from below the screen.
#[test]
fn a_list_moved_by_a_state_the_view_switches_rests_where_that_state_puts_it() {
    let main = wrap_main(
        r#"<Grid x:Name="Host">
  <Grid.RowDefinitions><RowDefinition Height="*"/><RowDefinition Height="400"/></Grid.RowDefinitions>
  <Grid x:Name="Holder">
    <Grid.Style>
      <Style TargetType="Grid">
        <Setter Property="Margin" Value="0,2000,0,-2000"/>
        <Setter Property="Opacity" Value="0"/>
        <Style.Triggers>
          <Trigger Property="Tag" Value="Row">
            <Setter Property="Grid.Row" Value="1"/>
            <Setter Property="Height" Value="300"/>
            <Setter Property="VerticalAlignment" Value="Bottom"/>
            <Trigger.EnterActions><BeginStoryboard><Storyboard>
              <ThicknessAnimation Storyboard.TargetProperty="Margin" From="0,0,0,-400" To="0,0,0,50" Duration="0:0:1"/>
              <DoubleAnimation Storyboard.TargetProperty="Opacity" From="0" To="1" Duration="0:0:1"/>
            </Storyboard></BeginStoryboard></Trigger.EnterActions>
          </Trigger>
        </Style.Triggers>
      </Style>
    </Grid.Style>
    <ListBoxEx x:Name="PART_ListGameItems"/>
  </Grid>
</Grid>
<ControlTemplate.Triggers>
  <MultiDataTrigger>
    <MultiDataTrigger.Conditions>
      <Condition Binding="{Settings Fullscreen.HorizontalLayout}" Value="True"/>
      <Condition Binding="{Binding ElementName=PART_ElemGameDetails, Path=Visibility}" Value="Collapsed"/>
    </MultiDataTrigger.Conditions>
    <Setter Property="Tag" Value="Row" TargetName="Holder"/>
  </MultiDataTrigger>
</ControlTemplate.Triggers>"#,
    );
    let c = compiled(&[("Views/Main.xaml", main)], &[]);
    assert!(c.main_problem.is_none(), "{:?}", c.main_problem);
    let list = list_of(&c.main.expect("main").elements).clone();
    // Row 1 (the bottom 400 px), 300 px high, 50 px above that row's bottom.
    let p = pair(&list, "pos");
    let s = pair(&list, "size");
    assert!(near(p[1], (1080.0 - 50.0 - 300.0) / 1080.0) && near(s[1], 300.0 / 1080.0), "{p:?} {s:?}");
}

/// The Play button's look comes from its template: a 70 px pill inside its 100 px box, grey
/// while the focus is elsewhere (on the game list, where Playnite keeps it).
#[test]
fn a_play_button_rests_as_its_template_paints_it_unfocused() {
    let main = wrap_main(
        r##"<Grid>
  <Grid.Resources>
    <Style x:Key="Pill" TargetType="Button">
      <Setter Property="Background" Value="#202020"/>
      <Setter Property="Template"><Setter.Value>
        <ControlTemplate TargetType="Button">
          <Grid Margin="0,10,0,10" VerticalAlignment="Top">
            <Border x:Name="Plate" Background="{TemplateBinding Background}" Height="70" CornerRadius="35"/>
            <ContentPresenter/>
          </Grid>
          <ControlTemplate.Triggers>
            <Trigger Property="IsFocused" Value="False">
              <Setter Property="Background" Value="#40808080" TargetName="Plate"/>
            </Trigger>
          </ControlTemplate.Triggers>
        </ControlTemplate>
      </Setter.Value></Setter>
    </Style>
  </Grid.Resources>
  <ListBoxEx x:Name="PART_ListGameItems" Height="300" VerticalAlignment="Top"/>
  <ButtonEx x:Name="PART_ButtonPlay" Style="{StaticResource Pill}" Width="300" Height="100"
            HorizontalAlignment="Left" VerticalAlignment="Bottom" Margin="100,0,0,100" Content="Play"/>
</Grid>"##,
    );
    let els = compiled(&[("Views/Main.xaml", main)], &[]).main.expect("main").elements;
    let plate = by_type(&els, "rect")
        .into_iter()
        .find(|r| r.get("corner_radius").is_some())
        .unwrap_or_else(|| panic!("no rounded plate: {els:#?}"));
    assert!(near(pair(plate, "pos")[1], (1080.0 - 200.0 + 10.0) / 1080.0), "{plate:?}");
    assert!(near(pair(plate, "size")[1], 70.0 / 1080.0), "{plate:?}");
    assert!(near(plate.get("corner_radius").and_then(T::as_float).unwrap_or(0.0) as f32, 35.0 / 1080.0));
    let color = plate.get("color").and_then(T::as_str).and_then(Color::parse).expect("colour");
    assert_eq!(color.0[3], 0x40, "{color:?}");
}

/// A warning screen collapsed by a trigger written with a `<MultiDataTrigger.Setters>` block
/// no longer hides the list that waits for it to go away.
#[test]
fn setters_in_a_setters_block_count_for_the_parts_that_read_them() {
    let main = wrap_main(
        r#"<Grid>
  <Grid x:Name="Holder">
    <Grid.Style>
      <Style TargetType="Grid">
        <Setter Property="Visibility" Value="Visible"/>
        <Style.Triggers>
          <DataTrigger Binding="{Binding ElementName=Warning, Path=Visibility}" Value="Visible">
            <Setter Property="Visibility" Value="Collapsed"/>
          </DataTrigger>
        </Style.Triggers>
      </Style>
    </Grid.Style>
    <ListBoxEx x:Name="PART_ListGameItems" Height="300" VerticalAlignment="Bottom"/>
  </Grid>
  <Grid x:Name="Warning">
    <Grid.Style>
      <Style TargetType="Grid">
        <Setter Property="Visibility" Value="Visible"/>
        <Style.Triggers>
          <MultiDataTrigger>
            <MultiDataTrigger.Conditions>
              <Condition Binding="{PluginSettings Plugin=ThemeOptions, Path=IsInstalled}" Value="True"/>
            </MultiDataTrigger.Conditions>
            <MultiDataTrigger.Setters>
              <Setter Property="Visibility" Value="Collapsed"/>
            </MultiDataTrigger.Setters>
          </MultiDataTrigger>
        </Style.Triggers>
      </Style>
    </Grid.Style>
    <TextBlock Text="Install Theme Options" FontSize="40"/>
  </Grid>
</Grid>"#,
    );
    let c = compiled(&[("Views/Main.xaml", main)], &[]);
    assert!(c.main_problem.is_none(), "{:?}", c.main_problem);
    assert!(text_of(&c.main.expect("main").elements, "Install Theme Options").is_none());
}

/// A screen shown unless Playnite's list is set to the theme's own columns and cover aspect
/// warns about settings Lodestar doesn't have: it stays hidden, and so doesn't cover the list.
#[test]
fn a_warning_about_the_list_size_settings_stays_hidden() {
    let main = wrap_main(
        r#"<Grid>
  <ListBoxEx x:Name="PART_ListGameItems" Height="300" VerticalAlignment="Bottom"/>
  <Grid x:Name="WrongSettings" Background="Black">
    <Grid.Style>
      <Style TargetType="Grid">
        <Setter Property="Visibility" Value="Visible"/>
        <Style.Triggers>
          <MultiDataTrigger>
            <MultiDataTrigger.Conditions>
              <Condition Binding="{Settings CoverAspectRatio}" Value="1:1"/>
              <Condition Binding="{Settings Fullscreen.Columns}" Value="5"/>
            </MultiDataTrigger.Conditions>
            <Setter Property="Visibility" Value="Collapsed"/>
          </MultiDataTrigger>
        </Style.Triggers>
      </Style>
    </Grid.Style>
    <TextBlock Text="Set five columns of square covers" FontSize="40"/>
  </Grid>
</Grid>"#,
    );
    let c = compiled(&[("Views/Main.xaml", main)], &[]);
    assert!(c.main_problem.is_none(), "{:?}", c.main_problem);
    assert!(text_of(&c.main.expect("main").elements, "Set five columns of square covers").is_none());
}

/// A dimming layer faded through a mask the theme switches off (painted transparent, as a
/// trailer's fade is when there's no trailer) shows nothing, as in WPF.
#[test]
fn a_layer_seen_through_a_transparent_mask_draws_nothing() {
    let main = wrap_main(
        r##"<Grid>
  <Border x:Name="Fade" Background="Transparent"/>
  <ListBoxEx x:Name="PART_ListGameItems" Height="300" VerticalAlignment="Bottom"/>
  <Border Background="#123456" Opacity="0.5">
    <Border.OpacityMask><VisualBrush Visual="{Binding ElementName=Fade}"/></Border.OpacityMask>
  </Border>
</Grid>"##,
    );
    let els = compiled(&[("Views/Main.xaml", main)], &[]).main.expect("main").elements;
    assert!(
        !els.iter().any(|e| e.get("color").and_then(T::as_str).is_some_and(|c| c.starts_with("#123456"))),
        "{els:#?}"
    );
}
