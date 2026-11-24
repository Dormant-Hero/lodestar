//! Settings → Achievements: the switches, the optional Steam Web API key, the RetroAchievements
//! account (username + Web API key, checked with one call) and whether each emulator is signed
//! in to RetroAchievements (read from their own settings; Lodestar never signs them in).

use super::*;
use crate::achievements::RaAccount;
use crate::achievements::ra::emulators::{Signin, SigninStatus};
use crate::ui::desktop::emulation_ui::{card, empty_row, row, row_separator, tone};
use crossbeam_channel::Receiver;

const RA_SETTINGS_URL: &str = "https://retroachievements.org/settings";
const STEAM_KEY_URL: &str = "https://steamcommunity.com/dev/apikey";

/// The page's own state while Settings is open.
#[derive(Default)]
pub(crate) struct AchPage {
    /// The emulators' sign-ins (read on a helper thread when the page opens / on Refresh).
    signins: Option<Vec<Signin>>,
    reading: Option<Receiver<Vec<Signin>>>,
    /// "Check" was pressed for what's in the fields now (edits hide the old answer).
    checked: bool,
    /// The successful answer was stored in the config already.
    stored: bool,
    /// Testing hook: a made-up check answer (None inside = still checking).
    debug_check: Option<Option<Result<RaAccount, String>>>,
}

impl AchPage {
    /// Testing hook (`settings-achievements[-checking|-signedin|-error]`).
    pub(crate) fn debug(what: &str) -> AchPage {
        let answer = match what {
            "checking" => Some(None),
            "signedin" => {
                Some(Some(Ok(RaAccount { display_name: "Player1".into(), ulid: "00003EMFWR7XB8SDPEHB3K56ZQ".into() })))
            }
            "error" => Some(Some(Err("RetroAchievements doesn't know this username and key".into()))),
            _ => None,
        };
        // A made-up success is stored like a real one (screenshot runs use a copy of a home).
        AchPage { checked: answer.is_some(), debug_check: answer, ..Default::default() }
    }

    fn read_signins(&mut self, ctx: &egui::Context) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = ctx.clone();
        let fake = crate::ui::achievements::fake_kind().is_some();
        let spawned = std::thread::Builder::new().name("ra-signins".into()).spawn(move || {
            let list = if fake {
                crate::ui::achievements::fake_signins()
            } else {
                crate::achievements::ra::emulators::signins()
            };
            let _ = tx.send(list);
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.reading = Some(rx);
        }
    }

    fn poll(&mut self) {
        if let Some(list) = self.reading.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.signins = Some(list);
            self.reading = None;
        }
    }
}

pub(super) fn page(ui: &mut Ui, st: &mut AppState, d: &mut SettingsDlg, pal: &Pal) -> bool {
    let p = &mut d.ach;
    if p.signins.is_none() && p.reading.is_none() {
        p.read_signins(ui.ctx());
    }
    p.poll();
    let before = st.config.achievements.clone();
    let mut ch = false;
    let w = (ui.available_width() - 150.0).clamp(160.0, 340.0);
    let mut open_url: Option<&'static str> = None;

    heading(ui, "Achievements", pal);
    hint_text(
        ui,
        "Your progress on each game's page. Lodestar only shows achievements: games and emulators unlock them.",
        pal,
    );
    ui.add_space(6.0);
    let c = &mut st.config.achievements;
    card(ui, pal, |ui| {
        ch |=
            switch_row(ui, "Show achievements on game pages", "Steam and RetroAchievements progress", &mut c.show, pal);
        row_separator(ui, pal);
        let show = c.show;
        ui.add_enabled_ui(show, |ui| {
            ch |= switch_row(
                ui,
                "Show counts in the library (covers, list and controller mode)",
                "How many you have unlocked, like 31 / 54, without opening the game",
                &mut c.library_counts,
                pal,
            );
        });
        row_separator(ui, pal);
        ch |= switch_row(ui, "Steam", "From the Steam client's own files, no sign-in needed", &mut c.steam, pal);
        row_separator(ui, pal);
        ch |= switch_row(
            ui,
            "RetroAchievements",
            "Emulated games, with your RetroAchievements username and Web API key",
            &mut c.retroachievements,
            pal,
        );
    });

    ui.add_space(14.0);
    heading(ui, "Steam Web API key (optional)", pal);
    hint_text(
        ui,
        "Only needed for games Steam hasn't synced to this computer yet. Your Steam profile's Game details \
         must be public for it to work.",
        pal,
    );
    if crate::ui::desktop::dialogs::link_text(ui, "Get a key on steamcommunity.com", STEAM_KEY_URL, pal) {
        open_url = Some(STEAM_KEY_URL);
    }
    ch |= crate::ui::desktop::sources_ui::value_field(
        ui,
        "Web API key",
        &mut c.steam_web_api_key,
        "optional",
        Some(&mut d.show_key),
        w,
    );

    ui.add_space(14.0);
    heading(ui, "RetroAchievements account", pal);
    hint_text(
        ui,
        "Your username and Web API key (not your password). The key is on retroachievements.org → Settings → \
         Keys.",
        pal,
    );
    if crate::ui::desktop::dialogs::link_text(ui, "Where do I find this?", RA_SETTINGS_URL, pal) {
        open_url = Some(RA_SETTINGS_URL);
    }
    let mut user = Some(c.ra_username.clone()).filter(|u| !u.is_empty());
    let user_ch = crate::ui::desktop::sources_ui::value_field(ui, "Username", &mut user, "username", None, w);
    let key_ch = crate::ui::desktop::sources_ui::value_field(
        ui,
        "Web API key",
        &mut c.ra_api_key,
        "Web API key",
        Some(&mut d.show_key),
        w,
    );
    if user_ch || key_ch {
        c.ra_username = user.unwrap_or_default().trim().to_string();
        // Not checked any more: the stored account id belonged to what was there before.
        c.ra_ulid = None;
        p.checked = false;
        p.debug_check = None;
        ch = true;
    }
    ch |= account_check(ui, st, p, pal);

    ui.add_space(14.0);
    emulators(ui, st, p, pal);

    if let Some(url) = open_url {
        st.open_url(url);
    }
    if st.config.achievements != before {
        // Answers so far followed the old settings.
        st.achievements.clear();
    }
    ch
}

/// "Check" and its answer: "Signed in as <name> ✓", the reason it failed, or a spinner. The
/// answer's account id and the name's own spelling are stored.
fn account_check(ui: &mut Ui, st: &mut AppState, p: &mut AchPage, pal: &Pal) -> bool {
    let mut ch = false;
    let c = &st.config.achievements;
    let ready = !c.ra_username.trim().is_empty() && c.ra_api_key.as_deref().is_some_and(|k| !k.trim().is_empty());
    let answer: Option<Option<Result<RaAccount, String>>> = if p.debug_check.is_some() {
        p.debug_check.clone()
    } else if p.checked {
        st.achievements.ra_account_check().map(|a| a.cloned())
    } else {
        None
    };
    let checking = matches!(answer, Some(None));
    ui.horizontal(|ui| {
        ui.add_space(134.0);
        let b = ui
            .add_enabled(!checking && ready, egui::Button::new("Check").min_size(vec2(80.0, 26.0)))
            .on_disabled_hover_text("Type your username and Web API key first");
        if b.clicked() {
            let key = c.ra_api_key.clone().unwrap_or_default();
            st.achievements.check_ra_account(&c.ra_username, &key);
            p.checked = true;
            p.stored = false;
            p.debug_check = None;
        }
        match &answer {
            Some(None) => {
                crate::ui::widgets::lazy_spinner(ui, 14.0, pal.accent);
                ui.label(RichText::new("Checking…").size(12.5).color(pal.dim));
            }
            Some(Some(Ok(acc))) => signed_in(ui, &acc.display_name, pal),
            Some(Some(Err(e))) => {
                // Never a key on screen, whatever the answer quoted.
                let mut e = e.clone();
                for k in st.config.achievements.secrets() {
                    e = e.replace(&k, "…");
                }
                ui.add(egui::Label::new(RichText::new(e).size(12.5).color(pal.danger)).wrap());
            }
            // Checked before (an earlier session): the stored account.
            None if ready && st.config.achievements.ra_ulid.is_some() => {
                signed_in(ui, &st.config.achievements.ra_username, pal)
            }
            None => {}
        }
    });
    if let Some(Some(Ok(acc))) = &answer
        && !p.stored
    {
        p.stored = true;
        let c = &mut st.config.achievements;
        c.ra_username = acc.display_name.clone();
        c.ra_ulid = Some(acc.ulid.clone());
        ch = true;
    }
    ch
}

fn signed_in(ui: &mut Ui, name: &str, pal: &Pal) {
    ui.label(RichText::new(format!("Signed in as {name} ✓")).size(12.5).color(tone(ChipTone::Good, pal)));
}

use crate::ui::desktop::emulation_ui::catalog::ChipTone;

/// "Your emulators": each one that supports RetroAchievements, signed in or not, with a way to
/// open it.
fn emulators(ui: &mut Ui, st: &mut AppState, p: &mut AchPage, pal: &Pal) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        // `heading` without its space above (it would push the title right in a row).
        ui.label(RichText::new("Your emulators").size(15.0).strong().color(pal.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(p.reading.is_none(), egui::Button::new("Refresh")).clicked() {
                p.read_signins(ui.ctx());
            }
        });
    });
    hint_text(
        ui,
        "Emulators unlock achievements themselves, so each one needs you signed in to RetroAchievements inside \
         it. Lodestar only reads whether they are.",
        pal,
    );
    ui.add_space(6.0);
    let mine = st.config.achievements.ra_username.trim().to_string();
    let mut open: Option<String> = None;
    card(ui, pal, |ui| match &p.signins {
        None => empty_row(ui, "Looking at your emulators…", true, pal),
        Some(list) if list.is_empty() => empty_row(
            ui,
            "No emulator with RetroAchievements found (RetroArch, DuckStation, PCSX2, PPSSPP, Dolphin, Flycast).",
            false,
            pal,
        ),
        Some(list) => {
            for (i, s) in list.iter().enumerate() {
                if i > 0 {
                    row_separator(ui, pal);
                }
                let (text, c) = status_text(&s.status, &mine, pal);
                let mut sub = vec![(text, c)];
                if !matches!(s.status, SigninStatus::SignedIn { .. }) || c != tone(ChipTone::Good, pal) {
                    sub.push((format!(" · Sign in under {}", s.where_to_sign_in), pal.dim));
                }
                row(ui, pal, &s.label, &sub, |ui| {
                    if ui
                        .button(format!("Open {}", short_name(&s.label)))
                        .on_hover_text(format!("Sign in under {}", s.where_to_sign_in))
                        .clicked()
                    {
                        open = Some(s.instance.clone());
                    }
                });
            }
        }
    });
    if let Some(instance) = open {
        let emu = crate::emulation::registry::current().and_then(|r| r.emulator(&instance).cloned());
        match emu {
            Some(e) => crate::ui::desktop::emulation_ui::open_emulator(st, &e),
            None => st.notify_error("That emulator isn't found any more".to_string()),
        }
    }
}

/// "DuckStation (Flatpak)" → "DuckStation".
fn short_name(label: &str) -> &str {
    label.split(" (").next().unwrap_or(label).trim()
}

/// What a row says about one emulator's sign-in, and its colour. `mine`: the username in
/// Settings ("" = not set up: any account counts).
pub(super) fn status_text(s: &SigninStatus, mine: &str, pal: &Pal) -> (String, egui::Color32) {
    match s {
        // RetroArch's sealed keychain: signed in, the name isn't readable.
        SigninStatus::SignedIn { username } if username.trim().is_empty() => {
            ("Signed in".into(), tone(ChipTone::Good, pal))
        }
        SigninStatus::SignedIn { username } if mine.is_empty() || username.eq_ignore_ascii_case(mine) => {
            (format!("Signed in as {username}"), tone(ChipTone::Good, pal))
        }
        SigninStatus::SignedIn { username } => {
            (format!("Signed in as {username}, not {mine}"), tone(ChipTone::Warn, pal))
        }
        SigninStatus::NotSignedIn => ("Not signed in".into(), tone(ChipTone::Warn, pal)),
        SigninStatus::Disabled => ("Achievements are off in it".into(), pal.dim),
        SigninStatus::Unknown => ("Couldn't read its settings".into(), pal.dim),
    }
}

fn switch_row(ui: &mut Ui, title: &str, sub: &str, on: &mut bool, pal: &Pal) -> bool {
    let mut changed = false;
    row(ui, pal, title, &[(sub.to_string(), pal.dim)], |ui| {
        changed = crate::ui::widgets::toggle_switch(ui, on, 20.0, pal.accent).changed();
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emulator_status_words() {
        let pal = Pal::from_palette(&crate::theme::model::Palette::default());
        let me = SigninStatus::SignedIn { username: "player1".into() };
        assert_eq!(status_text(&me, "Player1", &pal).0, "Signed in as player1");
        assert_eq!(status_text(&me, "", &pal).0, "Signed in as player1");
        assert_eq!(status_text(&me, "Other", &pal).0, "Signed in as player1, not Other");
        assert_eq!(status_text(&SigninStatus::NotSignedIn, "x", &pal).0, "Not signed in");
        let sealed = SigninStatus::SignedIn { username: String::new() };
        assert_eq!(status_text(&sealed, "Player1", &pal).0, "Signed in");
        assert_eq!(short_name("DuckStation (Flatpak)"), "DuckStation");
        assert_eq!(short_name("PCSX2"), "PCSX2");
    }
}
