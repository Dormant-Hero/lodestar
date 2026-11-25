//! How the main screen rests: triggers on the focus, on other elements' unset properties and
//! on Playnite's own state, decided as Playnite leaves them when nothing has been pressed.

use super::*;

fn main_els(body: &str) -> Vec<Table> {
    compiled(&[("Views/Main.xaml", wrap_main(body))], &[]).main.expect("main").elements
}

/// A rect of `color` that fades to `to` while `trigger` holds.
fn veil(color: &str, trigger: &str, to: &str) -> String {
    format!(
        r##"<Border Background="{color}">
          <Border.Style><Style TargetType="Border">
            <Setter Property="Opacity" Value="0"/>
            <Style.Triggers>
              {trigger}
                <DataTrigger.EnterActions><BeginStoryboard><Storyboard>
                  <DoubleAnimation Storyboard.TargetProperty="Opacity" To="{to}" Duration="0:0:0.3"/>
                </Storyboard></BeginStoryboard></DataTrigger.EnterActions>
              </DataTrigger>
            </Style.Triggers>
          </Style></Border.Style>
        </Border>"##
    )
}

#[test]
fn a_veil_fading_in_with_the_focus_rests_where_the_focus_is_at_rest() {
    let els = main_els(&format!(
        r##"<Grid>
          <StackPanel x:Name="Menu" VerticalAlignment="Top" Height="100"><ButtonEx Content="Store"/></StackPanel>
          <Grid x:Name="Games" VerticalAlignment="Bottom" Height="500">
            <ListBoxEx x:Name="PART_ListGameItems"/>
          </Grid>
          {}
          {}
        </Grid>"##,
        veil(
            "#000001",
            r#"<DataTrigger Binding="{Binding ElementName=Menu, Path=IsKeyboardFocusWithin}" Value="True">"#,
            "0.6"
        ),
        veil(
            "#000002",
            r#"<DataTrigger Binding="{Binding ElementName=Games, Path=IsKeyboardFocusWithin}" Value="True">"#,
            "0.4"
        ),
    ));
    assert!(rects_colored(&els, "#000001").is_empty(), "the menu doesn't hold the focus: {els:#?}");
    let games = rects_colored(&els, "#000002");
    assert_eq!(games.len(), 1, "{els:#?}");
    assert!((games[0].get("opacity").and_then(T::as_float).unwrap_or(1.0) - 0.4).abs() < 1e-3);
}

#[test]
fn parts_waiting_for_a_helpers_fade_rest_where_that_fade_ends() {
    // "Id" fades itself in each time the selected game changes; the title waits for it.
    let els = main_els(
        r##"<Grid>
          <TextBlock x:Name="Id" Text="{Binding SelectedGame.Id, NotifyOnTargetUpdated=True}" Foreground="Transparent">
            <TextBlock.Triggers>
              <EventTrigger RoutedEvent="Binding.TargetUpdated"><BeginStoryboard><Storyboard>
                <DoubleAnimation Storyboard.TargetProperty="Opacity" Duration="0:0:0" To="0"/>
                <DoubleAnimation Storyboard.TargetProperty="Opacity" Duration="0:0:0.1" From="0" To="1"/>
              </Storyboard></BeginStoryboard></EventTrigger>
            </TextBlock.Triggers>
          </TextBlock>
          <TextBlock Text="Title" VerticalAlignment="Top">
            <TextBlock.Style><Style TargetType="TextBlock">
              <Setter Property="Opacity" Value="0"/>
              <Style.Triggers>
                <DataTrigger Binding="{Binding ElementName=Id, Path=Opacity}" Value="1">
                  <DataTrigger.EnterActions><BeginStoryboard><Storyboard>
                    <DoubleAnimation Storyboard.TargetProperty="Opacity" To="0.8" Duration="0:0:0.2"/>
                  </Storyboard></BeginStoryboard></DataTrigger.EnterActions>
                </DataTrigger>
              </Style.Triggers>
            </Style></TextBlock.Style>
          </TextBlock>
          <ListBoxEx x:Name="PART_ListGameItems" Height="400" VerticalAlignment="Bottom"/>
        </Grid>"##,
    );
    let title = text_of(&els, "Title").expect("title");
    assert!((title.get("opacity").and_then(T::as_float).unwrap_or(1.0) - 0.8).abs() < 1e-3, "{title:?}");
}

#[test]
fn unset_toggles_resources_and_playnites_own_state_decide_triggers() {
    let hidden_when = |binding: &str, value: &str, text: &str| {
        format!(
            r##"<TextBlock Text="{text}" VerticalAlignment="Top">
              <TextBlock.Style><Style TargetType="TextBlock"><Style.Triggers>
                <DataTrigger Binding="{binding}" Value="{value}"><Setter Property="Visibility" Value="Collapsed"/></DataTrigger>
              </Style.Triggers></Style></TextBlock.Style>
            </TextBlock>"##
        )
    };
    let els = main_els(&format!(
        r##"<Grid x:Name="PART_ViewHost">
          <Grid.Resources><sys:Boolean x:Key="Yes">True</sys:Boolean></Grid.Resources>
          <CheckBoxEx x:Name="More" Visibility="Hidden"/>
          {}
          {}
          {}
          {}
          {}
          <ButtonEx x:Name="PART_ButtonProgramUpdate" Content="Update"/>
          <ListBoxEx x:Name="PART_ListGameItems" Height="400" VerticalAlignment="Bottom"/>
        </Grid>"##,
        hidden_when("{Binding ElementName=More, Path=IsChecked}", "False", "Side panel"),
        hidden_when("{Settings Fullscreen.ShowClock}", "{StaticResource Yes}", "Clock note"),
        hidden_when("{Api Notifications.Count}", "0", "Notification dot"),
        hidden_when("{Binding ElementName=PART_ViewHost, Path=ActualWidth}", "1920", "Narrow screen note"),
        hidden_when("{Binding ElementName=PART_ButtonProgramUpdate, Path=Visibility}", "Collapsed", "Update badge"),
    ));
    for gone in ["Side panel", "Clock note", "Notification dot", "Narrow screen note", "Update badge"] {
        assert!(text_of(&els, gone).is_none(), "{gone}: {els:#?}");
    }
    assert!(text_of(&els, "Update").is_none());
}

#[test]
fn true_and_false_resources_are_the_booleans_without_the_theme_defining_them() {
    let hidden_when = |value: &str, text: &str| {
        format!(
            r##"<TextBlock Text="{text}" VerticalAlignment="Top">
              <TextBlock.Style><Style TargetType="TextBlock"><Style.Triggers>
                <DataTrigger Binding="{{Settings Fullscreen.ShowClock}}" Value="{value}">
                  <Setter Property="Visibility" Value="Collapsed"/>
                </DataTrigger>
              </Style.Triggers></Style></TextBlock.Style>
            </TextBlock>"##
        )
    };
    let els = main_els(&format!(
        r##"<Grid>{}{}<ListBoxEx x:Name="PART_ListGameItems" Height="400" VerticalAlignment="Bottom"/></Grid>"##,
        hidden_when("{StaticResource True}", "Clock note"),
        hidden_when("{StaticResource False}", "No clock note"),
    ));
    assert!(text_of(&els, "Clock note").is_none(), "{els:#?}");
    let shown = text_of(&els, "No clock note").expect("shown");
    assert!(shown.get("visible_if").is_none() && shown.get("hidden_if").is_none(), "decided: {shown:?}");
}

#[test]
fn a_wrapper_fading_in_while_it_holds_the_focus_is_judged_by_what_it_holds() {
    // Neither wrapper has a name: whether it holds the focus comes from what is inside it.
    let wrapper = |color: &str, inside: &str| {
        format!(
            r##"<Grid>
              <Grid.Style><Style TargetType="Grid">
                <Setter Property="Opacity" Value="0"/>
                <Style.Triggers>
                  <Trigger Property="IsKeyboardFocusWithin" Value="True">
                    <Trigger.EnterActions><BeginStoryboard><Storyboard>
                      <DoubleAnimation Storyboard.TargetProperty="Opacity" To="1" Duration="0:0:0.3"/>
                    </Storyboard></BeginStoryboard></Trigger.EnterActions>
                  </Trigger>
                </Style.Triggers>
              </Style></Grid.Style>
              <Border Background="{color}" Height="100" VerticalAlignment="Top"/>
              {inside}
            </Grid>"##
        )
    };
    let els = main_els(&format!(
        r##"<Grid>{}{}</Grid>"##,
        wrapper("#000003", r#"<ListBoxEx x:Name="PART_ListGameItems" Height="400" VerticalAlignment="Bottom"/>"#),
        wrapper("#000004", r#"<ButtonEx Content="Store" Height="60"/>"#),
    ));
    let holder = rects_colored(&els, "#000003");
    assert_eq!(holder.len(), 1, "the list's wrapper shows: {els:#?}");
    assert!(holder[0].get("opacity").and_then(T::as_float).unwrap_or(1.0) > 0.99, "{holder:?}");
    assert!(rects_colored(&els, "#000004").is_empty(), "the other one rests faded out: {els:#?}");
}

#[test]
fn a_text_waiting_for_another_texts_game_field_hides_without_it() {
    let els = main_els(
        r##"<Grid>
          <TextBlock x:Name="Developer" Text="{Binding SelectedGame.Developers}" Visibility="Collapsed"/>
          <TextBlock Text="Made by" VerticalAlignment="Top">
            <TextBlock.Style><Style TargetType="TextBlock"><Style.Triggers>
              <DataTrigger Binding="{Binding ElementName=Developer, Path=Text}" Value="">
                <Setter Property="Visibility" Value="Collapsed"/>
              </DataTrigger>
            </Style.Triggers></Style></TextBlock.Style>
          </TextBlock>
          <ListBoxEx x:Name="PART_ListGameItems" Height="400" VerticalAlignment="Bottom"/>
        </Grid>"##,
    );
    let label = text_of(&els, "Made by").expect("label");
    assert_eq!(label.get("visible_if").and_then(T::as_str), Some("{game.developer}"), "{label:?}");
}

#[test]
fn the_list_rests_where_its_first_game_puts_it() {
    // The list slides by the selected index; the screen opens on index 0.
    let nudge = |index: u8, to: &str| {
        format!(
            r##"<DataTrigger Binding="{{Binding ElementName=PART_ListGameItems, Path=SelectedIndex}}" Value="{index}">
              <DataTrigger.EnterActions><BeginStoryboard><Storyboard>
                <ThicknessAnimation Storyboard.TargetProperty="Margin" To="{to}" Duration="0:0:0.05"/>
              </Storyboard></BeginStoryboard></DataTrigger.EnterActions>
            </DataTrigger>"##
        )
    };
    let els = main_els(&format!(
        r##"<Grid>
          <ListBoxEx x:Name="PART_ListGameItems" Height="400" VerticalAlignment="Bottom">
            <ListBox.Style><Style TargetType="ListBoxEx"><Style.Triggers>{}{}</Style.Triggers></Style></ListBox.Style>
          </ListBoxEx>
        </Grid>"##,
        nudge(0, "96,0,-96,0"),
        nudge(1, "0,0,0,0"),
    ));
    let list = list_of(&els);
    assert!(near(pair(list, "pos")[0], 96.0 / 1920.0), "{list:?}");
}
