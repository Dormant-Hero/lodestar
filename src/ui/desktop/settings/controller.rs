//! Settings → Controller: controller mode's theme, the menu button, videos and sounds.

use super::*;
use crate::ui::desktop::dialogs::button_row;

/// Settings → Controller → Menu button: "Lodestar's menu button", the combination (a preset,
/// the one in use, or "Record…" on the pad) and what the choice means; the names are the
/// controller Settings → Input ones. Returns "changed".
fn menu_button_choice(ui: &mut Ui, st: &mut AppState, pal: &Pal) -> bool {
    use crate::config::{MenuButton, MenuCombo};
    use crate::ui::controller::settings::{
        MENU_BUTTON_LABEL, MENU_COMBO_LABEL, combo_note, home_button_note, home_note,
    };
    let mut ch = false;
    let c = &mut st.config.controller;
    ui.horizontal(|ui| {
        ui.label(MENU_BUTTON_LABEL);
        egui::ComboBox::from_id_salt("ls_menu_button").width(170.0).selected_text(c.menu_button.label()).show_ui(
            ui,
            |ui| {
                for b in MenuButton::ALL {
                    // Picking the one already shown counts too while the question is unanswered.
                    if ui.selectable_label(c.menu_button == b, b.label()).clicked()
                        && (c.menu_button != b || !c.menu_button_asked)
                    {
                        c.menu_button = b;
                        // Chosen here: "Which button opens Lodestar's menu?" has nothing left to ask.
                        c.menu_button_asked = true;
                        ch = true;
                    }
                }
            },
        );
    });
    let Some(combo) = c.active_menu_combo() else {
        hint_text(ui, &home_button_note(), pal);
        return ch;
    };
    let mut record = false;
    ui.horizontal(|ui| {
        ui.label(MENU_COMBO_LABEL);
        let mut choices: Vec<MenuCombo> = MenuCombo::PRESETS.to_vec();
        if !choices.contains(&combo) {
            choices.insert(0, combo);
        }
        egui::ComboBox::from_id_salt("ls_menu_combo").width(170.0).selected_text(combo.label()).show_ui(ui, |ui| {
            for m in choices {
                if ui.selectable_label(combo == m, m.label()).clicked() && combo != m {
                    c.menu_combo = m;
                    ch = true;
                }
            }
        });
        record =
            ui.button("Record…").on_hover_text("Hold the buttons you want on the controller, then let go").clicked();
    });
    ch |= combo_recorder(ui, st, record, pal);
    let c = &st.config.controller;
    hint_text(ui, &format!("{} {}", combo_note(c.menu_combo), home_note(c)), pal);
    ch
}

/// Desktop "Record…": the pad records the buttons held together (input.rs `record_combo`, renewed
/// every frame while this shows; it lapses on its own if the page closes), then Use / Try again /
/// Cancel. Returns "changed".
fn combo_recorder(ui: &mut Ui, st: &mut AppState, start: bool, pal: &Pal) -> bool {
    use crate::input::{RecordPoll, Recorded};
    #[derive(Clone, Copy)]
    enum Step {
        Waiting,
        Took(Recorded),
    }
    let id = egui::Id::unique("ls_menu_combo_recorder");
    let mut step = ui.data(|d| d.get_temp::<Step>(id));
    if start {
        step = Some(Step::Waiting);
    }
    let mut ch = false;
    let mut next = step;
    match step {
        None => {}
        Some(Step::Waiting) => match st.input.record_combo() {
            RecordPoll::Waiting(live) => {
                let held = if live.is_empty() { "nothing yet".to_string() } else { live.label() };
                ui.horizontal(|ui| {
                    ui.label(format!("Hold the buttons you want on the controller, then let go. Held: {held}"));
                    if ui.button("Cancel").clicked() {
                        st.input.stop_recording();
                        next = None;
                    }
                });
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
            }
            RecordPoll::Done(take) => {
                next = Some(Step::Took(take));
                ui.ctx().request_repaint();
            }
        },
        Some(Step::Took(take)) => {
            let v = crate::ui::controller::combo::take_verdict(take);
            hint_text(ui, &format!("{}: {}", v.heading, v.text), pal);
            let buttons = if v.usable { Buttons::primary("Use this combination", true) } else { Buttons::default() };
            let f = button_row(ui, pal, buttons.cancel("Cancel"), |ui| {
                if ui.button("Try again").clicked() {
                    next = Some(Step::Waiting);
                }
            });
            if f.primary {
                st.config.controller.menu_combo = take.combo;
                ch = true;
                next = None;
            }
            if f.cancel {
                next = None;
            }
        }
    }
    ui.data_mut(|d| match next {
        Some(s) => {
            d.insert_temp(id, s);
        }
        None => d.remove::<Step>(id),
    });
    ch
}

pub(super) fn page(ui: &mut Ui, st: &mut AppState, d: &mut SettingsDlg, pal: &Pal) -> bool {
    let mut ch = false;
    heading(ui, "Desktop mode", pal);
    let c = &mut st.config.controller;
    ch |= ui.checkbox(&mut c.desktop_navigation, "Use the controller in desktop mode").changed();
    let name = c.menu_button_name();
    hint_text(
        ui,
        &format!(
            "Move around the library, its menus and windows while Lodestar is the active window and no \
             game is running. While a game runs only the menu button ({name}) reaches Lodestar."
        ),
        pal,
    );
    ui.add_space(6.0);
    heading(ui, "Controller mode", pal);
    controller_look(ui, st, d);
    let c = &mut st.config.controller;
    ch |= ui.checkbox(&mut c.fullscreen, "Fullscreen").changed();
    ch |= ui.checkbox(&mut c.allow_quit, "Allow quitting from the controller menu").changed();
    ch |= ui
        .checkbox(&mut c.swap_ab, "Swap confirm / back (confirm on the right button, as in Nintendo games)")
        .changed();
    ch |= ui
        .checkbox(&mut c.hide_cursor, "Hide the mouse pointer in controller mode (it comes back when the mouse moves)")
        .changed();
    let mut fg_only = !c.background_input;
    if ui.checkbox(&mut fg_only, "Only use the controller when Lodestar is the active window").changed() {
        c.background_input = !fg_only;
        ch = true;
    }
    let name = c.menu_button_name();
    hint_text(
        ui,
        &format!(
            "While a game runs, Lodestar always ignores the controller (your presses only reach the game), \
             except the menu button ({name})."
        ),
        pal,
    );
    ui.add_space(6.0);
    heading(ui, "Menu button", pal);
    ch |= menu_button_choice(ui, st, pal);
    {
        use crate::ui::controller::settings::{MENU_BUTTON_DESKTOP, MENU_BUTTON_IN_GAME};
        let c = &mut st.config.controller;
        ch |= ui.checkbox(&mut c.guide_opens_lodestar, MENU_BUTTON_IN_GAME).changed();
        ch |= ui.checkbox(&mut c.guide_opens_controller_mode, MENU_BUTTON_DESKTOP).changed();
    }
    ui.add_space(6.0);
    heading(ui, "Button prompts", pal);
    {
        use crate::config::ButtonPrompts;
        use crate::theme::render::draw::{Button, Hint, Prompts, hints_width, paint_hints};
        let detected = st.detected_button_style().unwrap_or_default();
        let c = &mut st.config.controller;
        let name = |p: ButtonPrompts| match p {
            ButtonPrompts::Auto => format!("Automatic ({})", detected.label()),
            p => p.label().to_string(),
        };
        ui.horizontal(|ui| {
            ui.label("Show buttons as");
            egui::ComboBox::from_id_salt("ls_button_prompts")
                .width(220.0)
                .selected_text(name(c.button_prompts))
                .show_ui(ui, |ui| {
                    for p in ButtonPrompts::ALL {
                        if ui.selectable_label(c.button_prompts == p, name(p)).clicked() && c.button_prompts != p {
                            c.button_prompts = p;
                            ch = true;
                        }
                    }
                });
        });
        hint_text(
            ui,
            "Automatic shows the buttons of the controller you used last (Xbox without one). \
             Only the labels change: the button in the same place always does the same thing.",
            pal,
        );
        let prompts = Prompts::new(c.button_prompts.resolve(|| Some(detected)), c.swap_ab);
        let hints = [
            Hint::new(Button::A, "Select"),
            Hint::new(Button::B, "Back"),
            Hint::new(Button::X, "Favourite"),
            Hint::new(Button::Y, "Search"),
            Hint::new(Button::LB, "Page"),
            Hint::new(Button::Start, "Menu"),
            Hint::new(Button::Select, "Options"),
        ];
        let (px, fam) = (13.0, egui::FontFamily::Proportional);
        // A narrow window: two rows instead of running off the right edge.
        let all = hints_width(ui.painter(), &hints, prompts, fam.clone(), px);
        let rows: Vec<&[Hint]> =
            if all <= ui.available_width() { vec![&hints[..]] } else { vec![&hints[..4], &hints[4..]] };
        for row in rows {
            let w = hints_width(ui.painter(), row, prompts, fam.clone(), px);
            let (rect, _) = ui.allocate_exact_size(vec2(w, px * 2.0), egui::Sense::hover());
            paint_hints(
                ui.painter(),
                rect,
                row,
                prompts,
                fam.clone(),
                px,
                pal.text,
                pal.text,
                pal.bg,
                crate::theme::Align::Left,
            );
        }
    }
    ui.add_space(6.0);
    ch |= categories(ui, st, pal);
    ui.add_space(6.0);
    egui::CollapsingHeader::new(RichText::new("More options").strong().color(pal.text))
        .id_salt("ls_ctl_more")
        .default_open(false)
        .show(ui, |ui| {
            let c = &mut st.config.controller;
            heading(ui, "Screen", pal);
            ch |= ui.checkbox(&mut c.show_clock, "Show the clock").changed();
            ch |= ui.checkbox(&mut c.show_battery, "Show the battery level (laptops, handhelds)").changed();
            ch |= ui.checkbox(&mut c.blur_background, "Blur the background").changed();
            ch |= ui.checkbox(&mut c.darken_background, "Darken the background").changed();
            ch |= ui.checkbox(&mut c.show_titles, "Show game titles").changed();
            ch |= ui.checkbox(&mut c.darken_not_installed, "Dim games that aren't installed").changed();
            ui.horizontal(|ui| {
                ui.label("Text size");
                egui::ComboBox::from_id_salt("ls_ctl_text_size").selected_text(c.text_size.label()).show_ui(ui, |ui| {
                    for t in crate::config::TextSize::ALL {
                        if ui.selectable_label(c.text_size == t, t.label()).clicked() && c.text_size != t {
                            c.text_size = t;
                            ch = true;
                        }
                    }
                });
            });
            ui.add_space(6.0);
            ch |= videos_and_sounds(ui, st, pal);
            let c = &mut st.config.controller;
            ui.add_space(6.0);
            heading(ui, "Power options in the controller menu", pal);
            hint_text(ui, "Each one asks before it does anything.", pal);
            let p = &mut c.power;
            ui.horizontal_wrapped(|ui| {
                ch |= ui.checkbox(&mut p.shutdown, "Shut down").changed();
                ch |= ui.checkbox(&mut p.restart, "Restart").changed();
                ch |= ui.checkbox(&mut p.suspend, "Sleep").changed();
                ch |= ui.checkbox(&mut p.lock, "Lock").changed();
                ch |= ui.checkbox(&mut p.logout, "Log out").changed();
            });
            ui.add_space(6.0);
            heading(ui, "Held direction repeat", pal);
            egui::Grid::new("ls_ctl_grid").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
                ui.label("Initial delay");
                ch |= ui.add(egui::Slider::new(&mut c.repeat_delay_ms, 150..=800).suffix(" ms")).changed();
                ui.end_row();
                ui.label("Repeat interval");
                ch |= ui.add(egui::Slider::new(&mut c.repeat_interval_ms, 30..=250).suffix(" ms")).changed();
                ui.end_row();
            });
        });
    ui.add_space(8.0);
    heading(ui, "Connected controllers", pal);
    let pads = st.input.gamepads();
    if pads.is_empty() {
        ui.label(RichText::new("None detected").color(pal.dim));
    }
    for p in pads {
        ui.label(RichText::new(p).color(pal.text));
    }
    ui.add_space(8.0);
    if ui.button("Switch to controller mode now").clicked() {
        st.switch_mode(crate::app::Mode::Controller);
    }
    ch
}

/// Settings → Controller → Categories: the checklist of controller Settings → General →
/// Categories. Returns "changed".
fn categories(ui: &mut Ui, st: &mut AppState, pal: &Pal) -> bool {
    use crate::library::Category;
    use std::sync::Arc;
    type Rows = Arc<Vec<(&'static str, Vec<(Category, String)>)>>;
    heading(ui, "Categories in controller mode", pal);
    hint_text(ui, "Hiding a category never hides its games: they stay in All Games.", pal);
    // Counting every category's games is a pass over the library each: once per change.
    let id = egui::Id::unique("ls_ctl_categories");
    let generation = st.library.generation;
    let rows: Rows = match ui.data(|d| d.get_temp::<(u64, Rows)>(id)) {
        Some((g, rows)) if g == generation => rows,
        _ => {
            let rows: Rows = Arc::new(crate::ui::controller::categories::checklist(&st.library));
            ui.data_mut(|d| d.insert_temp(id, (generation, rows.clone())));
            rows
        }
    };
    let mut ch = false;
    let choice = &mut st.config.controller.categories;
    for (group, cats) in rows.iter() {
        ui.label(RichText::new(*group).color(pal.dim));
        ui.horizontal_wrapped(|ui| {
            for (c, label) in cats {
                let key = c.collection.key();
                if !c.hideable() {
                    ui.add_enabled(false, egui::Checkbox::new(&mut true, label.as_str()))
                        .on_disabled_hover_text("Always shown: every game stays one press away here");
                    continue;
                }
                let mut shown = choice.shows(&key, c.default);
                let mut checkbox = ui.checkbox(&mut shown, label.as_str());
                if c.collection.kind == crate::library::CollectionKind::Pc {
                    checkbox = checkbox.on_hover_text(crate::ui::controller::categories::PC_HELP);
                }
                if checkbox.changed() {
                    choice.set(&key, c.default, shown);
                    ch = true;
                }
            }
        });
    }
    ch
}

/// Settings → Controller → More options → Videos and sounds: the same settings, with the same
/// choices, as controller mode's Settings → Visuals (Theme videos) and Settings → Audio, so a
/// theme import report read on the desktop doesn't point at a switch only controller mode has.
fn videos_and_sounds(ui: &mut Ui, st: &mut AppState, pal: &Pal) -> bool {
    use crate::config::AudioMode;
    let mut ch = false;
    heading(ui, "Videos and sounds", pal);
    let theme = &st.controller_theme.file;
    let (sounds_theme, music_theme) = (!theme.sounds.is_empty(), !theme.music.is_empty());
    let c = &mut st.config.controller;
    ch |= ui
        .checkbox(&mut c.theme_videos, "Theme videos")
        .on_hover_text("Videos a controller theme plays behind the games (they rest while a game runs)")
        .changed();
    if c.theme_videos && !crate::media::videos_available() {
        hint_text(ui, &crate::media::deps::message(crate::media::deps::Need::Video), pal);
    }
    let mut audio = |ui: &mut Ui, id: &str, label: &str, mode: &mut AudioMode, theme_has: bool| {
        ui.horizontal(|ui| {
            ui.label(label);
            egui::ComboBox::from_id_salt(id).width(150.0).selected_text(mode.label(theme_has)).show_ui(ui, |ui| {
                for m in [AudioMode::Auto, AudioMode::On, AudioMode::Off] {
                    if ui.selectable_label(*mode == m, m.label(theme_has)).clicked() && *mode != m {
                        *mode = m;
                        ch = true;
                    }
                }
            });
        });
    };
    audio(ui, "ls_ctl_nav_sounds", "Navigation sounds", &mut c.nav_sounds, sounds_theme);
    audio(ui, "ls_ctl_music", "Background music", &mut c.music, music_theme);
    hint_text(
        ui,
        "Automatic plays them when the controller theme brings its own. \
         They play only in controller mode, never while a game runs. Volume and the music folder are \
         in controller mode: Settings → Audio.",
        pal,
    );
    ch
}

/// Settings → Controller: which theme controller mode uses (the same choice, same name and
/// same entries as Appearance → Controller theme), with each theme's description on hover,
/// and a way to that theme's options (they live under Appearance with the desktop theme's).
fn controller_look(ui: &mut Ui, st: &mut AppState, d: &mut SettingsDlg) {
    let mut set = None;
    ui.horizontal(|ui| {
        ui.label("Controller theme");
        let cur = st
            .themes
            .list()
            .iter()
            .find(|t| t.key == st.config.controller.theme)
            .map(|t| theme_choice_label(&t.meta.name, &t.meta.origin))
            .unwrap_or_else(|| st.controller_theme.file.theme.name.clone());
        egui::ComboBox::from_id_salt("ls_ctl_look").width(260.0).selected_text(cur).show_ui(ui, |ui| {
            for t in st.themes.list().iter().filter(|t| t.meta.controller) {
                let label = theme_choice_label(&t.meta.name, &t.meta.origin);
                let r = ui.selectable_label(t.key == st.config.controller.theme, label);
                let r = if t.meta.description.is_empty() { r } else { r.on_hover_text(&t.meta.description) };
                if r.clicked() {
                    set = Some(t.key.clone());
                }
            }
        });
    });
    if let Some(k) = set {
        st.set_controller_theme(&k);
    }
    let key = st.config.controller.theme.clone();
    if crate::ui::theme_options::spec_for(st, &key).is_some()
        && ui
            .add(egui::Button::new(RichText::new("Theme options…").size(12.5)).frame(false))
            .on_hover_text("Choices this theme's author offers (Settings → Appearance)")
            .clicked()
    {
        d.tab = Tab::Appearance;
    }
}
