//! Lodestar's own update on the desktop: the "What's new in <version>" dialog (release notes,
//! then Update now → progress → Restart now; the release page when Lodestar can't install it
//! itself), the line in Settings → General and the one in About.

use super::Pal;
use super::dialogs::{Buttons, footer, hint_text, link_text, modal, plain_button, primary_button, title};
use crate::app::AppState;
use crate::update_app::{Phase, UpdateState};
use egui::{RichText, Ui};

fn mb(b: u64) -> String {
    format!("{:.1} MB", b as f64 / 1_000_000.0)
}

/// "checked 3 hours ago".
fn checked_words(secs: i64) -> String {
    match secs.max(0) {
        s if s < 3600 => "checked just now".into(),
        s if s < 2 * 3600 => "checked an hour ago".into(),
        s if s < 86_400 => format!("checked {} hours ago", s / 3600),
        s if s < 2 * 86_400 => "checked yesterday".into(),
        s => format!("checked {} days ago", s / 86_400),
    }
}

/// About's button text, when there is an update to talk about.
pub(super) fn about_line(u: &UpdateState) -> Option<String> {
    let v = &u.latest.as_ref()?.version;
    Some(if u.ready() {
        format!("Restart to use {v}")
    } else if let Some(p) = u.percent() {
        format!("Downloading {v}… {p} %")
    } else if u.can_install() {
        format!("Update to {v}")
    } else {
        format!("Lodestar {v} is available")
    })
}

/// The progress bar of a running download, `room` narrower than the space left.
fn progress_bar(ui: &mut Ui, got: u64, total: u64, room: f32) {
    let f = if total == 0 { 0.0 } else { got as f32 / total as f32 };
    ui.add(egui::ProgressBar::new(f).desired_width((ui.available_width() - room).max(120.0)).text(format!(
        "Downloading… {} of {}",
        mb(got),
        mb(total)
    )));
}

/// The progress bar of a running download, with Cancel. True when Cancel was clicked.
fn progress(ui: &mut Ui, got: u64, total: u64) -> bool {
    let mut cancel = false;
    ui.horizontal(|ui| {
        progress_bar(ui, got, total, 100.0);
        cancel = plain_button(ui, "Cancel").clicked();
    });
    cancel
}

/// "What's new in 0.17.1": false once it closes.
pub(super) fn update_dialog(ctx: &egui::Context, st: &mut AppState, pal: &Pal, fam: &egui::FontFamily) -> bool {
    let Some(rel) = st.update.latest.clone() else { return false };
    let mut keep = true;
    let max_h = (ctx.content_rect().height() - 320.0).clamp(140.0, 380.0);
    let (_, close) = modal(ctx, "ls_update_dlg", pal, 540.0, |ui| {
        title(
            ui,
            &format!("What's new in {}", rel.version),
            Some(&format!("You have Lodestar {}", env!("CARGO_PKG_VERSION"))),
            pal,
            fam,
        );
        egui::ScrollArea::vertical().id_salt("ls_update_notes").max_height(max_h).auto_shrink([false, true]).show(
            ui,
            |ui| {
                if rel.notes.trim().is_empty() {
                    ui.label(RichText::new("This release came without notes.").color(pal.dim));
                } else {
                    crate::ui::whats_new::show(ui, &rel.notes, pal.text, pal.dim, pal.accent);
                }
            },
        );
        ui.add_space(12.0);
        let skipped = st.update_skipped() && matches!(st.update.phase, Phase::Idle | Phase::Failed(_));
        if skipped {
            hint_text(ui, "You chose to skip this version.", pal);
        }
        let mut skip = false;
        // "Skip this version" on the left of the footer, unless it is skipped already.
        let skip_button = |ui: &mut Ui| {
            if !skipped
                && plain_button(ui, "Skip this version")
                    .on_hover_text("Lodestar stops telling you about it; a newer version is announced again")
                    .clicked()
            {
                skip = true;
            }
        };
        let u = &st.update;
        if let Some(why) = u.why_not_installable() {
            hint_text(ui, &why, pal);
            let f = footer(ui, pal, Buttons::primary("Open the release page", true).cancel("Close"), skip_button);
            if f.primary {
                st.open_url(&rel.page);
            }
            if skip {
                st.skip_update();
            }
            keep = !f.cancel && !skip;
            return;
        }
        match u.phase.clone() {
            Phase::Downloading { got, total } => {
                hint_text(ui, "You can keep using Lodestar meanwhile, or close this.", pal);
                let f = footer(ui, pal, Buttons::default().cancel("Cancel"), |ui| progress_bar(ui, got, total, 0.0));
                if f.cancel {
                    st.cancel_update();
                }
            }
            Phase::Ready => {
                ui.label(
                    RichText::new(format!(
                        "Lodestar {} is installed. Restart now, or it starts the next time you open Lodestar.",
                        rel.version
                    ))
                    .color(pal.text),
                );
                let f = footer(ui, pal, Buttons::primary("Restart now", true).cancel("Later"), |_| {});
                if f.primary {
                    st.restart_after_update();
                }
                keep = !f.cancel;
            }
            phase => {
                if let Phase::Failed(e) = &phase {
                    ui.label(RichText::new(e).color(pal.danger));
                    ui.add_space(4.0);
                }
                hint_text(
                    ui,
                    "Lodestar downloads the new AppImage from GitHub, checks it and puts it in place of this \
                     one. Your library and settings stay as they are.",
                    pal,
                );
                let label = if matches!(phase, Phase::Failed(_)) { "Try again" } else { "Update now" };
                let f = footer(ui, pal, Buttons::primary(label, true).cancel("Not now"), skip_button);
                if f.primary {
                    st.start_update();
                }
                if skip {
                    st.skip_update();
                }
                keep = !f.cancel && !skip;
            }
        }
    });
    keep && !close
}

/// Settings → General → Updates. True when the setting changed.
pub(super) fn settings_section(ui: &mut Ui, st: &mut AppState, pal: &Pal) -> bool {
    let ch = ui.checkbox(&mut st.config.check_updates, "Check for updates automatically").changed();
    hint_text(
        ui,
        "Once a day Lodestar asks GitHub for the newest release. Nothing about you or your games is sent. \
         Switched off, Lodestar only asks when you want: Check now here, or Check for updates… in the menu.",
        pal,
    );
    ui.add_space(4.0);
    let u = &st.update;
    let (latest, phase, can_install) = (u.latest.clone(), u.phase.clone(), u.can_install());
    let (checking, checked_at) = (u.checking, u.checked_at);
    let mut open = false;
    let mut page = false;
    let mut cancel = false;
    let mut restart = false;
    let mut check = false;
    let mut skip = false;
    let skipped = st.update_skipped();
    match (&latest, phase) {
        (Some(r), Phase::Downloading { got, total }) => {
            ui.label(RichText::new(format!("Downloading Lodestar {}…", r.version)).color(pal.text));
            cancel = progress(ui, got, total);
        }
        (Some(r), Phase::Ready) => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Lodestar {} is installed.", r.version)).color(pal.text));
                restart = primary_button(ui, "Restart now", pal, true).clicked();
            });
        }
        (Some(r), _) => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Lodestar {} is available.", r.version)).strong().color(pal.text));
                let label = if can_install { format!("Update to {}", r.version) } else { "What's new".into() };
                open = primary_button(ui, &label, pal, true).clicked();
                if !can_install {
                    page = link_text(ui, "Release page", &r.page, pal);
                }
                if !skipped {
                    skip = ui
                        .button("Skip this version")
                        .on_hover_text("Lodestar stops telling you about it; a newer version is announced again")
                        .clicked();
                }
            });
            if skipped {
                hint_text(ui, "You chose to skip this version.", pal);
            }
        }
        (None, _) => {
            ui.horizontal(|ui| {
                let when = match checked_at {
                    _ if checking => "checking…".to_string(),
                    None => "not checked yet".into(),
                    Some(t) => checked_words(crate::model::now_unix() - t),
                };
                ui.label(
                    RichText::new(format!("You have Lodestar {} · {when}", env!("CARGO_PKG_VERSION"))).color(pal.dim),
                );
                check = ui.add_enabled(!checking, egui::Button::new("Check now")).clicked();
            });
            if let Some(n) = &st.update.note {
                hint_text(ui, n, pal);
            }
        }
    }
    if cancel {
        st.cancel_update();
    }
    if restart {
        st.restart_after_update();
    }
    if check {
        st.check_updates_now();
    }
    if skip {
        st.skip_update();
    }
    if open {
        st.update.show_notes = true;
    }
    if page {
        let p = st.update.page();
        st.open_url(&p);
    }
    ch
}
