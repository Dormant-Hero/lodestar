//! RomM server in the desktop UI: the Settings → Libraries → "RomM server" block (address,
//! token, Test connection, link to RomM's token docs, "More options"), the download progress
//! line under a RomM game's Download button, and the one-time "where should downloads go?"
//! question (romm_app.rs does the work).

use super::Pal;
use super::add::{Pick, PickKind};
use super::dialogs::{hint_text, link_text, modal, plain_button, primary_button};
use crate::app::AppState;
use crate::model::GameId;
use crate::ui::widgets::lazy_spinner;
use egui::{Color32, RichText, Ui, vec2};
use parking_lot::Mutex;
use std::path::PathBuf;

/// A folder dialog on its way: for the "where should downloads go?" question (Some(game)) or
/// More options → Download folder (None).
static PICK: Mutex<Option<(Option<GameId>, Pick)>> = Mutex::new(None);

const OK: Color32 = Color32::from_rgb(96, 186, 120);

/// One labelled text field. `secret`: hidden unless shown (Show / Hide next to it).
fn field(ui: &mut Ui, label: &str, value: &mut String, hint: &str, secret: bool) -> bool {
    let show_id = egui::Id::unique(("ls_romm_shown", label));
    let mut shown = ui.data(|d| d.get_temp::<bool>(show_id)).unwrap_or(false);
    let mut ch = false;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(vec2(118.0, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(118.0);
            ui.label(RichText::new(label).size(13.0));
        });
        // Same width with or without Show, so the fields line up.
        let w = (ui.available_width() - 72.0).clamp(100.0, 340.0);
        let r = ui.add(
            egui::TextEdit::singleline(value)
                .password(secret && !shown)
                .hint_text(hint)
                .desired_width(w)
                .font(egui::TextStyle::Monospace),
        );
        ch = r.changed();
        if secret && ui.small_button(if shown { "Hide" } else { "Show" }).clicked() {
            shown = !shown;
            ui.data_mut(|d| d.insert_temp(show_id, shown));
        }
    });
    ch
}

fn opt_field(ui: &mut Ui, label: &str, v: &mut Option<String>, hint: &str, secret: bool) -> bool {
    let mut s = v.clone().unwrap_or_default();
    let ch = field(ui, label, &mut s, hint, secret);
    if ch {
        let t = s.trim().to_string();
        *v = (!t.is_empty()).then_some(t);
    }
    ch
}

fn result_line(ui: &mut Ui, r: &Result<String, String>, pal: &Pal) {
    let (text, color) = match r {
        Ok(t) => (format!("✔ {t}"), OK),
        Err(e) => (e.clone(), pal.danger),
    };
    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(color)).wrap());
}

/// Settings → Libraries → RomM server (expanded). True when the settings changed.
pub(super) fn settings_block(ui: &mut Ui, st: &mut AppState, pal: &Pal) -> bool {
    // Never wider than the row (the settings window sizes to its content).
    ui.set_max_width(ui.available_width());
    let mut ch = false;
    hint_text(
        ui,
        "Your games on a RomM server. Lodestar lists them; Download puts a game in its console's ROM \
         folder, and it plays with your emulators.",
        pal,
    );
    ui.add_space(4.0);
    let mut url = st.config.romm.url.clone();
    if field(ui, "Server address", &mut url, "http://192.168.1.10:8080", false) {
        st.config.romm.url = url.trim().to_string();
        st.romm_settings_changed();
        ch = true;
    }
    if opt_field(ui, "API token", &mut st.config.romm.token, "rmm_…", true) {
        st.romm_settings_changed();
        ch = true;
    }
    ui.horizontal(|ui| {
        ui.add_space(122.0);
        ui.vertical(|ui| {
            ui.add(
                egui::Label::new(
                    RichText::new("In RomM: Administration → Client API Tokens → New token (read access is enough).")
                        .size(12.0)
                        .color(pal.dim),
                )
                .wrap(),
            );
            if link_text(ui, "How to make a token", crate::romm::TOKEN_DOCS_URL, pal) {
                st.open_url(crate::romm::TOKEN_DOCS_URL);
            }
        });
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let set = !st.config.romm.url.trim().is_empty();
        if ui.add_enabled(set && !st.romm.testing, egui::Button::new("Test connection")).clicked() {
            // Test what is typed now.
            st.save_config();
            st.romm_test();
        }
        if ui
            .add_enabled(set && !st.romm.syncing && st.romm_set_up(), egui::Button::new("Sync now"))
            .on_hover_text("Read the server's game list again (also done on every library scan)")
            .clicked()
        {
            st.save_config();
            st.romm_sync();
        }
        if st.romm.testing || st.romm.syncing {
            lazy_spinner(ui, 14.0, pal.dim);
            ui.label(
                RichText::new(if st.romm.testing { "Connecting…" } else { "Reading the server's games…" })
                    .size(12.5)
                    .color(pal.dim),
            );
        }
    });
    if let Some(r) = st.romm.test_result.clone() {
        result_line(ui, &r, pal);
    } else if let Some(r) = st.romm.sync_result.clone() {
        result_line(ui, &r, pal);
    }
    ui.add_space(4.0);
    egui::CollapsingHeader::new(RichText::new("More options").size(13.0))
        .id_salt("ls_romm_more")
        .default_open(false)
        .show(ui, |ui| {
            ch |= more_options(ui, st, pal);
        });
    ch
}

fn more_options(ui: &mut Ui, st: &mut AppState, pal: &Pal) -> bool {
    let mut ch = false;
    // Download folder.
    ui.label(RichText::new("Download folder").size(13.0).strong());
    hint_text(
        ui,
        "Games go to their console's ROM folder from Settings → Emulation. For consoles without one they go \
         here, one folder per console.",
        pal,
    );
    ui.horizontal(|ui| {
        let shown = st
            .config
            .romm
            .download_dir
            .as_ref()
            .map(|d| d.display().to_string())
            .unwrap_or_else(|| "Asked the first time".into());
        ui.label(RichText::new(shown).size(12.5).monospace());
        if ui.small_button("Choose…").clicked() {
            let start = st.config.romm.download_dir.clone();
            *PICK.lock() = Some((None, Pick::start(ui.ctx(), PickKind::Folder, "Folder for games from RomM", start)));
        }
        if st.config.romm.download_dir.is_some() && ui.small_button("Reset").clicked() {
            st.config.romm.download_dir = None;
            ch = true;
        }
    });
    ch |= poll_settings_pick(st);
    ui.add_space(8.0);
    // Pairing code.
    ui.label(RichText::new("Pairing code").size(13.0).strong());
    hint_text(ui, "Instead of the token: a short code from RomM's token list (Pair), used once.", pal);
    let code_id = egui::Id::unique("ls_romm_pair_code");
    let mut code = ui.data(|d| d.get_temp::<String>(code_id)).unwrap_or_default();
    ui.horizontal(|ui| {
        if ui.add(egui::TextEdit::singleline(&mut code).hint_text("12345678").desired_width(120.0)).changed() {
            ui.data_mut(|d| d.insert_temp(code_id, code.clone()));
        }
        let ready = !code.trim().is_empty() && !st.config.romm.url.trim().is_empty() && !st.romm.pairing;
        if ui.add_enabled(ready, egui::Button::new("Pair")).clicked() {
            st.save_config();
            st.romm_pair(code.trim());
            ui.data_mut(|d| d.remove_temp::<String>(code_id));
        }
        if st.romm.pairing {
            lazy_spinner(ui, 14.0, pal.dim);
        }
    });
    match &st.romm.pair_result {
        Some(Ok(())) => {
            ui.label(RichText::new("✔ Paired: the token is saved").size(12.5).color(OK));
        }
        Some(Err(e)) => {
            ui.add(egui::Label::new(RichText::new(e).size(12.5).color(pal.danger)).wrap());
        }
        None => {}
    }
    ui.add_space(8.0);
    // Older servers.
    ui.label(RichText::new("Older RomM servers").size(13.0).strong());
    hint_text(ui, "Servers without API tokens: your RomM user name and password instead.", pal);
    if opt_field(ui, "User name", &mut st.config.romm.username, "", false) {
        st.romm_settings_changed();
        ch = true;
    }
    if opt_field(ui, "Password", &mut st.config.romm.password, "", true) {
        st.romm_settings_changed();
        ch = true;
    }
    ui.add_space(6.0);
    if ui
        .checkbox(&mut st.config.romm.accept_invalid_certs, "Accept the server's own (self-signed) certificate")
        .on_hover_text("For a home server on https:// with a certificate it made itself")
        .changed()
    {
        st.romm_settings_changed();
        ch = true;
    }
    ch
}

fn poll_settings_pick(st: &mut AppState) -> bool {
    let mut guard = PICK.lock();
    let Some((None, pick)) = guard.as_ref() else { return false };
    let Some(paths) = pick.poll() else { return false };
    *guard = None;
    drop(guard);
    match paths.into_iter().next() {
        Some(p) => {
            st.config.romm.download_dir = Some(crate::romm::download::download_root(&p));
            true
        }
        None => false,
    }
}

/// Under a RomM game's Download button while it downloads: bar, numbers, Cancel.
pub(super) fn progress_line(ui: &mut Ui, st: &AppState, id: GameId, pal: &Pal) {
    let Some(p) = st.romm_progress(id) else { return };
    use std::sync::atomic::Ordering;
    let stopping = p.cancel.load(Ordering::Relaxed);
    let done = p.done.load(Ordering::Relaxed);
    let total = p.total.load(Ordering::Relaxed);
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let text = if stopping {
            "Stopping…".to_string()
        } else if total > 0 {
            format!(
                "Downloading… {} of {}",
                crate::romm::download::human(done.min(total)),
                crate::romm::download::human(total)
            )
        } else {
            format!("Downloading… {}", crate::romm::download::human(done))
        };
        let w = (ui.available_width() - 90.0).clamp(120.0, 320.0);
        ui.vertical(|ui| {
            ui.add(
                egui::ProgressBar::new(p.fraction().unwrap_or(0.0))
                    .desired_width(w)
                    .desired_height(8.0)
                    .fill(pal.accent),
            );
            ui.label(RichText::new(text).size(12.5).color(pal.text));
        });
        if ui
            .add_enabled(!stopping, egui::Button::new("Cancel"))
            .on_hover_text("Stop the download (nothing is kept)")
            .clicked()
        {
            // The download thread sees it at its next read and cleans up.
            p.cancel.store(true, Ordering::Relaxed);
        }
    });
    // A few repaints a second while it runs (the bytes come in on another thread).
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
}

/// The one-time "where should games from RomM go?" question (drawn every frame by the desktop;
/// nothing while no question is open).
pub(super) fn folder_prompt(ctx: &egui::Context, st: &mut AppState, pal: &Pal) {
    // A folder dialog for the question returned.
    let picked = {
        let mut guard = PICK.lock();
        match guard.as_ref() {
            Some((Some(id), pick)) => pick.poll().map(|paths| (*id, paths.into_iter().next())),
            _ => None,
        }
        .inspect(|_| *guard = None)
    };
    if let Some((id, dir)) = picked {
        if dir.is_some() {
            st.romm_folder_chosen(id, dir);
        }
        return;
    }
    let Some((id, suggest)) = st.romm.ask.clone() else { return };
    let picking = PICK.lock().is_some();
    let name = crate::ui::game_name(st, id);
    let console = st
        .library
        .get(id)
        .and_then(|g| g.platform.as_deref())
        .map(|p| super::platform_label(p).to_string())
        .unwrap_or_else(|| "this console".into());
    let shown = short(&suggest, &st.paths.home);
    let mut answer: Option<Option<PathBuf>> = None;
    let mut choose = false;
    let (_, close) = modal(ctx, "ls_romm_folder_prompt", pal, 460.0, |ui| {
        ui.label(RichText::new("Where should games from RomM go?").size(17.0).strong().color(pal.text));
        ui.add_space(6.0);
        ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "You have no ROM folder for {console} yet. Lodestar keeps downloaded games in one folder, with a \
                     folder per console inside it, and adds it to your ROM folders so {name} plays with your emulator."
                ))
                .size(13.0)
                .color(pal.text),
            )
            .wrap(),
        );
        ui.add_space(4.0);
        hint_text(ui, "Asked once; change it later in Settings → Libraries → RomM server → More options.", pal);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if primary_button(ui, &format!("Use {shown}"), pal, !picking).clicked() {
                answer = Some(Some(suggest.clone()));
            }
            if ui
                .add_enabled(!picking, egui::Button::new("Choose another folder…").min_size(vec2(80.0, 30.0)))
                .clicked()
            {
                choose = true;
            }
            if plain_button(ui, "Cancel").clicked() {
                answer = Some(None);
            }
        });
    });
    if choose {
        st.romm.ask = None;
        *PICK.lock() =
            Some((Some(id), Pick::start(ctx, PickKind::Folder, "Folder for games from RomM", Some(suggest))));
        return;
    }
    if close && answer.is_none() && !picking {
        answer = Some(None);
    }
    if let Some(a) = answer {
        st.romm_folder_chosen(id, a);
    }
}

fn short(p: &std::path::Path, home: &std::path::Path) -> String {
    match p.strip_prefix(home) {
        Ok(rest) if !rest.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => p.display().to_string(),
    }
}

/// Screenshot hooks (`LODESTAR_DESKTOP_OPEN=romm-…`, see DesktopUi).
pub(super) fn debug_open(
    what: &str,
    st: &mut AppState,
    id: GameId,
    settings: &mut Option<super::settings::SettingsDlg>,
) {
    match what {
        "-settings" => {
            let mut d = super::settings::SettingsDlg::new(super::settings::Tab::Library);
            d.expanded.push(crate::model::Source::Romm);
            *settings = Some(d);
        }
        "-ask" => st.romm.ask = Some((id, st.paths.home.join("ROMs"))),
        "-progress" => {
            let p = crate::romm::download::Progress::default();
            p.total.store(700 << 20, std::sync::atomic::Ordering::Relaxed);
            p.done.store(280 << 20, std::sync::atomic::Ordering::Relaxed);
            st.romm.downloads.insert(id, std::sync::Arc::new(p));
        }
        _ => {}
    }
}
