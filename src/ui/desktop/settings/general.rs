//! Settings → General: start-up, updates, scripts around games, what happens while playing, store clients.

use super::*;

pub(super) fn page(ui: &mut Ui, st: &mut AppState, d: &mut SettingsDlg, pal: &Pal) -> bool {
    let mut ch = false;
    let c = &mut st.config;
    heading(ui, "Startup", pal);
    ui.horizontal(|ui| {
        ui.label("Start in");
        ch |= ui.radio_value(&mut c.start_mode, StartMode::Desktop, "Desktop mode").changed();
        ch |= ui.radio_value(&mut c.start_mode, StartMode::Controller, "Controller mode").changed();
    });
    ui.add_space(8.0);
    heading(ui, "Updates", pal);
    ch |= crate::ui::desktop::update_ui::settings_section(ui, st, pal);
    ui.add_space(8.0);
    heading(ui, "While playing", pal);
    // Wayland lets a window minimise itself (KWin honours it), but there is no way for it to
    // un-minimise or raise itself afterwards (winit ignores both), so Lodestar would stay
    // minimised after the game exits. The launcher only minimises when it can restore.
    let restorable = st.window_restorable;
    let c = &mut st.config;
    let mut minimise = c.minimize_on_launch && restorable;
    let r = ui
        .add_enabled(restorable, egui::Checkbox::new(&mut minimise, "Minimise Lodestar while a game runs"))
        .on_disabled_hover_text(
            "Wayland lets Lodestar minimise itself, but not bring itself back when the game \
             exits, so it would stay minimised.",
        );
    if r.changed() && restorable {
        c.minimize_on_launch = minimise;
        ch = true;
    }
    if !restorable {
        hint_text(
            ui,
            "Not available on Wayland: Lodestar could minimise, but couldn't restore itself \
             after the game exits.",
            pal,
        );
    }
    ch |= close_while_playing(ui, st, pal);
    ch |= ui
        .checkbox(&mut st.config.auto_playing, "Set completion status to Playing when launching")
        .on_hover_text(
            "When a game's status isn't set yet, or is Not played / Plan to play. Change it any \
             time from ••• → Set completion status.",
        )
        .changed();
    ui.add_space(8.0);
    ch |= launchers(ui, st, pal);
    let c = &mut st.config;
    ui.add_space(8.0);
    heading(ui, "Interface", pal);
    ui.horizontal(|ui| {
        ui.label("UI scale");
        let r = ui.add(egui::Slider::new(&mut d.ui_scale, 0.75..=2.0).step_by(0.05).fixed_decimals(2).suffix("×"));
        if (r.drag_stopped() || (r.changed() && !r.dragged())) && (d.ui_scale - c.ui_scale).abs() > 0.001 {
            c.ui_scale = d.ui_scale;
            ui.ctx().set_zoom_factor(c.ui_scale.clamp(0.5, 3.0));
            ch = true;
        }
        if ui.small_button("Reset").clicked() {
            d.ui_scale = 1.0;
            c.ui_scale = 1.0;
            ui.ctx().set_zoom_factor(1.0);
            ch = true;
        }
    });
    ch |= ui
        .checkbox(&mut st.config.desktop.own_windows, "Open settings and editors in their own windows")
        .on_hover_text(
            "Settings, Add-ons, Edit game details, Emulated games and the artwork editor open as \
             separate windows you can move and resize (Lodestar remembers their size). Off: they \
             open inside the main window.",
        )
        .changed();
    ui.add_space(8.0);
    heading(ui, "Scripts", pal);
    hint_text(
        ui,
        "Shell commands run around every game, as you (never as root). A game can opt out in \
         Edit… → Scripts.",
        pal,
    );
    let mut test = None;
    let sc = &mut st.config.scripts;
    for (label, text) in [
        ("Before a game starts", &mut sc.before_start),
        ("After a game has started", &mut sc.after_start),
        ("After a game exits", &mut sc.after_exit),
    ] {
        let (changed, t) = crate::ui::desktop::edit::script_box(ui, label, text, pal);
        ch |= changed;
        if t {
            test = Some(text.clone());
        }
    }
    if let Some(t) = test {
        st.test_script(&t);
    }
    ch
}

/// "Show Lodestar in the system tray" and "When closing Lodestar" (tray / minimise / quit / ask).
fn close_while_playing(ui: &mut Ui, st: &mut AppState, pal: &Pal) -> bool {
    use crate::config::WhenClosing;
    let mut ch = false;
    let tray = st.tray_shown();
    let hides = st.can_hide_from_taskbar();
    ch |= ui
        .checkbox(&mut st.config.tray_icon, "Show Lodestar in the system tray")
        .on_hover_text("Its icon shows and hides the window, stops the running game and quits Lodestar.")
        .changed();
    if st.config.tray_icon && !tray {
        hint_text(ui, "No system tray found on this desktop (yet): the icon shows once there is one.", pal);
    }
    let c = &mut st.config;
    let effective = WhenClosing::effective(c.when_closing, tray);
    ui.horizontal(|ui| {
        ui.label("When closing Lodestar");
        egui::ComboBox::from_id_salt("ls_when_closing").width(240.0).selected_text(effective.label()).show_ui(
            ui,
            |ui| {
                for v in WhenClosing::ALL {
                    if ui.selectable_label(v == effective, v.label()).clicked() && c.when_closing != Some(v) {
                        c.when_closing = Some(v);
                        ch = true;
                    }
                }
            },
        );
    });
    let text = match effective {
        _ if effective.uses_tray() && !tray => {
            "There is no tray icon right now, so Lodestar minimises while a game runs instead."
        }
        WhenClosing::TrayWhilePlaying if hides => {
            "While a game runs Lodestar keeps going in the system tray, so its play time is recorded; \
             click the tray icon to bring it back. Otherwise closing quits."
        }
        WhenClosing::TrayAlways if hides => {
            "Closing the window always leaves Lodestar running in the system tray. Quit in the menu, \
             Ctrl+Q or the tray icon's Quit quits."
        }
        WhenClosing::TrayWhilePlaying | WhenClosing::TrayAlways => {
            "This desktop can't hide windows from the taskbar: Lodestar minimises instead, and the \
             tray icon still stops the game or quits."
        }
        WhenClosing::MinimiseWhilePlaying => {
            "Lodestar stays in the taskbar until the game ends, so its play time is recorded. \
             Quit in the menu always quits."
        }
        WhenClosing::Quit => "A running game keeps running; Lodestar picks it up again the next time it starts.",
        WhenClosing::Ask => "While a game runs Lodestar asks whether to keep running or quit.",
    };
    hint_text(ui, text, pal);
    ch
}

/// General → Launchers: store clients around games and one game at a time.
fn launchers(ui: &mut Ui, st: &mut AppState, pal: &Pal) -> bool {
    use crate::config::OneGameAtATime;
    let mut ch = false;
    let l = &mut st.config.launchers;
    heading(ui, "Launchers", pal);
    hint_text(
        ui,
        "Store apps such as Battle.net, the EA app, Ubisoft Connect or Heroic that a game is \
         started through.",
        pal,
    );
    ui.add_space(2.0);
    ch |= ui
        .checkbox(&mut l.hide_launchers, "Hide store launchers while playing")
        .on_hover_text("Minimised once the game is running. Sign-in, update and error screens stay in view.")
        .changed();
    ch |= ui
        .checkbox(&mut l.close_launchers_after, "Close launchers Lodestar opened when the game ends")
        .on_hover_text("A launcher you opened yourself stays open.")
        .changed();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label("Starting a game while another runs");
        egui::ComboBox::from_id_salt("ls_one_game").width(190.0).selected_text(l.one_game_at_a_time.label()).show_ui(
            ui,
            |ui| {
                for v in OneGameAtATime::ALL {
                    ch |= ui.selectable_value(&mut l.one_game_at_a_time, v, v.label()).changed();
                }
            },
        );
    });
    hint_text(
        ui,
        match l.one_game_at_a_time {
            OneGameAtATime::Ask => "Lodestar asks before closing the running game.",
            OneGameAtATime::CloseRunning => {
                "The running game is asked to quit, and closed if it doesn't, before the new one \
                 starts."
            }
            OneGameAtATime::AllowSeveral => "Games run side by side; nothing is closed.",
        },
        pal,
    );
    ui.add_space(8.0);
    ui.label(RichText::new("Steam").strong().color(pal.text));
    hint_text(ui, "When Steam isn't open yet, Lodestar starts it quietly in the background: no library window.", pal);
    ch |= ui
        .checkbox(&mut l.hide_steam, "Minimise Steam while a Steam game runs")
        .on_hover_text("Steam's library and pop-ups go out of sight once the game is running.")
        .changed();
    let mut close = l.close_steam_after;
    if ui
        .checkbox(&mut close, "Close Steam when the game ends (if Lodestar started it)")
        .on_hover_text(
            "Waits until Steam has finished saving to the cloud, then quits it normally. A Steam you opened \
             yourself, or one another game still uses, stays open.",
        )
        .changed()
    {
        l.set_close_steam(close);
        ch = true;
    }
    let mut keep = l.keep_steam_for_controller;
    if ui
        .checkbox(&mut keep, "Keep Steam running for controller support")
        .on_hover_text(
            "Steam Input controller layouts (also for non-Steam games added to Steam) only work while Steam \
             runs. Steam is then only minimised, never closed.",
        )
        .changed()
    {
        l.set_keep_steam(keep);
        ch = true;
    }
    ch
}
