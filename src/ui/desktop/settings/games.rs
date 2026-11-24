//! Settings → Games: the game options every game starts from, and what the computer was found to have.

use super::*;

/// A page drawn in pass `pass` and last drawn in `last` wasn't on screen the pass before.
fn newly_shown(last: Option<u64>, pass: u64) -> bool {
    last.is_none_or(|l| pass > l.saturating_add(1))
}

/// Settings → Games: the global game options (every game follows these unless its own Launch
/// options say otherwise).
pub(super) fn page(ui: &mut Ui, st: &mut AppState, pal: &Pal) -> bool {
    heading(ui, "Game options", pal);
    hint_text(
        ui,
        "Switch things on for every game here — no launch options to learn: Lodestar adds the right \
         variables and wrappers in the right order. Each game can override any of these under \
         Launch options. Auto = Lodestar adds nothing; Off switches a feature off.",
        pal,
    );
    // Shown again (tab picked, Settings reopened): look again for tools installed meanwhile
    // (background; nothing while a probe runs).
    let seen_id = egui::Id::NULL.with("ls_go_games_seen");
    let pass = ui.ctx().cumulative_pass_nr();
    let last: Option<u64> = ui.ctx().data(|d| d.get_temp(seen_id));
    ui.ctx().data_mut(|d| d.insert_temp(seen_id, pass));
    if newly_shown(last, pass) {
        st.reprobe_game_options();
    }
    ui.horizontal(|ui| {
        let probing = st.game_opts.probing();
        if ui
            .add_enabled(!probing, egui::Button::new(if probing { "Checking…" } else { "Check again" }))
            .on_hover_text("Look again for Gamescope, MangoHud, GameMode, Vulkan layers, GPUs …")
            .clicked()
        {
            st.reprobe_game_options();
        }
        if st.game_opts.stalled() && !st.game_opts.caps.probed {
            ui.label(
                RichText::new(
                    "Detection is taking long (a slow or unreachable drive?) — nothing is greyed out meanwhile.",
                )
                .size(12.0)
                .color(pal.dim),
            );
        } else if st.game_opts.probed && st.game_opts.caps.probed {
            ui.label(RichText::new(detected_summary(&st.game_opts.caps)).size(12.0).color(pal.dim));
        }
    });
    ui.add_space(4.0);
    // Applies to: every game, or one library (over the global values, under each game's own).
    let scope_id = egui::Id::NULL.with("ls_go_scope");
    let mut scope: Option<Source> = ui.ctx().data_mut(|d| d.get_temp(scope_id)).flatten();
    let libraries: Vec<Source> = st.library.sources_present();
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label("Applies to");
        let name = |s: Option<Source>, st: &AppState| match s {
            None => "All games".to_string(),
            Some(s) => {
                let set = st.config.game_options_by_source.get(s.key()).is_some_and(|o| !o.is_default());
                format!("{} games{}", s.display(), if set { "  •" } else { "" })
            }
        };
        egui::ComboBox::from_id_salt("ls_go_scope_box").width(220.0).selected_text(name(scope, st)).show_ui(ui, |ui| {
            ui.selectable_value(&mut scope, None, name(None, st));
            for s in &libraries {
                ui.selectable_value(&mut scope, Some(*s), name(Some(*s), st));
            }
        });
        if let Some(s) = scope
            && st.config.game_options_by_source.get(s.key()).is_some_and(|o| !o.is_default())
            && ui.small_button("Reset").on_hover_text("Follow All games again").clicked()
        {
            st.config.game_options_by_source.remove(s.key());
            changed = true;
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(scope_id, scope));
    if let Some(s) = scope {
        hint_text(
            ui,
            &format!("Only {} games. Default = the All games value; a game's own options still win.", s.display()),
            pal,
        );
    }
    let caps = st.game_opts.caps.clone();
    match scope {
        None => {
            let cx = crate::ui::desktop::game_options_ui::EditorCtx {
                caps: st.game_opts.probed.then_some(&*caps),
                global: None,
                report: &[],
                salt: "global",
            };
            changed |= crate::ui::desktop::game_options_ui::options_editor(ui, &mut st.config.game_options, &cx, pal);
        }
        Some(s) => {
            let global = st.config.game_options.clone();
            let mut lib = st.config.game_options_by_source.get(s.key()).cloned().unwrap_or_default();
            let cx = crate::ui::desktop::game_options_ui::EditorCtx {
                caps: st.game_opts.probed.then_some(&*caps),
                global: Some(&global),
                report: &[],
                salt: "library",
            };
            if crate::ui::desktop::game_options_ui::options_editor(ui, &mut lib, &cx, pal) {
                if lib.is_default() {
                    st.config.game_options_by_source.remove(s.key());
                } else {
                    st.config.game_options_by_source.insert(s.key().to_string(), lib);
                }
                changed = true;
            }
        }
    }
    if changed {
        // Previews ("Will run") follow at once; the file is written when Settings closes.
        st.publish_game_option_defaults();
    }
    changed
}

/// "Found: Gamescope, MangoHud, GameMode" for the Games tab.
fn detected_summary(c: &crate::game_options::Caps) -> String {
    let b = &c.bins;
    let mut found: Vec<&str> = vec![];
    for (have, name) in [
        (b.gamescope.is_some(), "Gamescope"),
        (b.mangohud.is_some(), "MangoHud"),
        (b.gamemoderun.is_some(), "GameMode"),
        (b.game_performance.is_some() || b.powerprofilesctl.is_some(), "power profiles"),
        (b.obs_gamecapture.is_some(), "OBS capture"),
        (c.lsfg.is_some(), "lsfg-vk"),
        (c.ntsync == crate::game_options::NtsyncState::Usable, "NTSync"),
    ] {
        if have {
            found.push(name);
        }
    }
    if found.is_empty() {
        "None of the optional tools were found.".into()
    } else {
        format!("Found: {}", found.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::newly_shown;

    #[test]
    fn games_page_reprobes_only_when_shown_again() {
        assert!(newly_shown(None, 0), "first time");
        assert!(!newly_shown(Some(10), 11), "still on screen");
        assert!(!newly_shown(Some(10), 10), "two editors in one pass");
        assert!(newly_shown(Some(10), 40), "tab picked again later");
        assert!(!newly_shown(Some(u64::MAX), 0));
    }
}
