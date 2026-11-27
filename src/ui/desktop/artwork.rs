//! Artwork editing: the "Edit artwork" dialog (one tab per slot: current preview, choose a file,
//! drop / paste a file or URL, reset, search online sources and pick from a gallery), the bulk
//! "Download artwork" dialog, the pencil button over the details artwork and the status bar's
//! bulk-download progress.
//!
//! Everything here only talks to the art API on `AppState` (`set_art_slot`, `set_art_from_file`,
//! `set_art_from_url`, `search_art`, `bulk_art`, `cancel_bulk_art`, `st.art.sources()`,
//! `st.art_state`); downloads and searches run in the background and arrive through
//! `st.art_state`. The native file picker runs on its own thread and is polled.

use super::dialogs::{Buttons, body_height, footer, hint_text, modal, title};
use super::{DesktopUi, Pal};
use crate::app::AppState;
use crate::art::{ArtSlot, BulkOptions, Candidate, SourceInfo};
use crate::art_app::SearchState;
use crate::images::{ImageCache, ImgSize};
use crate::model::*;
use crate::ui::widgets::{
    Icon, contain_rect, lazy_spinner, mix, paint_contain, paint_cover, paint_icon, truncated, vgradient, with_alpha,
};
use crossbeam_channel::{Receiver, TryRecvError};
use egui::{Color32, Key, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// The gallery and the bulk dialog.
mod gallery;

mod bulk;

pub(crate) use bulk::*;
use gallery::*;

// ------------------------------------------------------------------------------- helpers

/// What the user typed / pasted / dropped as the new image.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ArtInput {
    Url(String),
    File(PathBuf),
}

/// Interpret a typed or pasted path / URL: `http(s)://…` → URL; `file://…` (percent-decoded),
/// `~/…` and plain paths → file. Quotes and surrounding whitespace are ignored; of several
/// lines (a file manager's copied file list) the first non-empty one counts.
pub(crate) fn parse_art_input(s: &str) -> Option<ArtInput> {
    let line = s.lines().map(str::trim).find(|l| !l.is_empty())?;
    let line = line.trim_matches(|c| c == '"' || c == '\'').trim();
    if line.is_empty() {
        return None;
    }
    let lower = line.get(..8).unwrap_or(line).to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(ArtInput::Url(line.to_string()));
    }
    if let Some(rest) = line.strip_prefix("file://") {
        // file:///home/… (empty host) or file://localhost/home/…
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        return rest.starts_with('/').then(|| ArtInput::File(PathBuf::from(percent_decode(rest))));
    }
    if line.contains("://") {
        return None; // some other scheme we can't fetch
    }
    Some(ArtInput::File(super::settings::expand_tilde(line)))
}

/// `%20` → ' ' etc. (invalid escapes are kept as they are).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Display aspect (width / height) of a slot.
pub(crate) fn slot_aspect(slot: ArtSlot) -> f32 {
    match slot {
        ArtSlot::Cover => 2.0 / 3.0,
        ArtSlot::Hero => 1920.0 / 620.0,
        ArtSlot::Logo => 2.6,
        ArtSlot::Icon => 1.0,
        ArtSlot::Banner => 460.0 / 215.0,
    }
}

/// Gallery cell: minimum width before another column fits.
fn cell_min_w(slot: ArtSlot) -> f32 {
    match slot {
        ArtSlot::Cover => 118.0,
        ArtSlot::Hero => 250.0,
        ArtSlot::Logo => 190.0,
        ArtSlot::Icon => 86.0,
        ArtSlot::Banner => 205.0,
    }
}

/// Is `path` Lodestar's own user art for game `id` (`…/user/<id>/<slot>.<ext>`)?
#[cfg(test)]
pub(crate) fn is_user_art(path: &Path, id: GameId) -> bool {
    let Some(dir) = path.parent() else {
        return false;
    };
    let id_ok = dir.file_name().and_then(|n| n.to_str()).is_some_and(|n| n == id.to_string());
    id_ok && dir.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "user")
}

/// "600×900", "" when unknown.
fn size_text(w: u32, h: u32) -> String {
    if w > 0 && h > 0 { format!("{w}×{h}") } else { String::new() }
}

/// The key under which a candidate's thumbnail is stored in `art_state.thumbs`.
fn thumb_key(c: &Candidate) -> &str {
    c.thumb_url.as_deref().unwrap_or(&c.url)
}

const IMAGE_EXTS: [&str; 10] = ["png", "jpg", "jpeg", "webp", "gif", "bmp", "ico", "svg", "qoi", "avif"];

/// Start the native file picker on its own thread (xdg-desktop-portal can take a while and must
/// never block the UI); the result arrives on the returned channel.
fn spawn_picker(ctx: &egui::Context, slot: ArtSlot, start: Option<PathBuf>) -> Receiver<Option<PathBuf>> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name("art-picker".into())
        .spawn(move || {
            let mut dlg = rfd::FileDialog::new()
                .set_title(format!("Choose {} image", slot.label().to_lowercase()))
                .add_filter("Images", &IMAGE_EXTS);
            if let Some(dir) = start.filter(|d| d.is_dir()) {
                dlg = dlg.set_directory(dir);
            }
            let picked = dlg.pick_file();
            let _ = tx.send(picked);
            ctx.request_repaint();
        })
        .map(|_| ())
        .unwrap_or_else(|e| log::warn!("file picker thread: {e}"));
    rx
}

// ------------------------------------------------------------------------------- edit dialog

pub(crate) struct ArtDlg {
    pub id: GameId,
    name: String,
    pub slot: ArtSlot,
    /// Path / URL field.
    input: String,
    input_error: Option<String>,
    /// Search term (defaults to the game's name).
    pub query: String,
    default_query: String,
    /// Term each slot was last searched with from this dialog.
    searched: Vec<(ArtSlot, String)>,
    /// The user asked for online results: switching tabs searches the new slot too.
    pub online: bool,
    /// Native file picker running for this slot.
    picker: Option<(ArtSlot, Receiver<Option<PathBuf>>)>,
    /// Last candidate clicked per slot (highlighted; shows progress while it downloads).
    picked: Vec<(ArtSlot, String)>,
    /// Sources, refreshed every second (cheap, but not every frame).
    sources: Vec<SourceInfo>,
    /// Show only this source's results (source name; None = all).
    pub filter: Option<String>,
    sources_at: Option<Instant>,
    /// Testing hook: apply the first result once the search has some.
    pub debug_pick: bool,
    /// Another dialog (Settings) is open on top: drops and Ctrl+V are not for us then.
    pub covered: bool,
    /// Testing hook: close the dialog once nothing is downloading any more (after a pick /
    /// an import / a reset), so the screenshot shows the updated library behind it.
    pub debug_close: bool,
    /// Testing hook: submit this text through the path / URL field ("Use").
    pub debug_input: Option<String>,
    /// Testing hook: press "Reset to default" for the current slot.
    pub debug_reset: bool,
    /// The background preview being dragged / zoomed: the crop so far (saved when it ends).
    bg_live: Option<(Option<BackgroundPlacement>, Option<f64>)>,
}

/// What the edit dialog wants after this frame.
pub(crate) enum ArtOutcome {
    Keep,
    Close,
    /// Keep the dialog and open Settings → Metadata on top (API keys).
    OpenSettings,
}

impl ArtDlg {
    pub fn new(g: &Game, slot: ArtSlot) -> ArtDlg {
        let name = g.display_name().to_string();
        ArtDlg {
            id: g.id,
            name: name.clone(),
            slot,
            input: String::new(),
            input_error: None,
            query: name.clone(),
            default_query: name,
            searched: vec![],
            online: false,
            picker: None,
            picked: vec![],
            sources: vec![],
            filter: None,
            sources_at: None,
            debug_pick: false,
            covered: false,
            debug_close: false,
            debug_input: None,
            debug_reset: false,
            bg_live: None,
        }
    }

    fn searched_for(&self, slot: ArtSlot) -> Option<&str> {
        self.searched.iter().find(|(s, _)| *s == slot).map(|(_, q)| q.as_str())
    }

    fn picked_for(&self, slot: ArtSlot) -> Option<&str> {
        self.picked.iter().find(|(s, _)| *s == slot).map(|(_, u)| u.as_str())
    }

    /// Start a search for the current slot with the current term.
    fn search(&mut self, st: &mut AppState) {
        self.online = true;
        let q = self.query.trim().to_string();
        let q = if q.is_empty() {
            self.query = self.default_query.clone();
            self.default_query.clone()
        } else {
            q
        };
        // The game's own name: let the backend use the Steam appid / its own matching.
        let custom = (q != self.default_query).then_some(q.as_str());
        st.search_art(self.id, self.slot, custom);
        self.searched.retain(|(s, _)| *s != self.slot);
        self.searched.push((self.slot, q));
    }

    fn apply_input(&mut self, st: &mut AppState, text: &str) {
        match parse_art_input(text) {
            Some(ArtInput::Url(u)) => {
                self.input_error = None;
                self.picked.retain(|(s, _)| *s != self.slot);
                self.picked.push((self.slot, u.clone()));
                st.set_art_from_url(self.id, self.slot, &u);
            }
            Some(ArtInput::File(p)) => {
                if p.is_file() {
                    self.input_error = None;
                    st.set_art_from_file(self.id, self.slot, &p);
                } else {
                    self.input_error = Some(format!("File not found: {}", p.display()));
                }
            }
            None => {
                self.input_error = Some("Enter an image file path or an http(s):// URL".to_string());
            }
        }
    }
}

/// Draw the edit dialog (state in `d`; actions go straight to the art API on `st`).
pub(crate) fn art_dialog(
    ctx: &egui::Context,
    st: &mut AppState,
    d: &mut ArtDlg,
    pal: &Pal,
    fam: &egui::FontFamily,
) -> ArtOutcome {
    if st.library.get(d.id).is_none() {
        return ArtOutcome::Close; // removed meanwhile
    }
    let mut out = ArtOutcome::Keep;
    let screen = ctx.content_rect();
    let width = (screen.width() - 80.0).clamp(560.0, 900.0);
    // Short windows: the body scrolls instead of the dialog running off the screen.
    let in_main = (screen.height() - 200.0).max(200.0);
    let (_, close) = modal(ctx, "ls_art_dlg", pal, width, |ui| {
        title(ui, "Edit artwork", Some(&d.name), pal, fam);
        let body_h = body_height(ui, in_main);
        out = art_editor(ui, st, d, pal, fam, body_h);
        if footer(ui, pal, Buttons::primary("Done", true), |_| {}).primary {
            out = ArtOutcome::Close;
        }
    });
    if d.debug_close && !d.debug_pick && d.debug_input.is_none() && !d.debug_reset && st.art_state.applying.is_empty() {
        out = ArtOutcome::Close;
    }
    if close && matches!(out, ArtOutcome::Keep) {
        // Esc closes the dialog (not while the file picker is still open: its result would
        // be lost).
        if d.picker.is_none() {
            out = ArtOutcome::Close;
        }
    }
    out
}

/// The Background preview (the whole picture, `r`): drag the frame to choose what part the game
/// page shows, scroll to zoom, double-click to reset (Arrange mode's placement).
fn background_drag(
    ui: &mut Ui,
    st: &mut AppState,
    d: &mut ArtDlg,
    r: Rect,
    tex: egui::Vec2,
    crop: Option<BackgroundPlacement>,
) {
    use gallery::{HEADER_SHAPE, crop_window};
    let resp = ui
        .interact(r, egui::Id::NULL.with(("ls_art_bg_drag", d.id)), Sense::click_and_drag())
        .on_hover_text("Drag the frame to choose what shows · scroll to zoom · double-click to reset");
    let align = vec2(0.5, 0.3);
    let cur = crop.unwrap_or_else(|| {
        BackgroundPlacement::from_uv(aligned_uv(tex, HEADER_SHAPE, align), tex, HEADER_SHAPE, align)
    });
    let win = crop_window(r, tex, Some(cur));
    let now = ui.input(|i| i.time);
    let scroll = ui.input(|i| i.smooth_scroll_delta.y);
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    let mut save = None;
    if resp.double_clicked() {
        d.bg_live = None;
        save = Some(None);
    } else if resp.dragged() && win.width() > 0.0 && win.height() > 0.0 {
        // The frame follows the pointer: the opposite of dragging the picture on the page.
        let dd = resp.drag_delta();
        let delta = vec2(-dd.x / win.width() * HEADER_SHAPE.x, -dd.y / win.height() * HEADER_SHAPE.y);
        d.bg_live = Some((Some(cur.panned(tex, HEADER_SHAPE, delta)), None));
    } else if resp.hovered() && scroll.abs() > 0.1 && win.width() > 0.0 && win.height() > 0.0 {
        ui.ctx().input_mut(|i| i.smooth_scroll_delta.y = 0.0);
        let anchor = ui.input(|i| i.pointer.hover_pos()).map_or(vec2(0.5, 0.5), |p| (p - win.min) / win.size());
        let z = cur.zoomed(tex, HEADER_SHAPE, anchor, (1.0 + scroll * 0.002).clamp(0.8, 1.25));
        d.bg_live = Some((Some(z), Some(now)));
    }
    if resp.drag_stopped()
        && let Some((c, _)) = d.bg_live.take()
    {
        save = Some(c);
    }
    // A zoom is saved once the wheel pauses.
    if let Some((c, Some(t))) = d.bg_live {
        if now - t >= 0.4 {
            d.bg_live = None;
            save = Some(c);
        } else {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
    if let Some(c) = save
        && let Some(g) = st.library.get(d.id)
    {
        let l = super::details::Layout { bg: c, ..super::details::Layout::of(&g.edits) };
        super::details::save_layout(st, d.id, l);
    }
}

/// Is the native file picker open for this editor (Esc must not close it then)?
impl ArtDlg {
    pub fn picking(&self) -> bool {
        self.picker.is_some()
    }
}

/// The edited game's artwork as of this frame (copied: the editor changes the game while drawing).
struct EditedArt {
    art: Art,
    /// The Logo tab previews the logo over the game's background.
    hero_for_logo: Option<PathBuf>,
    saved_crop: Option<BackgroundPlacement>,
    /// Console (emulated) games: by the console the art sources know it as (an emulated game
    /// without a library platform still has its emulation platform).
    console: bool,
}

/// The artwork editor itself (slot tabs, current image, file / URL, online search), shared by
/// the standalone dialog and Edit Game Details' Media tab. `body_h`: height it may scroll in.
pub(crate) fn art_editor(
    ui: &mut Ui,
    st: &mut AppState,
    d: &mut ArtDlg,
    pal: &Pal,
    fam: &egui::FontFamily,
    body_h: f32,
) -> ArtOutcome {
    let ctx = ui.ctx().clone();
    let Some(game) = st.library.get(d.id) else {
        return ArtOutcome::Close; // removed meanwhile
    };
    let edited = EditedArt {
        art: game.art.clone(),
        hero_for_logo: game.art.hero.clone(),
        saved_crop: game.edits.background_placement,
        console: crate::art::sources::console_platform(crate::art::sources::art_platform(game).as_deref()).is_some(),
    };
    let list = crate::art::sources::art_list(game);
    if d.sources_at.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
        // The source list of Settings → Metadata this game uses (PC games / emulated games).
        d.sources = st.art.sources_for(list);
        d.sources_at = Some(Instant::now());
    }
    let hovering_files = take_outside_input(&ctx, st, d);
    // Switching tabs after asking for online results searches the new slot as well.
    if d.online && d.searched_for(d.slot).is_none() && !st.art_state.searches.contains_key(&(d.id, d.slot)) {
        d.search(st);
    }

    let mut out = ArtOutcome::Keep;
    let gallery_h = (body_h - 462.0).clamp(150.0, 420.0);
    egui::ScrollArea::vertical().id_salt("ls_art_body").max_height(body_h).auto_shrink([false, true]).show(ui, |ui| {
        slot_tabs(ui, d, &edited.art, pal);
        current_image(ui, st, d, &edited, pal, fam);
        ui.add_space(16.0);
        if online_section(ui, st, d, edited.console, gallery_h, pal, fam) {
            out = ArtOutcome::OpenSettings;
        }
    });
    if hovering_files {
        drop_highlight(ui, d.slot, pal);
    }
    out
}

/// Images arriving from outside the editor: the file picker's answer, a dropped file
/// (X11 / XWayland; winit can't receive drops on native Wayland yet) and Ctrl+V with no text
/// field focused (a copied file's file:// URI, a path or an image URL). Returns whether files
/// are being dragged over the window.
fn take_outside_input(ctx: &egui::Context, st: &mut AppState, d: &mut ArtDlg) -> bool {
    if let Some((slot, rx)) = &d.picker {
        match rx.try_recv() {
            Ok(Some(p)) => {
                let slot = *slot;
                d.picker = None;
                st.set_art_from_file(d.id, slot, &p);
            }
            Ok(None) | Err(TryRecvError::Disconnected) => d.picker = None,
            Err(TryRecvError::Empty) => {}
        }
    }
    let hovering_files = ctx.input(|i| !i.raw.hovered_files.is_empty());
    let hovering_files = hovering_files && !d.covered;
    let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
    if !d.covered
        && let Some(p) = dropped
    {
        d.input_error = None;
        st.set_art_from_file(d.id, d.slot, &p);
    }
    if !d.covered
        && !ctx.text_edit_focused()
        && let Some(text) = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Paste(s) => Some(s.clone()),
                _ => None,
            })
        })
    {
        d.apply_input(st, &text);
    }
    hovering_files
}

/// One tab per slot (dimmed when the slot has no image) over a thin rule.
fn slot_tabs(ui: &mut Ui, d: &mut ArtDlg, art: &Art, pal: &Pal) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for slot in ArtSlot::ALL {
            let has = slot.get(art).is_some();
            if slot_tab(ui, slot.label(), d.slot == slot, has, pal).clicked() {
                d.slot = slot;
                d.input_error = None;
            }
        }
    });
    let sep_y = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), sep_y, Stroke::new(1.0, with_alpha(pal.border, 160)));
    ui.add_space(14.0);
}

/// The slot's current image (the Background one can be dragged / zoomed) beside where it comes
/// from and the ways to replace it.
fn current_image(
    ui: &mut Ui,
    st: &mut AppState,
    d: &mut ArtDlg,
    edited: &EditedArt,
    pal: &Pal,
    fam: &egui::FontFamily,
) {
    let slot = d.slot;
    let current = slot.get(&edited.art).cloned();
    ui.horizontal_top(|ui| {
        // Same height on every tab (the dialog is centred: no jumping around).
        ui.set_min_height(212.0);
        let (prev_rect, _) = ui.allocate_exact_size(vec2(300.0, 200.0), Sense::hover());
        let crop = d.bg_live.map_or(edited.saved_crop, |l| l.0);
        let shown = paint_preview(
            ui.painter(),
            &mut st.images,
            slot,
            current.as_deref(),
            edited.hero_for_logo.as_deref(),
            prev_rect,
            pal,
            &d.name,
            crop,
        );
        if let Some((r, tex)) = shown {
            background_drag(ui, st, d, r, tex, crop);
        }
        ui.add_space(16.0);
        ui.vertical(|ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(slot.label()).family(fam.clone()).size(16.0).strong().color(pal.text));
            hint_text(ui, slot.hint(), pal);
            ui.add_space(6.0);
            image_origin(ui, st, d, current.as_deref(), pal);
            ui.add_space(10.0);
            file_buttons(ui, st, d, current.as_deref());
            ui.add_space(8.0);
            path_or_url_field(ui, st, d);
            if let Some(e) = &d.input_error {
                ui.label(RichText::new(e).size(12.0).color(pal.danger));
            } else {
                hint_text(ui, "Tip: drop an image file on this window, or copy one and press Ctrl+V.", pal);
            }
        });
    });
}

/// Where the current image comes from ("Your image", the game's source, none), and its file.
fn image_origin(ui: &mut Ui, st: &AppState, d: &ArtDlg, current: Option<&Path>, pal: &Pal) {
    let (origin, origin_col) = match current {
        Some(p) if crate::art::is_user_art_file(st.art.art_dir(), d.id, p) => ("Your image", pal.accent),
        Some(_) => ("From the game's source", pal.dim),
        None => ("No image yet", pal.dim),
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new(origin).size(12.5).strong().color(origin_col));
        if st.art_state.applying.contains(&(d.id, d.slot)) {
            lazy_spinner(ui, 12.0, pal.accent);
            ui.label(RichText::new("Updating…").size(12.0).color(pal.dim));
        }
    });
    if let Some(p) = current {
        let full = p.display().to_string();
        let text = if crate::art::is_user_art_file(st.art.art_dir(), d.id, p) {
            RichText::new("Saved in Lodestar's art folder").size(12.0)
        } else {
            RichText::new(&full).size(11.5).monospace()
        };
        let r = ui.add(egui::Label::new(text.color(mix(pal.dim, pal.bg, 0.15))).truncate());
        r.on_hover_text(full);
    }
}

/// Choose file… (the native picker, on its own thread) and Reset to default.
fn file_buttons(ui: &mut Ui, st: &mut AppState, d: &mut ArtDlg, current: Option<&Path>) {
    let slot = d.slot;
    ui.horizontal(|ui| {
        let picking = d.picker.is_some();
        let label = if picking { "Waiting for file…" } else { "Choose file…" };
        if ui
            .add_enabled(!picking, egui::Button::new(label).min_size(vec2(110.0, 30.0)))
            .on_hover_text("Pick an image on this computer (it is copied into Lodestar's art folder)")
            .clicked()
        {
            let start = current
                .filter(|p| !crate::art::is_user_art_file(st.art.art_dir(), d.id, p))
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .or_else(dirs::picture_dir)
                .or_else(dirs::home_dir);
            d.picker = Some((slot, spawn_picker(ui.ctx(), slot, start)));
        }
        let user = current.is_some_and(|p| crate::art::is_user_art_file(st.art.art_dir(), d.id, p));
        let reset = ui
            .add_enabled(user, egui::Button::new("Reset to default").min_size(vec2(0.0, 30.0)))
            .on_hover_text("Go back to the artwork the game's source provides")
            .on_disabled_hover_text("This is already the default artwork");
        if reset.clicked() || std::mem::take(&mut d.debug_reset) {
            st.set_art_slot(d.id, slot, None);
            d.picked.retain(|(s, _)| *s != slot);
        }
    });
}

/// A file path or image URL typed / pasted into the field (Enter or Use).
fn path_or_url_field(ui: &mut Ui, st: &mut AppState, d: &mut ArtDlg) {
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut d.input)
                .hint_text("Paste a file path or an image URL")
                .desired_width((ui.available_width() - 60.0).max(120.0))
                .margin(egui::Margin::symmetric(8, 6)),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let ok = !d.input.trim().is_empty();
        let use_btn = ui.add_enabled(ok, egui::Button::new("Use").min_size(vec2(48.0, 30.0)));
        if let Some(t) = d.debug_input.take() {
            d.input = t;
            d.apply_input(st, &d.input.clone());
            d.input.clear();
        }
        if ok && (enter || use_btn.clicked()) {
            let text = std::mem::take(&mut d.input);
            d.apply_input(st, &text);
            if d.input_error.is_some() {
                d.input = text; // keep it so the user can fix it
            }
        }
    });
}

/// Find online: the search bar, the source chips, the results gallery and a line about the
/// results. True when a chip asked for Settings → Metadata.
fn online_section(
    ui: &mut Ui,
    st: &mut AppState,
    d: &mut ArtDlg,
    console: bool,
    gallery_h: f32,
    pal: &Pal,
    fam: &egui::FontFamily,
) -> bool {
    let slot = d.slot;
    let mut want_search = search_bar(ui, d, pal, fam);
    ui.add_space(4.0);
    let key = (d.id, slot);
    let search = st.art_state.searches.get(&key);
    let open_settings = source_chips(ui, &d.sources, search, slot, console, &mut d.filter, pal);
    ui.add_space(8.0);

    // The source filter (chips above): only that source's results.
    let filter = d.filter.clone();
    let shown: Vec<&Candidate> = search
        .map(|s| s.items.iter().filter(|c| filter.as_ref().is_none_or(|f| c.source == *f)).collect())
        .unwrap_or_default();
    let mut clear_filter = false;
    let picked = d.picked_for(slot).map(str::to_string);
    let applying = st.art_state.applying.contains(&key);
    let mut clicked: Option<String> = None;
    let mut cta = false;
    egui::Frame::new()
        .fill(mix(pal.bg, pal.surface, 0.35))
        .stroke(Stroke::new(1.0, with_alpha(pal.border, 140)))
        .corner_radius(8)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height(gallery_h);
            match search {
                None => {
                    cta = gallery_empty(
                        ui,
                        pal,
                        &format!(
                            "Look for {} images on {}.",
                            slot.label().to_lowercase(),
                            source_names(&d.sources, slot, console)
                        ),
                        d.sources.iter().any(|s| s.available).then_some("Find online"),
                    );
                }
                Some(s) if shown.is_empty() && !s.done => {
                    let term = d.searched_for(slot).unwrap_or(&d.default_query);
                    searching_note(ui, term, gallery_h, pal);
                }
                Some(s) if shown.is_empty() && !s.items.is_empty() => {
                    // Results, but none from the source picked above.
                    clear_filter = none_from_source(ui, filter.as_deref().unwrap_or(""), slot, s, gallery_h, pal);
                }
                Some(s) if s.items.is_empty() => {
                    let q = d.searched_for(slot).unwrap_or(&d.default_query);
                    nothing_found(ui, q, slot, s, gallery_h, pal);
                }
                Some(_) => {
                    if let Some(url) =
                        gallery(ui, &mut st.images, &st.art_state, &shown, slot, picked.as_deref(), applying, pal)
                    {
                        clicked = Some(url);
                    }
                }
            }
        });
    results_line(ui, search, shown.len(), pal);
    want_search |= cta;
    if d.debug_pick
        && let Some(c) = shown.first()
    {
        d.debug_pick = false;
        clicked = Some(c.url.clone());
    }
    if let Some(url) = clicked {
        d.picked.retain(|(s, _)| *s != slot);
        d.picked.push((slot, url.clone()));
        st.set_art_from_url(d.id, slot, &url);
    }
    if clear_filter {
        d.filter = None;
    }
    if want_search {
        d.search(st);
    }
    open_settings
}

/// "Find online", the search term, Search and ↺ (back to the game's name). True: search now.
fn search_bar(ui: &mut Ui, d: &mut ArtDlg, pal: &Pal, fam: &egui::FontFamily) -> bool {
    let mut want_search = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Find online").family(fam.clone()).size(16.0).strong().color(pal.text));
        ui.add_space(8.0);
        let r = ui.add(
            egui::TextEdit::singleline(&mut d.query)
                .hint_text("Search term")
                .desired_width((ui.available_width() - 100.0).clamp(140.0, 340.0))
                .margin(egui::Margin::symmetric(8, 5)),
        );
        want_search |= r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        let any = d.sources.iter().any(|s| s.available);
        want_search |= ui
            .add_enabled(any, egui::Button::new(RichText::new("Search").strong()).min_size(vec2(76.0, 28.0)))
            .on_disabled_hover_text("No online artwork source is available")
            .clicked();
        if d.query.trim() != d.default_query
            && ui
                .add(egui::Button::new(RichText::new("↺").size(14.0)).frame(false))
                .on_hover_text("Search for the game's name again")
                .clicked()
        {
            d.query = d.default_query.clone();
            want_search = true;
        }
    });
    want_search
}

/// The gallery while a search runs and nothing has arrived yet.
fn searching_note(ui: &mut Ui, term: &str, gallery_h: f32, pal: &Pal) {
    ui.add_space(gallery_h * 0.36);
    ui.vertical_centered(|ui| {
        lazy_spinner(ui, 22.0, pal.accent);
        ui.add_space(6.0);
        ui.label(RichText::new(format!("Searching for “{term}”…")).color(pal.dim));
    });
}

/// Results, but none from the source picked in the chips. True: "Show all" was clicked.
fn none_from_source(ui: &mut Ui, source: &str, slot: ArtSlot, s: &SearchState, gallery_h: f32, pal: &Pal) -> bool {
    let mut show_all = false;
    ui.add_space(gallery_h * 0.34);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(format!("No {} from {source}", plural(slot))).size(15.0).color(pal.text));
        ui.add_space(4.0);
        if ui.link(format!("Show all {} results", s.items.len())).clicked() {
            show_all = true;
        }
    });
    show_all
}

/// A finished search without results (and the sources' errors).
fn nothing_found(ui: &mut Ui, query: &str, slot: ArtSlot, s: &SearchState, gallery_h: f32, pal: &Pal) {
    ui.add_space(gallery_h * 0.3);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(format!("No {} found for “{query}”", plural(slot))).size(15.0).color(pal.text));
        ui.label(RichText::new("Try a shorter or different search term.").color(pal.dim));
        errors(ui, &s.errors, pal);
    });
}

/// "N results · click one to use it", still searching, source errors. Always one line high, so
/// the dialog doesn't grow when results arrive.
fn results_line(ui: &mut Ui, search: Option<&SearchState>, n: usize, pal: &Pal) {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), 18.0),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            let Some(s) = search.filter(|s| !s.items.is_empty()) else {
                return;
            };
            ui.label(
                RichText::new(format!("{n} result{} · click one to use it", if n == 1 { "" } else { "s" }))
                    .size(12.0)
                    .color(pal.dim),
            );
            if !s.done {
                lazy_spinner(ui, 12.0, pal.dim);
                ui.label(RichText::new("still searching…").size(12.0).color(pal.dim));
            }
            if !s.errors.is_empty() {
                let r = ui
                    .label(RichText::new(format!("· {} source error(s)", s.errors.len())).size(12.0).color(pal.danger));
                r.on_hover_text(s.errors.join("\n"));
            }
        },
    );
}

/// Files dragged over the window: a highlight over the whole editor.
fn drop_highlight(ui: &Ui, slot: ArtSlot, pal: &Pal) {
    let r = ui.min_rect().expand(10.0);
    let p = ui.painter();
    p.rect_filled(r, 8.0, with_alpha(pal.bg, 215));
    p.rect_stroke(r.shrink(4.0), 8.0, Stroke::new(2.0, pal.accent), StrokeKind::Inside);
    let ir = Rect::from_center_size(r.center() - vec2(0.0, 26.0), vec2(40.0, 40.0));
    paint_icon(p, ir, Icon::Image, pal.accent);
    p.text(
        r.center() + vec2(0.0, 14.0),
        egui::Align2::CENTER_CENTER,
        format!("Drop to use as the {}", slot.label().to_lowercase()),
        egui::FontId::proportional(17.0),
        pal.text,
    );
}

fn plural(slot: ArtSlot) -> &'static str {
    match slot {
        ArtSlot::Cover => "covers",
        ArtSlot::Hero => "backgrounds",
        ArtSlot::Logo => "logos",
        ArtSlot::Icon => "icons",
        ArtSlot::Banner => "banners",
    }
}

/// The available sources that can have `slot` images for this game (libretro: console games
/// only), as "A, B and C".
fn source_names(sources: &[SourceInfo], slot: ArtSlot, console: bool) -> String {
    let names: Vec<&str> = sources
        .iter()
        .filter(|s| s.available && s.kind.has_slot(slot))
        .filter(|s| console || s.kind != crate::art::SourceKind::Libretro)
        .map(|s| s.name.as_str())
        .collect();
    match names.len() {
        0 => "the web".into(),
        1 => names[0].into(),
        _ => {
            let (last, rest) = names.split_last().unwrap_or((&"", &[]));
            format!("{} and {last}", rest.join(", "))
        }
    }
}

fn errors(ui: &mut Ui, errs: &[String], pal: &Pal) {
    for e in errs.iter().take(3) {
        ui.label(RichText::new(e).size(12.0).color(pal.danger));
    }
}

// ------------------------------------------------------------------------------- pencil button

/// Small round "edit artwork" (pencil) button in the top-right corner of `area`, shown while
/// `show` (the pointer is over the artwork) or while it is hovered itself. True when clicked.
pub(crate) fn edit_art_button(ui: &mut Ui, area: Rect, show: bool, id_salt: &str, tooltip: &str, pal: &Pal) -> bool {
    art_corner_button(ui, area, show, id_salt, tooltip, pal, Icon::Pencil, false)
}

/// The "view full size" (magnifier) button: like [`edit_art_button`], in the top-left corner.
pub(crate) fn view_art_button(ui: &mut Ui, area: Rect, show: bool, id_salt: &str, tooltip: &str, pal: &Pal) -> bool {
    art_corner_button(ui, area, show, id_salt, tooltip, pal, Icon::Search, true)
}

#[allow(clippy::too_many_arguments)]
fn art_corner_button(
    ui: &mut Ui,
    area: Rect,
    show: bool,
    id_salt: &str,
    tooltip: &str,
    pal: &Pal,
    icon: Icon,
    left: bool,
) -> bool {
    let size = 30.0;
    let inset = 10.0;
    let x = if left { area.left() + inset } else { area.right() - size - inset };
    let rect = Rect::from_min_size(pos2(x, area.top() + inset), vec2(size, size));
    let id = ui.scope_id().with(("ls_edit_art", id_salt, left));
    let show = show && !ui.ctx().any_popup_open();
    let resp = ui.interact(rect, id, Sense::click());
    let t = ui.ctx().animate_bool_with_time(id, show || resp.hovered(), 0.12);
    if t <= 0.0 {
        return false;
    }
    let p = ui.painter();
    let hov = resp.hovered();
    let bg = if hov { mix(Color32::from_gray(20), pal.accent, 0.45) } else { Color32::from_black_alpha(150) };
    let a = |c: Color32| {
        let [r, g, b, al] = c.to_srgba_unmultiplied();
        Color32::from_rgba_unmultiplied(r, g, b, (al as f32 * t) as u8)
    };
    p.circle_filled(rect.center(), size / 2.0, a(bg));
    p.circle_stroke(rect.center(), size / 2.0 - 0.5, Stroke::new(1.0, a(Color32::from_white_alpha(50))));
    paint_icon(p, rect.shrink(8.0), icon, a(Color32::from_gray(240)));
    let clicked = resp.clicked();
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tooltip);
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_urls_and_paths() {
        assert_eq!(
            parse_art_input("  https://cdn.example.com/a.png \n"),
            Some(ArtInput::Url("https://cdn.example.com/a.png".into()))
        );
        assert_eq!(parse_art_input("HTTP://x/y.jpg"), Some(ArtInput::Url("HTTP://x/y.jpg".into())));
        assert_eq!(
            parse_art_input("file:///home/me/My%20Pics/cover.png"),
            Some(ArtInput::File("/home/me/My Pics/cover.png".into()))
        );
        assert_eq!(parse_art_input("file://localhost/tmp/a.png"), Some(ArtInput::File("/tmp/a.png".into())));
        // A file manager's copied list: first line wins.
        assert_eq!(parse_art_input("\n'/tmp/a b.png'\n/tmp/c.png"), Some(ArtInput::File("/tmp/a b.png".into())));
        assert_eq!(parse_art_input("ftp://x/y"), None);
        assert_eq!(parse_art_input("   "), None);
        assert_eq!(parse_art_input("file://relative"), None);
        match parse_art_input("~/x.png") {
            Some(ArtInput::File(p)) => assert!(!p.starts_with("~")),
            o => panic!("{o:?}"),
        }
    }

    #[test]
    fn percent_decoding_keeps_invalid_escapes() {
        assert_eq!(percent_decode("a%2Fb%zz%"), "a/b%zz%");
        assert_eq!(percent_decode("%C3%A9"), "é");
    }

    #[test]
    fn user_art_paths() {
        assert!(is_user_art(Path::new("/d/art/user/42/cover.png"), 42));
        assert!(!is_user_art(Path::new("/d/art/user/43/cover.png"), 42));
        assert!(!is_user_art(Path::new("/d/art/steam/42/cover.png"), 42));
        assert!(!is_user_art(Path::new("cover.png"), 42));
    }

    #[test]
    fn source_name_lists() {
        let s = |n: &str, a: bool| SourceInfo {
            name: n.into(),
            available: a,
            reason: None,
            kind: crate::art::SourceKind::Steam,
            enabled: true,
        };
        let cover = ArtSlot::Cover;
        assert_eq!(source_names(&[], cover, false), "the web");
        assert_eq!(source_names(&[s("Steam", true), s("SteamGridDB", false)], cover, false), "Steam");
        assert_eq!(
            source_names(&[s("Steam", true), s("SteamGridDB", true), s("X", true)], cover, false),
            "Steam, SteamGridDB and X"
        );
        // Only sources with that kind of image; libretro for console games only.
        let k = |n: &str, kind| SourceInfo { kind, ..s(n, true) };
        use crate::art::SourceKind as K;
        let all = [k("Steam", K::Steam), k("libretro", K::Libretro), k("PCGamingWiki", K::PcGamingWiki)];
        assert_eq!(source_names(&all, cover, false), "Steam and PCGamingWiki");
        assert_eq!(source_names(&all, cover, true), "Steam, libretro and PCGamingWiki");
        assert_eq!(source_names(&all, ArtSlot::Logo, true), "Steam");
    }
}
