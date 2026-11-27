//! Settings → Appearance: the desktop theme, UI scale and how the library looks.

use super::*;

pub(super) fn page(ui: &mut Ui, st: &mut AppState, d: &mut SettingsDlg, pal: &Pal, desk: &mut DesktopUi) -> bool {
    let mut ch = false;
    heading(ui, "Themes", pal);
    let infos: Vec<(String, String, bool, String)> = st
        .themes
        .list()
        .iter()
        .map(|i| (i.key.clone(), i.meta.name.clone(), i.meta.controller, i.meta.origin.clone()))
        .collect();
    let name_of = |key: &str| infos.iter().find(|i| i.0 == key).map(|i| i.1.clone()).unwrap_or_else(|| key.to_string());
    let mut set_desktop = None;
    let mut set_controller = None;
    let mut set_scheme: Option<(bool, Option<String>)> = None;
    egui::Grid::new("ls_theme_grid").num_columns(2).spacing([14.0, 8.0]).show(ui, |ui| {
        ui.label("Desktop theme");
        egui::ComboBox::from_id_salt("ls_desk_theme")
            .width(260.0)
            .selected_text(name_of(&st.config.desktop_theme))
            .show_ui(ui, |ui| {
                for (key, name, _, origin) in &infos {
                    let label = theme_choice_label(name, origin);
                    if ui.selectable_label(*key == st.config.desktop_theme, label).clicked() {
                        set_desktop = Some(key.clone());
                    }
                }
            });
        ui.end_row();
        ui.label("Controller theme");
        egui::ComboBox::from_id_salt("ls_ctl_theme")
            .width(260.0)
            .selected_text(name_of(&st.config.controller.theme))
            .show_ui(ui, |ui| {
                for (key, name, controller, origin) in &infos {
                    if !controller {
                        continue;
                    }
                    let label = theme_choice_label(name, origin);
                    if ui.selectable_label(*key == st.config.controller.theme, label).clicked() {
                        set_controller = Some(key.clone());
                    }
                }
            });
        ui.end_row();
        for (controller, theme) in [(false, st.desktop_theme.clone()), (true, st.controller_theme.clone())] {
            if theme.file.color_schemes.is_empty() {
                continue;
            }
            ui.label(if controller { "Controller colours" } else { "Desktop colours" });
            let cur = if controller {
                st.config.controller.color_scheme.clone()
            } else {
                st.config.desktop_color_scheme.clone()
            };
            let label = |k: &Option<String>| {
                k.as_ref()
                    .and_then(|k| theme.file.color_schemes.get(k).map(|s| s.label_or(k).to_string()))
                    .unwrap_or_else(|| "Theme's own".into())
            };
            egui::ComboBox::from_id_salt(("ls_theme_scheme", controller))
                .width(260.0)
                .selected_text(label(&cur))
                .show_ui(ui, |ui| {
                    let mut opts: Vec<Option<String>> = vec![None];
                    opts.extend(theme.file.color_schemes.keys().cloned().map(Some));
                    for o in opts {
                        if ui.selectable_label(o == cur, label(&o)).clicked() {
                            set_scheme = Some((controller, o));
                        }
                    }
                });
            ui.end_row();
        }
    });
    if let Some((controller, s)) = set_scheme {
        if controller {
            st.config.controller.color_scheme = s;
            let k = st.config.controller.theme.clone();
            st.set_controller_theme(&k);
        } else {
            st.config.desktop_color_scheme = s;
            let k = st.config.desktop_theme.clone();
            st.set_desktop_theme(&k);
        }
    }
    if let Some(k) = set_desktop {
        st.set_desktop_theme(&k);
    }
    if let Some(k) = set_controller {
        st.set_controller_theme(&k);
    }
    hint_text(ui, "The desktop UI uses a theme's colours and fonts; controller mode uses its full layout.", pal);
    theme_options::show(ui, st, pal);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Game background behind the window");
        let cur = st.config.desktop.background_image;
        let d = &st.desktop_theme.file.desktop;
        let theme_says = match (d.background_image, d.background_image_details_only) {
            (false, _) => "off",
            (true, false) => "on",
            (true, true) => "in the details view",
        };
        let label = |v: Option<bool>| match v {
            None => format!("Theme default ({theme_says})"),
            Some(true) => "On".to_string(),
            Some(false) => "Off".to_string(),
        };
        egui::ComboBox::from_id_salt("ls_ambient").selected_text(label(cur)).show_ui(ui, |ui| {
            for v in [None, Some(true), Some(false)] {
                if ui.selectable_label(cur == v, label(v)).clicked() {
                    st.config.desktop.background_image = v;
                    ch = true;
                }
            }
        });
    });
    hint_text(
        ui,
        "Blends the selected game's artwork, blurred, behind the whole desktop window. \
         The \"Lodestar Ambient\" theme turns it on by default.",
        pal,
    );

    ui.add_space(8.0);
    heading(ui, "Game page", pal);
    ch |= ui.checkbox(&mut st.config.desktop.show_page_logo, "Show the logo or name over the background").changed();
    hint_text(ui, "To hide it for one game only, use Hide logo in Arrange mode on that game's page.", pal);

    ui.add_space(8.0);
    heading(ui, "Grid", pal);
    let mut names = desk.show_names;
    if ui.checkbox(&mut names, "Show game names under covers").changed() {
        desk.show_names = names;
        if let Err(e) = st.db.kv_set(crate::ui::desktop::KV_SHOW_NAMES, if names { "1" } else { "0" }) {
            log::warn!("saving \"show names\": {e:#}");
        }
    }
    ch |= ui
        .checkbox(&mut st.config.desktop.controller_badge, "Controller badge on covers")
        .on_hover_text("A small gamepad on the covers of games with full controller support")
        .changed();
    ui.horizontal(|ui| {
        ui.label("Cover size");
        ch |= ui
            .add(egui::Slider::new(&mut st.config.desktop.cover_width, 100.0..=320.0).suffix(" px").fixed_decimals(0))
            .changed();
    });
    hint_text(ui, "Ctrl + mouse wheel over the grid changes it too.", pal);

    ui.add_space(10.0);
    heading(ui, "More themes", pal);
    hint_text(ui, "Browse themes made by the community and install one in a click.", pal);
    ui.horizontal(|ui| {
        if ui.button("Get more themes").clicked() {
            d.open_addons = Some(crate::ui::desktop::addons::Section::Browse);
        }
    });
    ch
}

/// A theme in the theme pickers: "Classic" for Lodestar's own, "Classic · Playnite" for
/// imported ones, so an imported "Classic" isn't a second identical
/// entry next to the built-in one.
pub(crate) fn theme_choice_label(name: &str, origin: &str) -> String {
    let from = match origin.trim() {
        "" | "lodestar" => return name.to_string(),
        "playnite" => "Playnite",
        other => other,
    };
    format!("{name} · {from}")
}

#[cfg(test)]
mod tests {
    use super::theme_choice_label;

    /// Imported themes say where they come from, in words: an imported "Classic" is not a
    /// second identical entry next to the built-in one.
    #[test]
    fn theme_pickers_name_where_a_theme_comes_from() {
        assert_eq!(theme_choice_label("Classic", "lodestar"), "Classic");
        assert_eq!(theme_choice_label("Classic", "playnite"), "Classic · Playnite");
    }
}
