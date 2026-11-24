//! Menu → Changelog: the whole history built into this version, with Export (a web page,
//! the Markdown, plain text, plain text to the clipboard, or the web page opened in the browser).
//!
//! Saving never blocks the UI: the desktop's save dialog and the write run on a helper thread
//! (like the file pickers in `add.rs`); the result comes back as a toast.

use super::dialogs::{modal, plain_button, primary_button, title};
use super::{DesktopUi, Pal};
use crate::app::AppState;
use crate::ui::whats_new::export::Format;
use crate::ui::whats_new::{self, CHANGELOG};
use crate::ui::widgets::{Icon, paint_icon};
use crossbeam_channel::{Receiver, TryRecvError};
use egui::{Rect, vec2};
use std::path::{Path, PathBuf};

/// How a save ended.
enum Saved {
    Cancelled,
    Done(PathBuf),
    Failed(String),
}

/// A save dialog (then the write) running on its own thread. It lives outside the changelog
/// dialog, so closing that while the save dialog is still up still reports the result.
pub(crate) struct ChangelogSave {
    rx: Receiver<Saved>,
    /// Testing hook (`whatsnew-saving`): keeps the channel open so "Saving…" stays up.
    _hold: Option<crossbeam_channel::Sender<Saved>>,
}

/// Where the save dialog starts: ~/Documents when it exists, else the home folder.
fn default_dir() -> Option<PathBuf> {
    dirs::document_dir()
        .filter(|d| d.is_dir())
        .or_else(|| dirs::home_dir().map(|h| h.join("Documents")).filter(|d| d.is_dir()))
        .or_else(|| dirs::home_dir().filter(|d| d.is_dir()))
}

fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Write the changelog in `fmt` to `path` (adding the extension when the name has none).
fn write_export(fmt: Format, mut path: PathBuf) -> Saved {
    if path.extension().is_none() {
        path.set_extension(fmt.ext());
    }
    match std::fs::write(&path, fmt.render(CHANGELOG, version())) {
        Ok(()) => Saved::Done(path),
        Err(e) => Saved::Failed(format!("Couldn't save the changelog to {}: {e}", short(&path))),
    }
}

/// "Open in browser": the web-page export written to `dir` (the user's private runtime folder)
/// under a fixed name per version, so opening it again overwrites rather than piles up files.
fn write_browser_copy(dir: &Path) -> Result<PathBuf, String> {
    let path = dir.join(format!("lodestar-changelog-{}.html", version()));
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&path, Format::Html.render(CHANGELOG, version())))
        .map(|()| path)
        .map_err(|e| format!("Couldn't write the changelog page to {}: {e}", short(dir)))
}

impl ChangelogSave {
    fn start(ctx: &egui::Context, fmt: Format) -> ChangelogSave {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("changelog-save".into()).spawn(move || {
            let mut dlg = rfd::FileDialog::new()
                .set_title("Export the changelog")
                .set_file_name(fmt.default_file_name())
                .add_filter(fmt.filter_name(), &[fmt.ext()]);
            if let Some(dir) = default_dir() {
                dlg = dlg.set_directory(dir);
            }
            let r = match dlg.save_file() {
                Some(path) => write_export(fmt, path),
                None => Saved::Cancelled,
            };
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        if let Err(e) = spawned {
            // The sender went with the closure, so the next poll reports this.
            log::warn!("save dialog thread: {e}");
        }
        ChangelogSave { rx, _hold: None }
    }

    /// A save that never finishes (testing hook: shows the "Saving…" state headlessly).
    fn pending_forever() -> ChangelogSave {
        let (tx, rx) = crossbeam_channel::bounded(1);
        ChangelogSave { rx, _hold: Some(tx) }
    }

    /// Some(result) once done.
    fn poll(&self) -> Option<Saved> {
        match self.rx.try_recv() {
            Ok(r) => Some(r),
            // The helper ended without an answer (it couldn't start, or the dialog crashed).
            Err(TryRecvError::Disconnected) => {
                Some(Saved::Failed("Couldn't open the save dialog (see the log)".into()))
            }
            Err(TryRecvError::Empty) => None,
        }
    }
}

/// "/home/me/Documents/x" → "~/Documents/x".
fn short(p: &Path) -> String {
    match dirs::home_dir().and_then(|h| p.strip_prefix(&h).ok().map(Path::to_path_buf)) {
        Some(rest) => format!("~/{}", rest.display()),
        None => p.display().to_string(),
    }
}

fn report(st: &mut AppState, r: Saved) {
    match r {
        Saved::Cancelled => {}
        Saved::Done(p) => st.notify(format!("Changelog saved to {}", short(&p))),
        Saved::Failed(e) => st.notify_error(e),
    }
}

/// What the Export menu asked for this frame.
enum Export {
    Save(Format),
    Copy,
    Browser,
}

impl DesktopUi {
    /// The changelog dialog (when open) and any export still running.
    pub(super) fn whats_new_dialogs(&mut self, ctx: &egui::Context, st: &mut AppState) {
        if let Some(r) = self.dlg.changelog_save.as_ref().and_then(ChangelogSave::poll) {
            self.dlg.changelog_save = None;
            report(st, r);
        }
        if let Some(path) = self.dlg.changelog_export_to.take() {
            // Testing hook (`whatsnew-export:<path>`): write without the dialog.
            report(st, write_export(Format::from_path(&path), path));
        }
        if std::mem::take(&mut self.dlg.debug_export_saving) {
            self.dlg.changelog_save = Some(ChangelogSave::pending_forever());
        }
        if !self.dlg.whats_new {
            return;
        }
        let pal = self.pal;
        let fam = self.fonts.bold.clone();
        let saving = self.dlg.changelog_save.is_some();
        let open_menu = std::mem::take(&mut self.dlg.debug_export_menu);
        let (keep, export) = whats_new_dialog(ctx, &pal, &fam, saving, open_menu);
        match export {
            Some(Export::Save(fmt)) => self.dlg.changelog_save = Some(ChangelogSave::start(ctx, fmt)),
            Some(Export::Copy) => {
                ctx.copy_text(Format::Text.render(CHANGELOG, version()));
                st.notify("Changelog copied to the clipboard");
            }
            Some(Export::Browser) => match write_browser_copy(&crate::instance::runtime_dir()) {
                // The same way links open (and never in screenshot runs).
                Ok(p) => st.open_path(&p),
                Err(e) => st.notify_error(e),
            },
            None => {}
        }
        if !keep {
            self.dlg.whats_new = false;
        }
    }
}

/// The dialog itself: (keep it open, what Export asked for).
fn whats_new_dialog(
    ctx: &egui::Context,
    pal: &Pal,
    fam: &egui::FontFamily,
    saving: bool,
    open_menu: bool,
) -> (bool, Option<Export>) {
    let mut keep = true;
    let mut export = None;
    let max_h = (ctx.content_rect().height() - 200.0).clamp(240.0, 720.0);
    let (_, close) = modal(ctx, "ls_whats_new_dlg", pal, 720.0, |ui| {
        title(ui, "Changelog", Some(&format!("Lodestar {} · everything so far", version())), pal, fam);
        egui::ScrollArea::vertical().id_salt("ls_whats_new_scroll").max_height(max_h).auto_shrink([false, true]).show(
            ui,
            |ui| {
                whats_new::show(ui, CHANGELOG, pal.text, pal.dim, pal.accent);
            },
        );
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if primary_button(ui, "Close", pal, true).clicked() {
                keep = false;
            }
            let menu = export_button(ui, pal, saving);
            if open_menu {
                egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&menu));
            }
            egui::Popup::menu(&menu).show(|ui| {
                ui.set_min_width(190.0);
                for fmt in Format::ALL {
                    let b = ui.add_enabled(!saving, egui::Button::new(fmt.label()));
                    if b.on_hover_text(hover(fmt)).clicked() {
                        export = Some(Export::Save(fmt));
                    }
                }
                ui.separator();
                if ui
                    .button("Open in browser")
                    .on_hover_text("Show the changelog as a web page in your browser")
                    .clicked()
                {
                    export = Some(Export::Browser);
                }
                if ui.button("Copy to clipboard").on_hover_text("The whole changelog as plain text").clicked() {
                    export = Some(Export::Copy);
                }
            });
        });
    });
    (keep && !close, export)
}

fn hover(fmt: Format) -> &'static str {
    match fmt {
        Format::Html => "One file that opens in any web browser (follows its light / dark setting)",
        Format::Markdown => "The changelog's source, as written",
        Format::Text => "Readable anywhere: headings underlined, lines wrapped",
    }
}

/// "Export ▾" (the chevron drawn, the default fonts have no glyph for it).
fn export_button(ui: &mut egui::Ui, pal: &Pal, saving: bool) -> egui::Response {
    let resp = plain_button(ui, if saving { "Saving…      " } else { "Export      " });
    let r = resp.rect;
    let icon = Rect::from_center_size(r.right_center() - vec2(16.0, 0.0), vec2(12.0, 12.0));
    paint_icon(ui.painter(), icon, Icon::ChevronDown, pal.dim);
    resp.on_hover_text("Save the changelog as a file, open it in the browser or copy it")
}

#[cfg(test)]
mod tests;
