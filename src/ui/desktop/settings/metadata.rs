//! Settings → Metadata: artwork and metadata sources and their keys.

use super::*;

pub(super) fn page(ui: &mut Ui, st: &mut AppState, d: &mut SettingsDlg, pal: &Pal) -> bool {
    let mut ch = false;
    heading(ui, "Online metadata", pal);
    ch |= ui
        .checkbox(
            &mut st.config.metadata.steam_store,
            "Fetch missing covers and descriptions from the Steam store (Steam games, no account needed)",
        )
        .changed();
    ch |= ui
        .checkbox(&mut st.config.art.auto_fill, "Find artwork for new games without any after a scan")
        .on_hover_text(
            "Each game is looked up once per source, by its name (or its Steam / GOG id), in the \
             order below. Only exact matches are used, and art a game already has is never replaced.",
        )
        .changed();
    let igdb_ready = st.config.metadata.igdb_credentials().is_some();
    ch |= ui
        .checkbox(
            &mut st.config.metadata.igdb_metadata,
            "Fill in missing descriptions, genres, release dates, scores and series from IGDB",
        )
        .on_hover_text(if igdb_ready {
            "Only exact matches; what a game already has (and your own edits) is kept."
        } else {
            "Needs IGDB set up below (a free Twitch application)."
        })
        .changed();
    ch |= ui
        .checkbox(&mut st.config.metadata.wikipedia, "Fill in missing descriptions from Wikipedia (no account needed)")
        .on_hover_text(
            "Asked after the Steam store and IGDB. Only the game's own English article is used \
             (not a film or book of the same name); the game page links to it.",
        )
        .changed();
    if st.config.metadata.wikipedia {
        ui.horizontal(|ui| {
            ui.add_space(26.0);
            ch |= ui
                .checkbox(&mut st.config.metadata.prefer_wikipedia, "Prefer Wikipedia's description")
                .on_hover_text(
                    "Use the Wikipedia article's text even when the Steam store or IGDB has a \
                     description. Descriptions you wrote yourself are always kept.",
                )
                .changed();
        });
    }
    ui.add_space(8.0);
    heading(ui, "Artwork sources", pal);
    hint_text(
        ui,
        "Searched in this order. Downloads take each image from the first source that knows the \
         game for sure. Sources that need a key are skipped until it is set below.",
        pal,
    );
    ui.add_space(4.0);
    ch |= crate::ui::desktop::sources_ui::source_list(ui, st, pal);
    ui.add_space(10.0);
    // "Needs an API key — set it up below" in the list: show this part and its key field.
    let sgdb_wanted = crate::ui::desktop::sources_ui::take_scroll(ui, crate::art::SourceKind::SteamGridDb);
    if sgdb_wanted {
        ui.scroll_to_cursor(Some(egui::Align::TOP));
    }
    heading(ui, "SteamGridDB", pal);
    hint_text(
        ui,
        "With an API key, art for non-Steam games is looked up on SteamGridDB (steamgriddb.com → Preferences → API). \
         Copy the key there and paste it here with Ctrl+V.",
        pal,
    );
    if crate::ui::desktop::dialogs::link_text(
        ui,
        "Open steamgriddb.com → API",
        "https://www.steamgriddb.com/profile/preferences/api",
        pal,
    ) {
        st.open_url("https://www.steamgriddb.com/profile/preferences/api");
    }
    let key = d.key_edit.get_or_insert_with(|| st.config.metadata.steamgriddb_key.clone().unwrap_or_default());
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(key)
                .password(!d.show_key)
                .hint_text("API key")
                .desired_width(340.0)
                .font(egui::TextStyle::Monospace),
        );
        if sgdb_wanted {
            r.request_focus();
        }
        if ui.small_button(if d.show_key { "Hide" } else { "Show" }).clicked() {
            d.show_key = !d.show_key;
        }
        if !d.check_key && !key.trim().is_empty() && ui.small_button("Check").clicked() {
            d.check_key = true;
        }
        if r.changed() {
            let k = key.trim().to_string();
            st.config.metadata.steamgriddb_key = if k.is_empty() { None } else { Some(k) };
            d.check_key = true;
            ch = true;
        }
    });
    if d.check_key
        && let Some(c) = st.sgdb_key_check(key)
    {
        ui.horizontal(|ui| crate::ui::desktop::welcome_ui::key_status(ui, &c, pal));
    }
    ui.add_space(10.0);
    heading(ui, "More sources with your own key", pal);
    ch |= crate::ui::desktop::sources_ui::keyed_sources(ui, st, &mut d.show_key, pal);
    ui.add_space(8.0);
    let pending = st.metadata.pending();
    if pending > 0 {
        ui.horizontal(|ui| {
            {
                let c = ui.visuals().strong_text_color();
                crate::ui::widgets::lazy_spinner(ui, 16.0, c);
            }
            ui.label(RichText::new(format!("{pending} lookups queued")).color(pal.dim));
        });
    }
    ch
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(st: &mut AppState, d: &mut SettingsDlg) {
        let ctx = st.ctx().clone();
        let pal = Pal::default();
        let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
            page(ui, st, d, &pal);
        });
    }

    /// Opening the page must not send the saved key to SteamGridDB: only an edit or Check does.
    #[test]
    fn the_saved_key_is_checked_only_when_asked() {
        let tmp = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(tmp.path());
        st.config.metadata.steamgriddb_key = Some("0123456789abcdef".into());
        let mut d = SettingsDlg::new(Tab::Metadata);
        for _ in 0..3 {
            draw(&mut st, &mut d);
        }
        assert!(st.art_state.key_check.untouched(), "the page asked about the key on its own");
        // Check (or an edit) asks once the field stays the same a moment.
        d.check_key = true;
        draw(&mut st, &mut d);
        assert!(!st.art_state.key_check.untouched());
    }
}
