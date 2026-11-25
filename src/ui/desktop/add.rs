//! Add Game: Pick a program… (detect what a file is, Proton + prefix for Windows programs,
//! duplicate check), Scan for games… (the "Select games to import" checklist), the merge
//! suggestion banner, and the background file / folder picker the editors share.
//!
//! Nothing here blocks the UI thread: the native file dialog, detecting a picked program and
//! listing Proton builds all run on helper threads; folder scans run in `AppState`.

use super::dialogs::{Buttons, footer, hint_text, modal, title};
use super::{DesktopUi, Pal};
use crate::addgame::{DupMatch, DupReason, ImportCandidate, ImportSource, ProgramInfo, ProgramKind, ProtonVersion};
use crate::app::AppState;
use crate::edit_app::NewGame;
use crate::model::*;
use crate::ui::widgets::{lazy_spinner, mix, with_alpha};
use crossbeam_channel::{Receiver, TryRecvError};
use egui::{RichText, Sense, Ui, pos2, vec2};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

// ------------------------------------------------------------------------------- picker

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PickKind {
    File,
    /// Several files.
    Files,
    Folder,
    Image,
    /// One .zip archive (add-on packages).
    Zip,
    /// A Playnite theme: a .pthm package or a theme folder's theme.yaml.
    PlayniteTheme,
}

/// A native file dialog running on its own thread (xdg-desktop-portal can take a while and
/// must never block the UI).
pub(crate) struct Pick {
    rx: Receiver<Vec<PathBuf>>,
}

const IMAGE_EXTS: [&str; 9] = ["png", "jpg", "jpeg", "webp", "gif", "bmp", "ico", "svg", "avif"];

impl Pick {
    pub fn start(ctx: &egui::Context, kind: PickKind, title: &str, start: Option<PathBuf>) -> Pick {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let ctx = ctx.clone();
        let title = title.to_string();
        let spawned = std::thread::Builder::new().name("file-picker".into()).spawn(move || {
            let mut dlg = rfd::FileDialog::new().set_title(title);
            if let Some(dir) = start.filter(|d| d.is_dir()).or_else(|| dirs::home_dir().filter(|d| d.is_dir())) {
                dlg = dlg.set_directory(dir);
            }
            if kind == PickKind::Image {
                dlg = dlg.add_filter("Images", &IMAGE_EXTS);
            }
            if kind == PickKind::Zip {
                dlg = dlg.add_filter("Zip archives", &["zip"]);
            }
            if kind == PickKind::PlayniteTheme {
                dlg = dlg.add_filter("Playnite themes (.pthm, theme.yaml)", &["pthm", "zip", "yaml"]);
            }
            let picked: Vec<PathBuf> = match kind {
                PickKind::Folder => dlg.pick_folder().into_iter().collect(),
                PickKind::Files => dlg.pick_files().unwrap_or_default(),
                PickKind::File | PickKind::Image | PickKind::Zip | PickKind::PlayniteTheme => {
                    dlg.pick_file().into_iter().collect()
                }
            };
            let _ = tx.send(picked);
            ctx.request_repaint();
        });
        if let Err(e) = spawned {
            log::warn!("file picker thread: {e}");
        }
        Pick { rx }
    }

    /// Some(paths) once the dialog closed (empty: cancelled).
    pub fn poll(&self) -> Option<Vec<PathBuf>> {
        match self.rx.try_recv() {
            Ok(v) => Some(v),
            Err(TryRecvError::Disconnected) => Some(vec![]),
            Err(TryRecvError::Empty) => None,
        }
    }
}

/// Run `f` on a helper thread; its result arrives on the channel (with a repaint).
pub(super) fn background<T: Send + 'static>(
    ctx: &egui::Context,
    name: &str,
    f: impl FnOnce() -> T + Send + 'static,
) -> Receiver<T> {
    let (tx, rx) = crossbeam_channel::bounded(1);
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new().name(name.into()).spawn(move || {
        let _ = tx.send(f());
        ctx.request_repaint();
    });
    if let Err(e) = spawned {
        log::warn!("{name} thread: {e}");
    }
    rx
}

pub(crate) fn kind_label(k: ProgramKind) -> &'static str {
    match k {
        ProgramKind::Linux => "Linux program",
        ProgramKind::AppImage => "AppImage",
        ProgramKind::Script => "Script",
        ProgramKind::Windows => "Windows program",
        ProgramKind::Desktop => "Desktop launcher",
        ProgramKind::Flatpak => "Flatpak app",
    }
}

fn platform_for(k: ProgramKind) -> &'static str {
    if k == ProgramKind::Windows { "windows" } else { "linux" }
}

/// "/home/me/Games/x" → "~/Games/x".
fn tilde(p: &Path, home: &Path) -> String {
    match p.strip_prefix(home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => p.display().to_string(),
    }
}

/// A path short enough for one line: "~/…" and, when still long, only its last parts
/// ("…/Hades/x64/Hades.exe"); the full path goes in the tooltip.
pub(crate) fn short_path(p: &Path, home: &Path, max_chars: usize) -> String {
    let t = tilde(p, home);
    if t.chars().count() <= max_chars {
        return t;
    }
    let parts: Vec<&str> = t.split('/').filter(|s| !s.is_empty()).collect();
    for keep in (1..=parts.len().min(4)).rev() {
        let tail = format!("…/{}", parts[parts.len() - keep..].join("/"));
        if tail.chars().count() <= max_chars || keep == 1 {
            return tail;
        }
    }
    t
}

/// Proton builds, listed on a helper thread (directory listings).
struct Protons {
    list: Option<Vec<ProtonVersion>>,
    rx: Option<Receiver<Vec<ProtonVersion>>>,
    sel: usize,
    /// Preselect the build this existing prefix was last run with (when the list arrives).
    prefer_prefix: Option<PathBuf>,
}

impl Protons {
    fn start(ctx: &egui::Context, st: &AppState) -> Protons {
        let sctx = crate::scan::ScanContext::from_config(&st.config);
        Protons {
            list: None,
            rx: Some(background(ctx, "proton-list", move || crate::addgame::proton_versions(&sctx))),
            sel: 0,
            prefer_prefix: None,
        }
    }

    fn poll(&mut self) {
        if let Some(rx) = &self.rx
            && let Ok(v) = rx.try_recv()
        {
            if let Some(prefix) = &self.prefer_prefix
                && let Some(p) = crate::addgame::proton_for_prefix(prefix, &v)
                && let Some(i) = v.iter().position(|x| x.path == p.path)
            {
                self.sel = i;
            }
            self.list = Some(v);
            self.rx = None;
        }
    }

    fn selected(&self) -> Option<&ProtonVersion> {
        self.list.as_ref().and_then(|l| l.get(self.sel))
    }

    /// Combo box (or "looking…" / "none found"). Returns false when there's nothing to pick.
    fn ui(&mut self, ui: &mut Ui, id: &str, width: f32, pal: &Pal) -> bool {
        match &self.list {
            None => {
                lazy_spinner(ui, 14.0, pal.accent);
                ui.label(RichText::new("Looking for Proton…").size(12.5).color(pal.dim));
                false
            }
            Some(l) if l.is_empty() => {
                ui.label(
                    RichText::new("No Proton found. Install one in Steam, or GE-Proton with ProtonUp-Qt.")
                        .size(12.5)
                        .color(pal.danger),
                );
                false
            }
            Some(l) => {
                let cur = l.get(self.sel).map_or("", |p| p.name.as_str());
                egui::ComboBox::from_id_salt(id).width(width).selected_text(cur).show_ui(ui, |ui| {
                    for (i, p) in l.iter().enumerate() {
                        ui.selectable_value(&mut self.sel, i, &p.name).on_hover_text(p.path.display().to_string());
                    }
                });
                true
            }
        }
    }
}

// ------------------------------------------------------------------------------- Pick a program

enum Stage {
    Picking(Pick),
    Detecting(PathBuf, Receiver<Result<ProgramInfo, String>>),
    Ready(ProgramInfo),
    Failed(PathBuf, String),
}

pub(crate) struct ProgramDlg {
    stage: Stage,
    name: String,
    protons: Option<Protons>,
    prefix: String,
    prefix_edited: bool,
    prefix_pick: Option<Pick>,
    dups: Vec<DupMatch>,
    dups_for: Option<String>,
    allow_dup: bool,
    error: Option<String>,
}

pub(crate) enum ProgramOutcome {
    Keep,
    Close,
    Added(GameId),
    /// Open Edit Game Details on the new game, pre-filled.
    EditDetails(Box<NewGame>),
    /// Select an existing game (the duplicate) and close.
    Show(GameId),
}

impl ProgramDlg {
    /// Open the file dialog first.
    pub fn pick(ctx: &egui::Context) -> ProgramDlg {
        ProgramDlg::with_stage(Stage::Picking(Pick::start(
            ctx,
            PickKind::File,
            "Choose a game: program, script, AppImage, .exe or .desktop file",
            None,
        )))
    }

    /// As if `path` had been picked.
    pub fn for_path(ctx: &egui::Context, path: PathBuf) -> ProgramDlg {
        ProgramDlg::with_stage(detect(ctx, path))
    }

    fn with_stage(stage: Stage) -> ProgramDlg {
        ProgramDlg {
            stage,
            name: String::new(),
            protons: None,
            prefix: String::new(),
            prefix_edited: false,
            prefix_pick: None,
            dups: vec![],
            dups_for: None,
            allow_dup: false,
            error: None,
        }
    }

    /// The launch as configured, when it's complete.
    fn launch(&self) -> Option<LaunchSpec> {
        let Stage::Ready(info) = &self.stage else {
            return None;
        };
        if info.kind == ProgramKind::Windows {
            let p = self.protons.as_ref()?.selected()?;
            let prefix = self.prefix.trim();
            if prefix.is_empty() {
                return None;
            }
            Some(crate::addgame::windows_launch(&info.exe, p, Path::new(prefix)))
        } else {
            info.launch.clone()
        }
    }

    fn new_game(&self, info: &ProgramInfo) -> NewGame {
        NewGame {
            name: self.name.trim().to_string(),
            launch: self.launch(),
            art: Art { icon: info.icon.clone(), ..Default::default() },
            install_dir: info.install_dir.clone(),
            platform: Some(platform_for(info.kind).into()),
            edits: UserEdits::default(),
        }
    }
}

fn detect(ctx: &egui::Context, path: PathBuf) -> Stage {
    let p = path.clone();
    let rx =
        background(ctx, "detect-program", move || crate::addgame::detect_program(&p).map_err(|e| format!("{e:#}")));
    Stage::Detecting(path, rx)
}

fn dup_text(reason: &DupReason) -> &'static str {
    match reason {
        DupReason::SameProgram => "This program is already in your library",
        DupReason::SameName => "A game with this name is already in your library",
        DupReason::HiddenCopy => "A hidden copy of this game is in your library",
    }
}

fn program_dialog(
    ctx: &egui::Context,
    st: &mut AppState,
    d: &mut ProgramDlg,
    pal: &Pal,
    fam: &egui::FontFamily,
) -> ProgramOutcome {
    // Advance the background stages.
    let next = match &d.stage {
        Stage::Picking(p) => match p.poll() {
            Some(v) => match v.into_iter().next() {
                Some(path) => Some(detect(ctx, path)),
                None => return ProgramOutcome::Close, // cancelled
            },
            None => None,
        },
        Stage::Detecting(path, rx) => match rx.try_recv() {
            Ok(Ok(info)) => Some(Stage::Ready(info)),
            Ok(Err(e)) => Some(Stage::Failed(path.clone(), e)),
            Err(TryRecvError::Disconnected) => Some(Stage::Failed(path.clone(), "Detection stopped".into())),
            Err(TryRecvError::Empty) => None,
        },
        _ => None,
    };
    if let Some(s) = next {
        if let Stage::Ready(info) = &s {
            d.name = info.name.clone();
            if info.kind == ProgramKind::Windows {
                let mut p = Protons::start(ctx, st);
                // A program inside an existing Wine prefix (Faugus, Lutris, Heroic, Proton …)
                // keeps running in it, with the build that prefix uses.
                p.prefer_prefix = crate::addgame::prefix_of(&info.exe);
                d.protons = Some(p);
            }
        }
        d.stage = s;
    }
    if let Some(p) = &mut d.protons {
        p.poll();
    }
    if let Some(pick) = &d.prefix_pick
        && let Some(v) = pick.poll()
    {
        d.prefix_pick = None;
        if let Some(p) = v.into_iter().next() {
            d.prefix = p.display().to_string();
            d.prefix_edited = true;
        }
    }
    if let Stage::Ready(info) = &d.stage {
        if !d.prefix_edited && info.kind == ProgramKind::Windows {
            let name = if d.name.trim().is_empty() { "Game" } else { d.name.trim() };
            d.prefix = crate::addgame::prefix_of(&info.exe)
                .unwrap_or_else(|| crate::addgame::default_prefix(&st.paths, name))
                .display()
                .to_string();
        }
        // Duplicate check: in memory, only when the name changes.
        if d.dups_for.as_deref() != Some(d.name.as_str()) {
            d.dups = crate::addgame::find_duplicates(&st.library, d.name.trim(), info.launch.as_ref(), Some(&info.exe));
            d.dups_for = Some(d.name.clone());
        }
    }

    let mut out = ProgramOutcome::Keep;
    let home = st.paths.home.clone();
    let (_, close) = modal(ctx, "ls_program_dlg", pal, 540.0, |ui| {
        title(ui, "Add a program", None, pal, fam);
        match &d.stage {
            Stage::Picking(_) => {
                ui.horizontal(|ui| {
                    lazy_spinner(ui, 16.0, pal.accent);
                    ui.label(RichText::new("Choose a file in the file dialog…").color(pal.dim));
                });
                hint_text(
                    ui,
                    "A Linux program, script or AppImage, a Windows .exe (runs with Proton), or a \
                     .desktop launcher.",
                    pal,
                );
            }
            Stage::Detecting(path, _) => {
                ui.horizontal(|ui| {
                    lazy_spinner(ui, 16.0, pal.accent);
                    ui.label(RichText::new("Looking at the file…").color(pal.dim));
                });
                hint_text(ui, &short_path(path, &home, 70), pal);
            }
            Stage::Failed(path, e) => {
                ui.label(RichText::new(format!("Couldn't use {}", short_path(path, &home, 60))).color(pal.text));
                ui.label(RichText::new(e).size(12.5).color(pal.danger));
                ui.add_space(4.0);
                hint_text(
                    ui,
                    "You can still add it by hand: Add game → Manually…, then set its Play \
                     action.",
                    pal,
                );
            }
            Stage::Ready(info) => {
                let info = info.clone();
                program_form(ui, st, d, &info, pal, &home, &mut out);
            }
        }
        let ready = match &d.stage {
            Stage::Ready(info) => Some(info.clone()),
            _ => None,
        };
        let blocked = !d.dups.is_empty() && !d.allow_dup;
        let buttons = if ready.is_some() {
            let ok = !d.name.trim().is_empty() && d.launch().is_some() && !blocked;
            Buttons::primary("Add", ok).disabled_hint(if blocked {
                "Choose what to do about the copy above"
            } else {
                "Fill in the name (and Proton for Windows programs)"
            })
        } else if matches!(d.stage, Stage::Failed(..)) {
            Buttons::primary("Choose another…", true)
        } else {
            Buttons::default()
        };
        let f = footer(ui, pal, buttons.cancel("Cancel"), |ui| {
            if let Some(info) = &ready
                && ui
                    .add(egui::Button::new("Edit details…").min_size(vec2(0.0, 30.0)))
                    .on_hover_text("Open Edit game details with this program filled in")
                    .clicked()
            {
                out = ProgramOutcome::EditDetails(Box::new(d.new_game(info)));
            }
            if let Some(e) = &d.error {
                ui.label(RichText::new(e).size(12.5).color(pal.danger));
            }
        });
        if f.primary {
            match &ready {
                Some(info) => match st.add_game(d.new_game(info)) {
                    Some(id) => out = ProgramOutcome::Added(id),
                    None => d.error = Some("Couldn't add the game".into()),
                },
                None => d.stage = Stage::Picking(Pick::start(ui.ctx(), PickKind::File, "Choose a game", None)),
            }
        }
        if f.cancel {
            out = ProgramOutcome::Close;
        }
    });
    // Esc closes, but not while a file dialog is open (it would come back to nothing).
    let picking = matches!(d.stage, Stage::Picking(_)) || d.prefix_pick.is_some();
    if close && matches!(out, ProgramOutcome::Keep) && !picking {
        out = ProgramOutcome::Close;
    }
    out
}

fn program_form(
    ui: &mut Ui,
    st: &mut AppState,
    d: &mut ProgramDlg,
    info: &ProgramInfo,
    pal: &Pal,
    home: &Path,
    out: &mut ProgramOutcome,
) {
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
        let mut drawn = false;
        if let Some(icon) = &info.icon
            && let Some(tex) = st.images.get(icon, crate::images::ImgSize::Thumb)
        {
            crate::ui::widgets::paint_contain(ui.painter(), r, &tex, egui::Color32::WHITE, vec2(0.5, 0.5));
            drawn = true;
        }
        if !drawn {
            ui.painter().rect_filled(r, 10.0, mix(pal.surface_alt, pal.accent, 0.15));
            ui.painter().text(
                r.center(),
                egui::Align2::CENTER_CENTER,
                crate::ui::widgets::initials(if d.name.is_empty() { "?" } else { &d.name }),
                egui::FontId::proportional(20.0),
                pal.text,
            );
        }
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut d.name)
                    .hint_text("Name")
                    .font(egui::FontId::proportional(16.0))
                    .desired_width(f32::INFINITY)
                    .margin(egui::Margin::symmetric(8, 5)),
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new(kind_label(info.kind)).size(12.5).strong().color(mix(pal.text, pal.dim, 0.3)));
                ui.add(
                    egui::Label::new(
                        RichText::new(short_path(&info.exe, home, 48)).size(12.0).monospace().color(pal.dim),
                    )
                    .truncate(),
                )
                .on_hover_text(info.exe.display().to_string());
            });
        });
    });
    if info.kind == ProgramKind::Windows {
        ui.add_space(10.0);
        egui::Grid::new("ls_program_win").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
            ui.label(RichText::new("Proton").size(13.0).color(pal.dim));
            ui.horizontal(|ui| {
                if let Some(p) = &mut d.protons {
                    p.ui(ui, "ls_program_proton", 260.0, pal);
                }
            });
            ui.end_row();
            ui.label(RichText::new("Wine prefix").size(13.0).color(pal.dim));
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut d.prefix)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(300.0)
                        .margin(egui::Margin::symmetric(7, 4)),
                );
                if r.changed() {
                    d.prefix_edited = true;
                }
                if ui.add_enabled(d.prefix_pick.is_none(), egui::Button::new("Choose…")).clicked() {
                    let start = Some(PathBuf::from(d.prefix.trim())).and_then(|p| p.parent().map(Path::to_path_buf));
                    d.prefix_pick = Some(Pick::start(ui.ctx(), PickKind::Folder, "Choose a Wine prefix folder", start));
                }
            });
            ui.end_row();
        });
        let in_prefix = crate::addgame::prefix_of(&info.exe).is_some_and(|p| Path::new(d.prefix.trim()) == p);
        hint_text(
            ui,
            if in_prefix {
                "Runs with umu-run in the Wine prefix the program is installed in."
            } else {
                "Runs with umu-run. A new prefix is set up on the first start; pick an existing \
                 one to share it with another launcher."
            },
            pal,
        );
    }
    if !d.dups.is_empty() && !d.allow_dup {
        ui.add_space(10.0);
        let m = d.dups[0].clone();
        let (other, src) =
            st.library.get(m.id).map(|g| (g.display_name().to_string(), g.source.display())).unwrap_or_default();
        egui::Frame::new()
            .fill(with_alpha(pal.fav, 22))
            .stroke(egui::Stroke::new(1.0, with_alpha(pal.fav, 90)))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(dup_text(&m.reason)).strong().color(pal.text));
                ui.label(RichText::new(format!("“{other}” from {src}")).size(12.5).color(pal.dim));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if m.reason == DupReason::HiddenCopy {
                        if ui.button("Restore it").on_hover_text("Unhide the copy that's already there").clicked() {
                            st.restore_game(m.id);
                            *out = ProgramOutcome::Show(m.id);
                        }
                    } else if ui.button("Show it").clicked() {
                        *out = ProgramOutcome::Show(m.id);
                    }
                    if ui.button("Add anyway").clicked() {
                        d.allow_dup = true;
                    }
                });
            });
    }
}

// ------------------------------------------------------------------------------- Scan for games

/// A picked file and what it turned out to be.
type Detected = (PathBuf, Result<ProgramInfo, String>);

struct Row {
    c: ImportCandidate,
    name: String,
    checked: bool,
}

pub(crate) struct ImportDlg {
    rows: Vec<Row>,
    seen: HashSet<PathBuf>,
    /// `st.import_state.candidates.len()` already merged into `rows`.
    synced: usize,
    hide_imported: bool,
    browse: Option<Pick>,
    folder: Option<Pick>,
    detecting: Option<Receiver<Vec<Detected>>>,
    protons: Option<Protons>,
    error: Option<String>,
}

pub(crate) enum ImportOutcome {
    Keep,
    Close,
    Added(Vec<GameId>),
}

impl ImportDlg {
    /// Open the checklist and start looking: in `folder`, or for installed games.
    pub fn open(st: &mut AppState, folder: Option<PathBuf>) -> ImportDlg {
        match folder {
            Some(f) => st.start_import_scan(ImportSource::Folder, Some(f)),
            None => st.start_import_scan(ImportSource::Installed, None),
        }
        ImportDlg {
            rows: vec![],
            seen: HashSet::new(),
            synced: 0,
            hide_imported: true,
            browse: None,
            folder: None,
            detecting: None,
            protons: None,
            error: None,
        }
    }

    fn push(&mut self, c: ImportCandidate) {
        if self.seen.insert(c.path.clone()) {
            self.rows.push(Row { name: c.name.clone(), checked: c.existing.is_none(), c });
        }
    }

    /// Merge new scan results (the scan replaces its list when a new scan starts).
    fn sync(&mut self, st: &AppState) {
        let cands = &st.import_state.candidates;
        if cands.len() < self.synced {
            self.synced = 0;
        }
        if cands.len() == self.synced {
            return;
        }
        let new: Vec<ImportCandidate> = cands[self.synced..].to_vec();
        self.synced = cands.len();
        for c in new {
            self.push(c);
        }
    }

    fn visible(&self, i: usize) -> bool {
        !(self.hide_imported && self.rows[i].c.existing.is_some())
    }

    fn checked(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().enumerate().filter(|(i, r)| r.checked && self.visible(*i)).map(|(_, r)| r)
    }
}

fn import_dialog(
    ctx: &egui::Context,
    st: &mut AppState,
    d: &mut ImportDlg,
    pal: &Pal,
    fam: &egui::FontFamily,
) -> ImportOutcome {
    // Normally done every frame by the app; harmless to repeat, and the checklist must stay
    // live while it is open.
    st.poll_addgame();
    d.sync(st);
    poll_import_jobs(ctx, st, d);
    let needs_proton = d.checked().any(|r| r.c.launch.is_none() && r.c.kind == ProgramKind::Windows);
    if needs_proton && d.protons.is_none() {
        d.protons = Some(Protons::start(ctx, st));
    }
    if let Some(p) = &mut d.protons {
        p.poll();
    }

    let screen = ctx.content_rect();
    let width = (screen.width() - 64.0).clamp(600.0, 980.0);
    let table_h = (screen.height() - 300.0).clamp(160.0, 520.0);
    let home = st.paths.home.clone();
    let mut out = ImportOutcome::Keep;
    let (_, close) = modal(ctx, "ls_import_dlg", pal, width, |ui| {
        title(ui, "Scan for games", Some("Tick the games to add. Names can be changed here or later."), pal, fam);
        import_status(ui, st, d, pal, &home);
        ui.add_space(4.0);
        import_table(ui, d, pal, &home, table_h);
        if let Some(e) = &d.error {
            ui.label(RichText::new(e).size(12.0).color(pal.danger));
        }
        ui.add_space(10.0);
        let proton_ok = find_buttons(ui, st, d, pal, needs_proton);
        // The finishing buttons on a row of their own (the row above is for finding games).
        let n_checked = d.checked().count();
        let label = if n_checked == 0 {
            "Add games".to_string()
        } else {
            format!("Add {n_checked} game{}", if n_checked == 1 { "" } else { "s" })
        };
        let buttons = Buttons::primary(&label, n_checked > 0 && proton_ok)
            .cancel("Cancel")
            .disabled_hint(if n_checked == 0 { "Tick the games to add" } else { "Windows programs need Proton" });
        let f = footer(ui, pal, buttons, |_| {});
        if f.primary {
            out = ImportOutcome::Added(add_checked(st, d));
        }
        if f.cancel {
            out = ImportOutcome::Close;
        }
    });
    if matches!(out, ImportOutcome::Close) || (close && matches!(out, ImportOutcome::Keep)) {
        let picking = d.browse.is_some() || d.folder.is_some();
        if picking && !matches!(out, ImportOutcome::Close) {
            return ImportOutcome::Keep;
        }
        st.cancel_import_scan();
        return ImportOutcome::Close;
    }
    if matches!(out, ImportOutcome::Added(_)) {
        st.cancel_import_scan();
    }
    out
}

/// The file / folder pickers' answers, and the programs picked by Browse… once looked at.
fn poll_import_jobs(ctx: &egui::Context, st: &mut AppState, d: &mut ImportDlg) {
    if let Some(p) = &d.browse
        && let Some(files) = p.poll()
    {
        d.browse = None;
        if !files.is_empty() {
            d.detecting = Some(background(ctx, "detect-programs", move || {
                files
                    .into_iter()
                    .map(|f| {
                        let r = crate::addgame::detect_program(&f).map_err(|e| format!("{e:#}"));
                        (f, r)
                    })
                    .collect()
            }));
        }
    }
    if let Some(rx) = &d.detecting
        && let Ok(found) = rx.try_recv()
    {
        d.detecting = None;
        let mut errors = vec![];
        for (path, r) in found {
            match r {
                Ok(info) => {
                    let existing =
                        crate::addgame::find_duplicates(&st.library, &info.name, info.launch.as_ref(), Some(&info.exe))
                            .first()
                            .map(|m| m.id);
                    d.push(ImportCandidate {
                        name: info.name,
                        path: info.exe,
                        kind: info.kind,
                        launch: info.launch,
                        icon: info.icon,
                        install_dir: info.install_dir,
                        existing,
                        origin: "Picked".into(),
                    });
                }
                Err(e) => errors.push(format!("{}: {e}", path.file_name().unwrap_or_default().to_string_lossy())),
            }
        }
        d.error = (!errors.is_empty()).then(|| errors.join("\n"));
    }
    if let Some(p) = &d.folder
        && let Some(v) = p.poll()
    {
        d.folder = None;
        if let Some(f) = v.into_iter().next() {
            st.start_import_scan(ImportSource::Folder, Some(f));
        }
    }
}

/// What the scan is doing (or found), and "Hide games already in the library".
fn import_status(ui: &mut Ui, st: &mut AppState, d: &mut ImportDlg, pal: &Pal, home: &Path) {
    let is = crate::addgame::ImportState { candidates: vec![], ..st.import_state.clone() };
    let hidden = d.rows.iter().filter(|r| r.c.existing.is_some()).count();
    ui.horizontal(|ui| {
        ui.set_min_height(24.0);
        // Right part first, so the status text can truncate in what is left.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let label = if hidden > 0 {
                format!("Hide games already in the library ({hidden})")
            } else {
                "Hide games already in the library".to_string()
            };
            ui.checkbox(&mut d.hide_imported, label);
            ui.add_space(12.0);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let status = |ui: &mut Ui, text: String, color| {
                    ui.add(egui::Label::new(RichText::new(text).size(12.5).color(color)).truncate());
                };
                if is.running || d.detecting.is_some() {
                    lazy_spinner(ui, 14.0, pal.accent);
                    if is.running && ui.small_button("Stop").clicked() {
                        st.cancel_import_scan();
                    }
                    let what = match (&is.source, &is.folder) {
                        (_, Some(f)) if is.running => format!("Scanning {}", short_path(f, home, 50)),
                        (Some(ImportSource::Installed), _) if is.running => "Looking for installed games".to_string(),
                        _ => "Looking at the picked files".to_string(),
                    };
                    let files = if is.running && is.scanned_files > 0 {
                        format!(" · {} files", is.scanned_files)
                    } else {
                        String::new()
                    };
                    status(ui, format!("{what}…{files}"), pal.dim);
                } else if let Some(e) = &is.error {
                    status(ui, e.clone(), pal.danger);
                } else {
                    let n = d.rows.len() - hidden;
                    let text = match n {
                        0 if hidden > 0 => "No new games found".to_string(),
                        0 => String::new(),
                        n => format!("{n} game{} found", if n == 1 { "" } else { "s" }),
                    };
                    status(ui, text, pal.dim);
                }
            });
        });
    });
}

/// Browse…, Scan folder…, Detect installed, and the Proton picker when a ticked Windows
/// program needs one. Returns whether a Proton is chosen (or none is needed).
fn find_buttons(ui: &mut Ui, st: &mut AppState, d: &mut ImportDlg, pal: &Pal, needs_proton: bool) -> bool {
    let mut proton_ok = true;
    ui.horizontal(|ui| {
        let busy = d.browse.is_some() || d.folder.is_some();
        if ui
            .add_enabled(!busy, egui::Button::new("Browse…").min_size(vec2(0.0, 30.0)))
            .on_hover_text("Pick program files to add")
            .clicked()
        {
            d.browse = Some(Pick::start(ui.ctx(), PickKind::Files, "Choose games to add", None));
        }
        if ui
            .add_enabled(!busy, egui::Button::new("Scan folder…").min_size(vec2(0.0, 30.0)))
            .on_hover_text(
                "Find programs, AppImages and .exe files below a folder (installers, \
                 uninstallers and redistributables are skipped)",
            )
            .clicked()
        {
            d.folder = Some(Pick::start(ui.ctx(), PickKind::Folder, "Choose a folder with games", None));
        }
        if ui
            .add_enabled(!st.import_state.running, egui::Button::new("Detect installed").min_size(vec2(0.0, 30.0)))
            .on_hover_text(
                "Windows programs in Wine prefixes, .desktop games and Flatpak games that \
                 aren't in the library yet",
            )
            .clicked()
        {
            st.start_import_scan(ImportSource::Installed, None);
        }
        if needs_proton && let Some(p) = &mut d.protons {
            ui.add_space(8.0);
            ui.label(RichText::new("Proton").size(13.0).color(pal.dim));
            proton_ok = p.ui(ui, "ls_import_proton", 170.0, pal);
        }
    });
    proton_ok
}

/// Add every ticked game; returns the new ids.
fn add_checked(st: &mut AppState, d: &ImportDlg) -> Vec<GameId> {
    let proton = d.protons.as_ref().and_then(|p| p.selected()).cloned();
    let games: Vec<NewGame> = d
        .checked()
        .filter_map(|r| {
            let name = r.name.trim();
            let name = if name.is_empty() { r.c.name.trim() } else { name };
            let launch = match (&r.c.launch, r.c.kind) {
                (Some(l), _) => Some(l.clone()),
                (None, ProgramKind::Windows) => proton.as_ref().map(|p| {
                    let prefix = crate::addgame::default_prefix(&st.paths, name);
                    crate::addgame::windows_launch(&r.c.path, p, &prefix)
                }),
                _ => None,
            }?;
            Some(NewGame {
                name: name.to_string(),
                launch: Some(launch),
                art: Art { icon: r.c.icon.clone(), ..Default::default() },
                install_dir: r.c.install_dir.clone(),
                platform: Some(platform_for(r.c.kind).into()),
                edits: UserEdits::default(),
            })
        })
        .collect();
    let wanted = games.len();
    let ids: Vec<GameId> = games.into_iter().filter_map(|g| st.add_game(g)).collect();
    if ids.len() < wanted {
        st.notify_error(format!("Added {} of {wanted} games; the others couldn't be added", ids.len()));
    } else if !ids.is_empty() {
        st.notify(format!("Added {} game{}", ids.len(), if ids.len() == 1 { "" } else { "s" }));
    }
    ids
}

const ROW_H: f32 = 30.0;

fn import_table(ui: &mut Ui, d: &mut ImportDlg, pal: &Pal, home: &Path, height: f32) {
    let w = ui.available_width();
    let chk_w = 28.0;
    let name_w = (w * 0.3).clamp(170.0, 300.0);
    let kind_w = 118.0;
    let origin_w = (w * 0.16).clamp(100.0, 170.0);
    let path_w = (w - chk_w - name_w - kind_w - origin_w - 4.0 * 8.0).max(80.0);
    let vis: Vec<usize> = (0..d.rows.len()).filter(|&i| d.visible(i)).collect();
    egui::Frame::new()
        .fill(mix(pal.bg, pal.surface, 0.35))
        .stroke(egui::Stroke::new(1.0, with_alpha(pal.border, 150)))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.set_width(w);
            // header
            ui.horizontal(|ui| {
                ui.set_height(28.0);
                ui.spacing_mut().item_spacing.x = 8.0;
                ui.add_space(6.0);
                let all = !vis.is_empty() && vis.iter().all(|&i| d.rows[i].checked);
                let mut a = all;
                let r = ui.add_sized([chk_w - 6.0, 20.0], egui::Checkbox::without_text(&mut a));
                if r.changed() {
                    for &i in &vis {
                        d.rows[i].checked = a;
                    }
                }
                for (t, cw) in [("Name", name_w), ("Type", kind_w), ("Path", path_w), ("Found in", origin_w)] {
                    let (rr, _) = ui.allocate_exact_size(vec2(cw, 20.0), Sense::hover());
                    ui.painter().text(
                        pos2(rr.left() + 2.0, rr.center().y),
                        egui::Align2::LEFT_CENTER,
                        t,
                        egui::FontId::proportional(12.0),
                        pal.dim,
                    );
                }
            });
            let y = ui.cursor().top();
            ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1.0, with_alpha(pal.border, 150)));
            let body_h = height - 30.0;
            if vis.is_empty() {
                let (r, _) = ui.allocate_exact_size(vec2(w, body_h), Sense::hover());
                let text = if d.rows.is_empty() {
                    "Nothing found yet. Scan a folder with games, or Browse… for program files."
                } else {
                    "Everything found is already in your library."
                };
                ui.painter().text(
                    r.center(),
                    egui::Align2::CENTER_CENTER,
                    text,
                    egui::FontId::proportional(13.5),
                    pal.dim,
                );
                return;
            }
            egui::ScrollArea::vertical()
                .id_salt("ls_import_rows")
                .max_height(body_h)
                .min_scrolled_height(body_h)
                .auto_shrink([false, false])
                .show_rows(ui, ROW_H, vis.len(), |ui, range| {
                    for vi in range {
                        let i = vis[vi];
                        let row = &mut d.rows[i];
                        let in_lib = row.c.existing.is_some();
                        let (rect, _) = ui.allocate_exact_size(vec2(w, ROW_H), Sense::hover());
                        if vi % 2 == 1 {
                            ui.painter().rect_filled(rect, 0.0, with_alpha(pal.surface_alt, 60));
                        }
                        let mut child = ui.new_child(
                            egui::UiBuilder::new()
                                .max_rect(rect.shrink2(vec2(6.0, 0.0)))
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        let ui = &mut child;
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.add_sized([chk_w - 6.0, 20.0], egui::Checkbox::without_text(&mut row.checked));
                        let col = if in_lib { pal.dim } else { pal.text };
                        ui.add(
                            egui::TextEdit::singleline(&mut row.name)
                                .desired_width(name_w - 4.0)
                                .text_color(col)
                                .margin(egui::Margin::symmetric(6, 3)),
                        );
                        let small = |ui: &mut Ui, text: String, cw: f32, mono: bool| {
                            let mut t = RichText::new(text).size(12.0).color(pal.dim);
                            if mono {
                                t = t.monospace();
                            }
                            ui.allocate_ui_with_layout(
                                vec2(cw, 20.0),
                                egui::Layout::left_to_right(egui::Align::Center),
                                |ui| {
                                    ui.set_width(cw);
                                    ui.add(egui::Label::new(t).truncate())
                                },
                            )
                            .inner
                        };
                        small(ui, kind_label(row.c.kind).into(), kind_w, false);
                        small(ui, short_path(&row.c.path, home, (path_w / 7.5) as usize), path_w, true)
                            .on_hover_text(row.c.path.display().to_string());
                        let origin = if in_lib {
                            "In library".to_string()
                        } else if row.c.origin.starts_with('/') {
                            short_path(Path::new(&row.c.origin), home, (origin_w / 7.0) as usize)
                        } else {
                            row.c.origin.clone()
                        };
                        small(ui, origin, origin_w, false).on_hover_text(&row.c.origin);
                    }
                });
        });
}

// ------------------------------------------------------------------------------- dispatch

impl DesktopUi {
    /// Pick a program / Scan for games dialogs (called from `dialogs`).
    pub(super) fn add_dialogs(&mut self, ctx: &egui::Context, st: &mut AppState) {
        let pal = self.pal;
        let fam = self.fonts.bold.clone();
        if let Some(mut d) = self.dlg.program.take() {
            match program_dialog(ctx, st, &mut d, &pal, &fam) {
                ProgramOutcome::Keep => self.dlg.program = Some(d),
                ProgramOutcome::Close => {}
                ProgramOutcome::Added(id) | ProgramOutcome::Show(id) => self.select_new(id),
                ProgramOutcome::EditDetails(g) => {
                    self.dlg.edit = Some(super::edit::EditDlg::new_game(st, *g));
                }
            }
        }
        if let Some(mut d) = self.dlg.cloud.take() {
            use super::add_cloud::{CloudOutcome, cloud_dialog};
            match cloud_dialog(ctx, st, &mut d, &pal, &fam) {
                CloudOutcome::Keep => self.dlg.cloud = Some(d),
                CloudOutcome::Close => {}
                CloudOutcome::Added(id) | CloudOutcome::Show(id) => self.select_new(id),
            }
        }
        if let Some(mut d) = self.dlg.import.take() {
            match import_dialog(ctx, st, &mut d, &pal, &fam) {
                ImportOutcome::Keep => self.dlg.import = Some(d),
                ImportOutcome::Close => {}
                ImportOutcome::Added(ids) => {
                    if let Some(&id) = ids.last() {
                        self.select_new(id);
                    }
                }
            }
        }
    }

    /// Select a game that may only appear in the view next frame (just added / restored).
    pub(super) fn select_new(&mut self, id: GameId) {
        self.selected = Some(id);
        self.multi.clear();
        self.scroll_to_sel = true;
        // Recompute the view (and the selection's position) next frame.
        self.view_key = None;
    }

    /// "Same game?" banner for a hand-added game a scan found too (non-blocking).
    pub(super) fn merge_banner(&mut self, ctx: &egui::Context, st: &mut AppState) {
        if self.dlg.any_open() {
            return;
        }
        let Some(s) = st.merge_suggestions.first().cloned() else {
            return;
        };
        let (Some(manual), Some(scanned)) = (st.library.get(s.manual_id), st.library.get(s.scanned_id)) else {
            st.dismiss_merge(s.manual_id);
            return;
        };
        let manual_name = manual.display_name().to_string();
        let src = scanned.source.display();
        let more = st.merge_suggestions.len() - 1;
        let pal = self.pal;
        let mut merge = false;
        let mut dismiss = false;
        egui::Area::new(egui::Id::unique("ls_merge_banner"))
            .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -40.0])
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(pal.surface_alt)
                    .stroke(egui::Stroke::new(1.0, pal.border))
                    .corner_radius(8)
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .shadow(ui.visuals().popup_shadow)
                    .show(ui, |ui| {
                        ui.set_max_width(360.0);
                        ui.label(RichText::new("Same game?").size(14.0).strong().color(pal.text));
                        ui.label(
                            RichText::new(format!(
                                "“{manual_name}”, which you added by hand, was also found in \
                                 {src}."
                            ))
                            .size(12.5)
                            .color(pal.text),
                        );
                        hint_text(
                            ui,
                            "Merging keeps your play time, tags, edits and artwork on the \
                             scanned game and removes the hand-added copy.",
                            &pal,
                        );
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui
                                .add(
                                    egui::Button::new(RichText::new("Merge").color(pal.accent_text).strong())
                                        .fill(pal.accent),
                                )
                                .clicked()
                            {
                                merge = true;
                            }
                            if ui.button("Not the same").clicked() {
                                dismiss = true;
                            }
                            if more > 0 {
                                ui.label(RichText::new(format!("+{more} more")).size(12.0).color(pal.dim));
                            }
                        });
                    });
            });
        if merge {
            st.merge_games(s.manual_id, s.scanned_id);
            if self.selected == Some(s.manual_id) {
                self.select_new(s.scanned_id);
            }
        } else if dismiss {
            st.dismiss_merge(s.manual_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_paths() {
        let home = Path::new("/home/me");
        assert_eq!(short_path(Path::new("/home/me/Games/x"), home, 40), "~/Games/x");
        assert_eq!(
            short_path(Path::new("/home/me/a/very/long/folder/Hades/x64/Hades.exe"), home, 24),
            "…/Hades/x64/Hades.exe"
        );
        assert_eq!(short_path(Path::new("/mnt/x/Hades.exe"), home, 5), "…/Hades.exe", "the file name always stays");
    }
}
