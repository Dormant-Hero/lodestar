//! The game page's host on the main screen and the faces of its buttons.

use super::*;

/// A main screen with a collapsed details column of its own ahead of the page's host.
fn main_with_details_column(host: &str) -> String {
    wrap_main(&format!(
        r##"<Grid>
  <GameDetails x:Name="Column" Visibility="Collapsed" Width="600" HorizontalAlignment="Right">
    <GameDetails.Style><Style TargetType="{{x:Type GameDetails}}">
      <Setter Property="Template"><Setter.Value>
        <ControlTemplate TargetType="{{x:Type GameDetails}}">
          <TextBlock Text="Column title" FontSize="30"/>
        </ControlTemplate>
      </Setter.Value></Setter>
    </Style></GameDetails.Style>
  </GameDetails>
  <ListBoxEx x:Name="PART_ListGameItems" Height="400" VerticalAlignment="Bottom"/>
  {host}
</Grid>"##
    ))
}

fn page() -> String {
    wrap_details(
        r##"<Grid Margin="300,0,0,0">
          <TextBlock Text="Page title" FontSize="50" VerticalAlignment="Top"/>
          <ButtonEx x:Name="PART_ButtonContext" Width="300" Height="60"/>
        </Grid>"##,
    )
}

#[test]
fn the_page_opens_only_in_part_elem_game_details() {
    let main = main_with_details_column(r#"<GameDetails x:Name="PART_ElemGameDetails"/>"#);
    let c = compiled(&[("Views/Main.xaml", main), ("Views/GameDetails.xaml", page())], &[]);
    let els = c.details.expect("details").elements;
    let title = text_of(&els, "Page title").expect("the page");
    // Laid out over the whole screen (the host), not in the 600 px column.
    assert!(near(pair(title, "pos")[0], 300.0 / 1920.0), "{title:?}");
    assert!(text_of(&els, "Column title").is_none(), "the column stays collapsed");
    assert!(text_of(&c.main.expect("main").elements, "Column title").is_none());
}

#[test]
fn a_main_screen_without_the_pages_host_has_its_page_compiled_alone() {
    let main = main_with_details_column("");
    let c = compiled(&[("Views/Main.xaml", main), ("Views/GameDetails.xaml", page())], &[]);
    let els = c.details.expect("details").elements;
    assert!(text_of(&els, "Page title").is_some());
    assert!(!els.iter().any(|e| e.get("name").and_then(T::as_str) == Some("pn_list")), "not over the main screen");
}

#[test]
fn a_shown_details_column_draws_its_own_template() {
    let main = main_with_details_column(r#"<GameDetails x:Name="PART_ElemGameDetails"/>"#)
        .replace(r#"Visibility="Collapsed" Width="600""#, r#"Width="600""#);
    let c = compiled(&[("Views/Main.xaml", main), ("Views/GameDetails.xaml", page())], &[]);
    assert!(text_of(&c.main.expect("main").elements, "Column title").is_some());
}

/// A page with one Play button drawn by `template` (a ControlTemplate's content and triggers).
fn page_with_button(template: &str) -> Vec<Table> {
    let details = wrap_details(&format!(
        r##"<Grid>
          <Grid.Resources>
            <Style x:Key="Play" TargetType="ButtonEx">
              <Setter Property="Background" Value="#2A2A2A"/>
              <Setter Property="Foreground" Value="#FFFFFF"/>
              <Setter Property="Template"><Setter.Value>
                <ControlTemplate TargetType="Button">{template}</ControlTemplate>
              </Setter.Value></Setter>
            </Style>
          </Grid.Resources>
          <ButtonEx x:Name="PART_ButtonContext" Style="{{StaticResource Play}}" FontSize="36"
                    Width="300" Height="80" VerticalAlignment="Bottom" Margin="100,0,0,100"/>
        </Grid>"##
    ));
    compiled(&[("Views/GameDetails.xaml", details)], &[]).details.expect("details").elements
}

#[test]
fn a_face_painted_by_the_templates_root_panel_follows_the_focus() {
    let els = page_with_button(
        r##"<Grid Background="{TemplateBinding Background}">
              <TextBlock x:Name="Label" Foreground="{TemplateBinding Foreground}"><ContentPresenter/></TextBlock>
            </Grid>
            <ControlTemplate.Triggers>
              <Trigger Property="IsFocused" Value="True">
                <Setter Property="Background" Value="#4D54A81B"/>
              </Trigger>
            </ControlTemplate.Triggers>"##,
    );
    assert_eq!(rects_colored(&els, "#54a81b4d").len(), 1, "{els:#?}");
}

#[test]
fn the_label_takes_its_colour_and_size_from_the_templates_label_part() {
    let els = page_with_button(
        r##"<Grid>
              <Border x:Name="Face" CornerRadius="40" Background="#000000" Opacity="0.7"/>
              <TextBlock x:Name="Label" Foreground="White" FontSize="28"><ContentPresenter/></TextBlock>
            </Grid>
            <ControlTemplate.Triggers>
              <Trigger Property="IsFocused" Value="True">
                <Setter Property="Background" Value="#FFFFFF" TargetName="Face"/>
                <Setter Property="Opacity" Value="1" TargetName="Face"/>
                <Setter Property="Foreground" Value="Black" TargetName="Label"/>
              </Trigger>
            </ControlTemplate.Triggers>"##,
    );
    assert_eq!(rects_colored(&els, "#ffffff").len(), 1, "{els:#?}");
    let play = text_of(&els, "Play").expect("play");
    assert_eq!(play["style"].get("color").and_then(T::as_str), Some("#000000"));
    assert!((play["style"]["font_size"].as_float().unwrap() * 1080.0 - 28.0).abs() < 0.5);
}

#[test]
fn a_template_drawing_installed_games_differently_gives_one_look_each() {
    let els = page_with_button(
        r##"<Grid>
              <Border x:Name="Face" CornerRadius="10"/>
              <TextBlock x:Name="Mark" FontFamily="Segoe MDL2 Assets"/>
              <TextBlock x:Name="Label" Foreground="White"/>
            </Grid>
            <ControlTemplate.Triggers>
              <DataTrigger Binding="{Binding IsInstalled}" Value="True">
                <Setter Property="Text" Value="&#xF5B0;" TargetName="Mark"/>
                <Setter Property="Text" Value="P L A Y" TargetName="Label"/>
              </DataTrigger>
              <DataTrigger Binding="{Binding IsInstalled}" Value="False">
                <Setter Property="Text" Value="&#xE896;" TargetName="Mark"/>
                <Setter Property="Text" Value="I N S T A L L" TargetName="Label"/>
              </DataTrigger>
              <MultiDataTrigger>
                <MultiDataTrigger.Conditions>
                  <Condition Binding="{Binding IsInstalled}" Value="True"/>
                  <Condition Binding="{Binding RelativeSource={RelativeSource Self}, Path=IsFocused}" Value="True"/>
                </MultiDataTrigger.Conditions>
                <Setter Property="Background" Value="#00AA00" TargetName="Face"/>
              </MultiDataTrigger>
              <MultiDataTrigger>
                <MultiDataTrigger.Conditions>
                  <Condition Binding="{Binding IsInstalled}" Value="False"/>
                  <Condition Binding="{Binding RelativeSource={RelativeSource Self}, Path=IsFocused}" Value="True"/>
                </MultiDataTrigger.Conditions>
                <Setter Property="Background" Value="#0000AA" TargetName="Face"/>
              </MultiDataTrigger>
            </ControlTemplate.Triggers>"##,
    );
    let installed = |e: &Table| e.get("visible_if").and_then(T::as_str) == Some("{game.installed}");
    let others = |e: &Table| e.get("hidden_if").and_then(T::as_str) == Some("{game.installed}");
    let green = rects_colored(&els, "#00aa00");
    let blue = rects_colored(&els, "#0000aa");
    assert!(green.len() == 1 && installed(green[0]), "{els:#?}");
    assert!(blue.len() == 1 && others(blue[0]), "{els:#?}");
    assert!(installed(text_of(&els, "P L A Y").expect("play")));
    assert!(others(text_of(&els, "I N S T A L L").expect("install")));
    assert!(text_of(&els, "Play").is_none() && text_of(&els, "Install").is_none(), "{els:#?}");
}

#[test]
fn a_right_aligned_row_of_input_hint_buttons_becomes_a_help_bar_growing_leftwards() {
    let details = wrap_details(
        r##"<Grid>
          <ButtonEx x:Name="PART_ButtonContext" Width="300" Height="60" VerticalAlignment="Top"/>
          <StackPanel Orientation="Horizontal" HorizontalAlignment="Right" VerticalAlignment="Bottom" Margin="0,0,200,0" Height="60">
            <ButtonEx InputHint="{DynamicResource ButtonPromptX}" Content="{Binding Unknown}"/>
            <ButtonEx InputHint="{DynamicResource ButtonPromptB}" Content="Back"/>
          </StackPanel>
        </Grid>"##,
    );
    // A main screen without a hint row of its own under the page.
    let main = main_with_details_column(r#"<GameDetails x:Name="PART_ElemGameDetails"/>"#);
    let c = compiled(&[("Views/Main.xaml", main), ("Views/GameDetails.xaml", details)], &[]);
    let els = c.details.expect("details").elements;
    let bars = by_type(&els, "helpbar");
    assert_eq!(bars.len(), 1, "the theme's own row, not Lodestar's as well: {bars:#?}");
    let bar = bars[0];
    // Anchored at the row's right edge, as wide as Lodestar's hints.
    assert!(near(pair(bar, "pos")[0], 1720.0 / 1920.0), "{bar:?}");
    assert_eq!(pair(bar, "origin")[0], 1.0, "{bar:?}");
    assert_eq!(pair(bar, "size")[0], 0.0, "{bar:?}");
}
