//! The theme creator: make and edit controller-mode themes visually, with the real renderer and
//! the user's own library in the preview. Opened from Add-ons → Controller themes ("Create
//! theme…", "Edit") and the app menu; it has its own window like Settings (`Win::Creator`).
//!
//! * Start: a new theme from a template (any built-in or installed theme, copied into a new
//!   theme folder; built-ins are never changed), one of the user's themes, or a shared .zip.
//! * Layout: elements panel (drawing order, drag to reorder, add from the palette), the live
//!   preview (drag / resize with snapping, outlines, a screen shape and size, "Try it" to move
//!   through it with the arrow keys), and the inspector for the selected element (every
//!   setting, values like {game.name} from a picker with their live value, the element's TOML).
//!   Changes go to the theme itself or, picked above the preview, to one variant, screen shape
//!   or console only (`ops::Layer`).
//! * Theme pages: details for sharing, colours and colour schemes, fonts, console pictures,
//!   sounds and music, variants / screen shapes / console looks, a check in plain words, and
//!   the whole theme.toml.
//! * Undo / redo (Ctrl+Z / Ctrl+Shift+Z), saved automatically a moment after each change
//!   (atomic writes on a helper thread), test in controller mode, export / import .zip.
//!
//! Nothing here blocks the UI thread: file pickers, copies, zips and saves run on helpers
//! (`Job`), and the preview draws only when something changes.

mod bindings;
mod doc;
mod files;
mod geom;
mod inspector;
mod ops;
mod pages;
mod panel;
mod preview;
mod start;
mod validate;
mod widgets;
mod zip;

#[cfg(test)]
mod tests;

use super::Pal;
use super::add::Pick;
use super::dialogs::{hint_text, modal, plain_button, primary_button};
use crate::app::AppState;
use crate::theme::ViewKind;
use crossbeam_channel::Receiver;
use doc::Doc;
use egui::{Key, Modifiers, RichText, Ui, vec2};
use ops::{ElPath, Layer};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub(crate) const MODAL_ID: &str = "ls_theme_creator";

/// The theme pages.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    Layout,
    Details,
    Colours,
    Fonts,
    Consoles,
    Sounds,
    Choices,
    Check,
    File,
}

impl Page {
    const ALL: [Page; 9] = [
        Page::Layout,
        Page::Details,
        Page::Colours,
        Page::Fonts,
        Page::Consoles,
        Page::Sounds,
        Page::Choices,
        Page::Check,
        Page::File,
    ];

    fn label(self) -> &'static str {
        match self {
            Page::Layout => "Layout",
            Page::Details => "Details",
            Page::Colours => "Colours",
            Page::Fonts => "Fonts",
            Page::Consoles => "Console pictures",
            Page::Sounds => "Sounds & music",
            Page::Choices => "Variants & shapes",
            Page::Check => "Check",
            Page::File => "Theme file",
        }
    }

    fn from_key(k: &str) -> Option<Page> {
        Page::ALL.into_iter().find(|p| p.label().to_lowercase().starts_with(&k.to_lowercase()))
    }
}

/// What finished on a helper thread.
pub(crate) enum Done {
    /// A new theme folder (key).
    Created(Result<String, String>),
    /// A file brought into the theme for `for_`.
    Brought(AssetFor, Result<String, String>),
    Saved(u64, Box<crate::theme::ThemeFile>, Result<(), String>),
    Exported(PathBuf, Result<usize, String>),
    Imported(Result<zip::Imported, String>),
    /// Console pictures added from a folder: (set, matched (file name, theme path), unmatched files).
    ConsolePictures(String, Vec<(String, String)>, Vec<PathBuf>),
    /// A preview picture was written (theme path).
    Captured(Result<String, String>),
    /// A save-as path was picked (None: cancelled).
    SavePath(Option<PathBuf>),
    /// One of the user's themes was deleted (its name).
    Deleted(String, Result<(), String>),
    /// Something on a helper thread failed (plain words).
    Failed(String),
}

/// Where a brought-in file goes once it is in the theme folder.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum AssetFor {
    /// An element's picture / video / selector … (`inspector::Slot`).
    Element(ViewKind, ElPath, inspector::Slot),
    /// A font name in `[fonts]`.
    Font(String),
    Sound(String),
    Music,
    /// The screen's background (`panel::set_background`).
    Background(ViewKind, bool),
}

pub(crate) struct Job {
    rx: Receiver<Done>,
    pub label: String,
}

/// An open file picker and what it is for.
pub(crate) enum PickFor {
    Asset(files::Asset, AssetFor),
    ImportZip,
    ConsoleFolder(String),
    ConsoleOne(String, String),
}

/// Something to say above the page (error: red).
#[derive(Clone)]
pub(crate) struct Banner {
    pub text: String,
    pub error: bool,
}

/// The values picker: which field it fills.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BindTarget {
    pub id: egui::Id,
    pub kinds: Vec<bindings::Use>,
}

pub(crate) struct CreatorDlg {
    pub doc: Option<Doc>,
    pub page: Page,
    pub view: ViewKind,
    pub sel: Option<ElPath>,
    pub layer: Layer,
    pub pv: preview::Preview,
    pub start: start::Start,
    jobs: Vec<Job>,
    pick: Option<(PickFor, Pick)>,
    pub banner: Option<Banner>,
    /// Last change not saved yet: when it was made.
    changed_at: Option<Instant>,
    saving: bool,
    save_error: Option<String>,
    pub bind: Option<BindTarget>,
    /// A value chosen in the picker, for the field `id` to take.
    pub bind_pick: Option<(egui::Id, String)>,
    pub export: Option<pages::ExportDlg>,
    pub raw: inspector::RawPane,
    pub file_text: Option<(u64, String, Option<String>)>,
    pub consoles: pages::ConsolesPage,
    /// Testing hooks (`theme-creator:…`): open the values picker / export on the first frame.
    debug_bind: bool,
    /// Asked: test this theme in controller mode.
    test: bool,
    /// Save right after each change (screenshot runs).
    quick_save: bool,
    close: bool,
}

/// What the window asks of the desktop after a frame.
pub(crate) enum CreatorOutcome {
    Keep,
    Close,
    /// Use this theme in controller mode and switch to it (the creator stays open).
    Test(String),
}

impl CreatorDlg {
    pub fn new() -> CreatorDlg {
        CreatorDlg {
            doc: None,
            page: Page::Layout,
            view: ViewKind::Games,
            sel: None,
            layer: Layer::Base,
            pv: preview::Preview::default(),
            start: start::Start::default(),
            jobs: vec![],
            pick: None,
            banner: None,
            changed_at: None,
            saving: false,
            save_error: None,
            bind: None,
            bind_pick: None,
            export: None,
            raw: Default::default(),
            file_text: None,
            consoles: Default::default(),
            debug_bind: false,
            test: false,
            quick_save: false,
            close: false,
        }
    }

    /// Open a user theme for editing.
    pub fn open(st: &AppState, key: &str) -> CreatorDlg {
        let mut d = CreatorDlg::new();
        d.open_key(st, key);
        if d.doc.as_ref().is_some_and(|doc| doc.dir.join("playnite-options.toml").is_file()) {
            d.banner = Some(Banner {
                text: "This theme was imported from Playnite: changing its Playnite theme options later imports it again and undoes changes made here. Making a copy (◀ Themes → Edit a copy) keeps them.".into(),
                error: false,
            });
        }
        d
    }

    fn open_key(&mut self, st: &AppState, key: &str) {
        let dir = st.themes.themes_dir().join(key);
        match Doc::open(key, dir) {
            Ok(doc) => {
                self.view = if doc.file.home.is_some() && doc.file.games.elements.is_empty() {
                    ViewKind::Home
                } else {
                    ViewKind::Games
                };
                self.doc = Some(doc);
                self.sel = None;
                self.layer = Layer::Base;
                self.page = Page::Layout;
                self.pv = preview::Preview::default();
                self.banner = None;
            }
            Err(e) => self.banner = Some(Banner { text: format!("Couldn't open “{key}”: {e:#}"), error: true }),
        }
    }

    /// Testing hook: `theme-creator[:<theme>[:<option>…]]` (`LODESTAR_DESKTOP_OPEN`). A built-in
    /// theme is copied first ("My <name>"); options: a page name (layout, colours, consoles …),
    /// view=<collections|games|details|home>, select=<element name>, shape=<16:9|deck|4:3|21:9>,
    /// outlines, try, bindings (the values picker for the selected element), export (the export
    /// window), layer=<variant:name|shape:16:10|console:key>, scheme=<name>.
    pub fn debug(st: &mut AppState, spec: &str) -> CreatorDlg {
        let mut d = CreatorDlg::new();
        let mut parts = spec.split(':');
        let theme = parts.next().unwrap_or("").trim();
        if !theme.is_empty() {
            let dir = st.themes.themes_dir().join(theme);
            let key = if dir.join("theme.toml").is_file() {
                theme.to_string()
            } else {
                let tpl = files::Template::Builtin(crate::theme::builtin::current_key(theme).to_string());
                match files::create(st.themes.themes_dir(), &tpl, &format!("My {theme}"), "") {
                    Ok(k) => k,
                    Err(e) => {
                        d.banner = Some(Banner { text: format!("{e:#}"), error: true });
                        return d;
                    }
                }
            };
            st.themes.refresh();
            d.open_key(st, &key);
        }
        let rest: Vec<&str> = parts.collect();
        let mut i = 0;
        while i < rest.len() {
            let o = rest[i];
            let (k, v) = o.split_once('=').unwrap_or((o, ""));
            match k {
                "view" => {
                    d.view = match v {
                        "collections" => ViewKind::Collections,
                        "details" => ViewKind::Details,
                        "home" => ViewKind::Home,
                        _ => ViewKind::Games,
                    }
                }
                "select" => {
                    if let Some(doc) = &d.doc
                        && let Some(view) = doc.file.view(d.view)
                    {
                        d.sel = view.elements.iter().position(|e| e.common().name == v).map(ElPath::top);
                    }
                }
                "shape" => d.pv.shape = preview::Shape::from_key(v),
                "outlines" => d.pv.outlines = true,
                "try" => d.pv.try_mode = true,
                "bindings" => d.debug_bind = true,
                "export" => d.export = Some(pages::ExportDlg::default()),
                "scheme" => d.pv.scheme = Some(v.to_string()),
                "layer" => {
                    // layer=shape:16:10 has a colon in it: take the rest.
                    let whole = rest[i..].join(":");
                    let val = whole.trim_start_matches("layer=");
                    d.layer = match val.split_once(':') {
                        Some(("variant", n)) => Layer::Variant(n.into()),
                        Some(("shape", n)) => Layer::Aspect(n.into()),
                        Some(("console", n)) => Layer::Console(n.into()),
                        _ => Layer::Base,
                    };
                    break;
                }
                p => {
                    if let Some(page) = Page::from_key(p) {
                        d.page = page;
                    }
                }
            }
            i += 1;
        }
        d
    }

    fn spawn(&mut self, ctx: &egui::Context, label: &str, f: impl FnOnce() -> Done + Send + 'static) {
        let rx = super::add::background(ctx, "theme-creator", f);
        self.jobs.push(Job { rx, label: label.to_string() });
    }

    pub fn pick(&mut self, ctx: &egui::Context, what: PickFor, title: &str) {
        use super::add::PickKind;
        let kind = match &what {
            PickFor::Asset(files::Asset::Picture, _) => PickKind::Image,
            PickFor::ConsoleOne(..) => PickKind::Image,
            PickFor::ImportZip => PickKind::Zip,
            PickFor::ConsoleFolder(_) => PickKind::Folder,
            PickFor::Asset(..) => PickKind::File,
        };
        self.pick = Some((what, Pick::start(ctx, kind, title, None)));
    }

    /// Bring `src` into the theme folder for `target` (helper thread).
    pub fn bring_in(&mut self, ctx: &egui::Context, src: PathBuf, kind: files::Asset, target: AssetFor) {
        let Some(dir) = self.doc.as_ref().map(|d| d.dir.clone()) else { return };
        self.spawn(ctx, "Adding the file to the theme…", move || {
            Done::Brought(target, files::bring_in(&dir, &src, kind).map_err(|e| format!("{e:#}")))
        });
    }

    fn poll(&mut self, ctx: &egui::Context, st: &mut AppState) {
        if let Some((_, pick)) = &self.pick
            && let Some(paths) = pick.poll()
        {
            let what = match self.pick.take() {
                Some((w, _)) => w,
                None => return,
            };
            if let Some(p) = paths.into_iter().next() {
                self.picked(ctx, st, what, p);
            }
        }
        let mut done = vec![];
        self.jobs.retain(|j| match j.rx.try_recv() {
            Ok(d) => {
                done.push(d);
                false
            }
            Err(crossbeam_channel::TryRecvError::Empty) => true,
            Err(_) => false,
        });
        for d in done {
            self.finished(ctx, st, d);
        }
    }

    fn picked(&mut self, ctx: &egui::Context, st: &mut AppState, what: PickFor, p: PathBuf) {
        match what {
            PickFor::Asset(kind, target) => self.bring_in(ctx, p, kind, target),
            PickFor::ImportZip => {
                let themes = st.themes.themes_dir().to_path_buf();
                self.spawn(ctx, "Adding the theme…", move || {
                    Done::Imported(zip::import(&p, &themes).map_err(|e| format!("{e:#}")))
                });
            }
            PickFor::ConsoleFolder(set) => {
                let Some(dir) = self.doc.as_ref().map(|d| d.dir.clone()) else { return };
                let colls: Vec<crate::library::Collection> = st.library.collections();
                self.spawn(ctx, "Adding console pictures…", move || {
                    let (ok, rest) = pages::import_console_folder(&dir, &set, &p, &colls);
                    Done::ConsolePictures(set, ok, rest)
                });
            }
            PickFor::ConsoleOne(slot, key) => self.pick_done_console(ctx, slot, key, p),
        }
    }

    /// Copy a picture for one console's `slot` into the theme (helper thread).
    pub fn pick_done_console(&mut self, ctx: &egui::Context, slot: String, key: String, p: PathBuf) {
        let Some(dir) = self.doc.as_ref().map(|d| d.dir.clone()) else { return };
        self.spawn(ctx, "Adding the picture…", move || match files::copy_console_picture(&dir, &slot, &key, &p) {
            Ok(rel) => Done::ConsolePictures(slot, vec![(key, rel)], vec![]),
            Err(e) => Done::Failed(format!("{e:#}")),
        });
    }

    fn finished(&mut self, ctx: &egui::Context, st: &mut AppState, d: Done) {
        match d {
            Done::Created(Ok(key)) => {
                st.themes.refresh();
                self.open_key(st, &key);
            }
            Done::Created(Err(e)) => {
                self.banner = Some(Banner { text: format!("Couldn't make the theme: {e}"), error: true })
            }
            Done::Brought(_, Err(e)) => self.banner = Some(Banner { text: e, error: true }),
            Done::Brought(target, Ok(rel)) => self.place_asset(ctx, st, target, rel),
            Done::Saved(rev, file, r) => {
                self.saving = false;
                match r {
                    Ok(()) => {
                        self.save_error = None;
                        if let Some(doc) = &mut self.doc {
                            doc.mark_saved(*file);
                            let _ = rev;
                        }
                        st.themes.refresh();
                        self.reload_in_use(st);
                    }
                    Err(e) => self.save_error = Some(e),
                }
            }
            Done::Exported(path, r) => match r {
                Ok(n) => {
                    self.banner =
                        Some(Banner { text: format!("Exported {n} files to {}", path.display()), error: false });
                    if let Some(e) = &mut self.export {
                        e.done = Some(path);
                    }
                }
                Err(e) => self.banner = Some(Banner { text: format!("Couldn't export: {e}"), error: true }),
            },
            Done::SavePath(Some(path)) => {
                if let Some(doc) = &self.doc {
                    let dir = doc.dir.clone();
                    let top = doc.key.clone();
                    self.spawn(ctx, "Exporting…", move || {
                        Done::Exported(path.clone(), zip::export(&dir, &top, &path).map_err(|e| format!("{e:#}")))
                    });
                }
            }
            Done::SavePath(None) => {}
            Done::Imported(Ok(i)) => {
                st.themes.refresh();
                let skipped =
                    if i.skipped > 0 { format!(" ({} very large files left out)", i.skipped) } else { String::new() };
                self.open_key(st, &i.key);
                self.banner = Some(Banner { text: format!("Added “{}”{skipped}.", i.name), error: false });
            }
            Done::Imported(Err(e)) => {
                self.banner = Some(Banner { text: format!("Couldn't add the theme: {e}"), error: true })
            }
            Done::ConsolePictures(slot, ok, rest) => {
                if let Some(doc) = &mut self.doc {
                    for (_, rel) in &ok {
                        st.images.invalidate(&doc.dir.join(rel));
                    }
                    if !ok.is_empty() {
                        doc.change("Add console pictures", |f| {
                            for (key, rel) in &ok {
                                pages::set_art(f, key, &slot, Some(rel.clone()));
                            }
                        });
                    }
                }
                self.pv.forget_files();
                if ok.len() != 1 || !rest.is_empty() {
                    self.consoles.after_import(ok.len(), rest);
                }
            }
            Done::Deleted(name, r) => {
                self.start.refresh = true;
                if let Some(files::Template::Folder(dir)) = &self.start.template
                    && !dir.exists()
                {
                    self.start.template = None;
                }
                self.banner = Some(match r {
                    Ok(()) => Banner { text: format!("Deleted “{name}”."), error: false },
                    Err(e) => Banner { text: format!("Couldn't delete “{name}”: {e}"), error: true },
                });
            }
            Done::Failed(e) => self.banner = Some(Banner { text: e, error: true }),
            Done::Captured(Ok(rel)) => {
                if let Some(doc) = &mut self.doc {
                    st.images.invalidate(&doc.dir.join(&rel));
                    doc.change("Add a preview picture", |f| {
                        if !f.theme.previews.contains(&rel) {
                            f.theme.previews.push(rel.clone());
                        }
                        f.theme.previews.truncate(4);
                    });
                }
            }
            Done::Captured(Err(e)) => {
                self.banner = Some(Banner { text: format!("Couldn't save the picture: {e}"), error: true })
            }
        }
    }

    fn place_asset(&mut self, ctx: &egui::Context, st: &mut AppState, target: AssetFor, rel: String) {
        if let Some(doc) = &self.doc {
            st.images.invalidate(&doc.dir.join(&rel));
        }
        self.pv.forget_files();
        match target {
            AssetFor::Element(view, path, slot) => {
                let Some(doc) = self.doc.as_mut() else { return };
                edit_element(doc, ctx, &self.layer, view, &path, &mut self.banner, |e| {
                    inspector::set_slot(e, slot, rel)
                });
            }
            AssetFor::Font(name) => {
                if let Some(doc) = &mut self.doc {
                    doc.change("Add a font", |f| pages::set_font(&mut f.fonts, &name, Some(rel)));
                }
            }
            AssetFor::Sound(k) => {
                if let Some(doc) = &mut self.doc {
                    doc.change("Set a sound", |f| {
                        f.sounds.insert(k, rel);
                    });
                }
            }
            AssetFor::Music => {
                if let Some(doc) = &mut self.doc {
                    doc.change("Add music", |f| f.music.push(rel));
                }
            }
            AssetFor::Background(view, video) => {
                if let Some(doc) = &mut self.doc {
                    let mut sel = None;
                    doc.change("Set the background", |f| sel = panel::set_background(f, view, Some(rel), video));
                    if sel.is_some() {
                        self.sel = sel;
                    }
                }
            }
        }
    }

    /// The theme in use in controller mode / on the desktop is this one: show the saved changes.
    fn reload_in_use(&self, st: &mut AppState) {
        let Some(doc) = &self.doc else { return };
        if st.controller_theme.key == doc.key {
            let (t, _) = st.load_controller_theme(&doc.key);
            st.controller_theme = t;
        }
    }

    /// Save a moment after the last change (helper thread), or now (`force`).
    fn autosave(&mut self, ctx: &egui::Context, force: bool) {
        let Some(doc) = &self.doc else { return };
        if !doc.dirty() {
            self.changed_at = None;
            return;
        }
        let at = *self.changed_at.get_or_insert_with(Instant::now);
        // Screenshot runs end soon after their last key: save at once there.
        let wait = Duration::from_millis(if self.quick_save { 0 } else { 900 });
        if !force && at.elapsed() < wait {
            ctx.request_repaint_after(wait - at.elapsed());
            return;
        }
        if self.saving {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        }
        let mut file = doc.file.clone();
        file.theme.made_with = env!("CARGO_PKG_VERSION").to_string();
        let raw = doc.raw.clone();
        let dir = doc.dir.clone();
        let rev = doc.rev;
        let saved_as = doc.file.clone();
        self.saving = true;
        self.changed_at = None;
        self.spawn(ctx, "Saving…", move || {
            let r = file.to_theme_toml(Some(&raw)).and_then(|t| files::save(&dir, &t)).map_err(|e| format!("{e:#}"));
            Done::Saved(rev, Box::new(saved_as), r)
        });
    }

    /// Lodestar quits: write what isn't saved yet, now.
    pub fn save_now(&self) {
        let Some(doc) = &self.doc else { return };
        if doc.dirty()
            && let Ok(t) = doc.text()
            && let Err(e) = files::save(&doc.dir, &t)
        {
            log::warn!("saving theme {}: {e:#}", doc.key);
        }
    }

    fn undo(&mut self) {
        if let Some(d) = &mut self.doc
            && d.undo()
        {
            self.after_history();
        }
    }

    fn redo(&mut self) {
        if let Some(d) = &mut self.doc
            && d.redo()
        {
            self.after_history();
        }
    }

    fn after_history(&mut self) {
        // The selection may be gone.
        if let (Some(doc), Some(p)) = (&self.doc, &self.sel)
            && doc.file.view(self.view).is_none_or(|v| ops::get(&v.elements, p).is_none())
        {
            self.sel = None;
        }
        self.raw.reset();
    }
}

/// Apply `f` to element `p` of `view` in `layer` as an undo step (merged while one widget is
/// being typed in or dragged). Says so when the layer can't take a change.
pub(crate) fn edit_element(
    doc: &mut Doc,
    ctx: &egui::Context,
    layer: &Layer,
    view: ViewKind,
    p: &ElPath,
    banner: &mut Option<Banner>,
    f: impl FnOnce(&mut crate::theme::Element),
) -> bool {
    let tag = ctx
        .dragged_id()
        .or_else(|| ctx.memory(|m| m.focused()))
        .map(|id| format!("w{:?}{:?}", id, p))
        .unwrap_or_default();
    let mut lost = vec![];
    let label = "Change an element";
    let changed = if tag.is_empty() {
        doc.change(label, |file| {
            if let Ok(l) = ops::edit_in_layer(file, layer, view, p, f) {
                lost = l;
            }
        })
    } else {
        doc.change_merged(&tag, label, |file| {
            if let Ok(l) = ops::edit_in_layer(file, layer, view, p, f) {
                lost = l;
            }
        })
    };
    if !lost.is_empty() {
        *banner = Some(Banner {
            text: format!(
                "Only the theme itself can take away {} — switch “Changes go to” to “The whole theme” for that.",
                lost.join(", ")
            ),
            error: true,
        });
    }
    changed
}

/// The window (or the dialog inside the main window). `fam`: the heading font.
pub(crate) fn show(
    ctx: &egui::Context,
    st: &mut AppState,
    d: &mut CreatorDlg,
    pal: &Pal,
    fam: &egui::FontFamily,
) -> CreatorOutcome {
    d.poll(ctx, st);
    d.quick_save = st.no_launch;
    let own = super::own_window::in_own_window(ctx);
    let area = ctx.content_rect();
    let width = if own { area.width() - 42.0 } else { (area.width() - 110.0).max(700.0) };
    let height = if own { area.height() - 42.0 } else { (area.height() - 100.0).max(500.0) };
    keys(ctx, d);
    let (_, close) = modal(ctx, MODAL_ID, pal, width, |ui| {
        ui.set_min_height(height);
        ui.set_max_height(height);
        ui.set_width(width);
        if d.doc.is_none() {
            start::show(ui, st, d, pal, fam);
        } else {
            editor(ui, st, d, pal, fam, height);
        }
    });
    if let Some(e) = &mut d.export {
        let keep = pages::export_window(ctx, st, d.doc.as_ref(), e, pal);
        if let Some(req) = std::mem::take(&mut e.request) {
            pages::start_export(d, ctx, req);
        } else if !keep {
            d.export = None;
        }
    }
    bindings_window(ctx, st, d, pal);
    d.autosave(ctx, close || d.close || d.test);
    if d.test {
        d.test = false;
        if let Some(doc) = &d.doc {
            // The save was started just now: write the same text here too, so controller mode
            // loads this version (a few kB, atomic).
            if let Ok(t) = doc.text() {
                let _ = files::save(&doc.dir, &t);
            }
            return CreatorOutcome::Test(doc.key.clone());
        }
    }
    if close || std::mem::take(&mut d.close) {
        return CreatorOutcome::Close;
    }
    CreatorOutcome::Keep
}

/// Shortcuts: Ctrl+Z / Ctrl+Shift+Z (or Ctrl+Y) undo / redo, Ctrl+S saves now.
fn keys(ctx: &egui::Context, d: &mut CreatorDlg) {
    if d.doc.is_none() {
        return;
    }
    let typing = ctx.memory(|m| m.focused()).is_some() && ctx.egui_wants_keyboard_input();
    let (undo, redo, save) = ctx.input_mut(|i| {
        let redo =
            i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || i.consume_key(Modifiers::COMMAND, Key::Y);
        let undo = !redo && !typing && i.consume_key(Modifiers::COMMAND, Key::Z);
        let save = i.consume_key(Modifiers::COMMAND, Key::S);
        (undo, redo, save)
    });
    if undo {
        d.undo();
    }
    if redo {
        d.redo();
    }
    if save {
        d.changed_at = Some(Instant::now() - Duration::from_secs(5));
    }
}

fn editor(ui: &mut Ui, st: &mut AppState, d: &mut CreatorDlg, pal: &Pal, fam: &egui::FontFamily, height: f32) {
    top_bar(ui, st, d, pal, fam);
    if let Some(b) = d.banner.clone() {
        let color = if b.error { pal.danger } else { pal.accent };
        egui::Frame::new()
            .fill(crate::ui::widgets::with_alpha(color, 22))
            .stroke(egui::Stroke::new(1.0, crate::ui::widgets::with_alpha(color, 80)))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&b.text).size(12.5).color(pal.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").on_hover_text("Close this message").clicked() {
                            d.banner = None;
                        }
                    });
                });
            });
        ui.add_space(4.0);
    }
    let body_h = (height - (ui.cursor().min.y - ui.min_rect().min.y) - 8.0).max(300.0);
    match d.page {
        Page::Layout => layout_page(ui, st, d, pal, body_h),
        page => {
            // Pages that change the look keep a live preview beside them.
            let side = matches!(page, Page::Details | Page::Colours | Page::Fonts | Page::Consoles | Page::Choices)
                && ui.available_width() > 1100.0;
            let page_w = if side { (ui.available_width() * 0.55).min(760.0) } else { ui.available_width() };
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(page_w, body_h), egui::Layout::top_down(egui::Align::Min), |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt(("ls_creator_page", page.label()))
                        .max_height(body_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.set_max_width(900.0f32.min(page_w - 16.0));
                            pages::show(ui, st, d, pal, page);
                        });
                });
                if side {
                    ui.add_space(12.0);
                    ui.vertical(|ui| preview::mini(ui, st, d, pal));
                }
            });
        }
    }
    if d.debug_bind {
        d.debug_bind = false;
        d.bind = Some(BindTarget { id: egui::Id::unique("ls_creator_text_field"), kinds: vec![bindings::Use::Text] });
    }
}

fn top_bar(ui: &mut Ui, st: &mut AppState, d: &mut CreatorDlg, pal: &Pal, fam: &egui::FontFamily) {
    let Some(doc) = &d.doc else { return };
    let name = doc.file.theme.name.clone();
    let status = if let Some(e) = &d.save_error {
        (format!("Not saved: {e}"), pal.danger)
    } else if d.saving || doc.dirty() {
        ("Saving…".to_string(), pal.dim)
    } else {
        ("All changes saved".to_string(), pal.dim)
    };
    let (can_undo, can_redo) = (doc.undo_label().map(str::to_string), doc.redo_label().map(str::to_string));
    ui.horizontal(|ui| {
        if ui.button("◀ Themes").on_hover_text("Back to the list of themes").clicked() {
            d.autosave(ui.ctx(), true);
            d.doc = None;
            d.start.refresh = true;
            return;
        }
        ui.add_space(6.0);
        ui.label(RichText::new(&name).family(fam.clone()).size(18.0).strong().color(pal.text));
        ui.label(RichText::new(&status.0).size(12.0).color(status.1));
        ui.add_space(8.0);
        let u = ui.add_enabled(can_undo.is_some(), egui::Button::new("↶ Undo"));
        if u.on_hover_text(can_undo.as_deref().map_or("Nothing to undo".into(), |l| format!("Undo: {l} (Ctrl+Z)")))
            .clicked()
        {
            d.undo();
        }
        let r = ui.add_enabled(can_redo.is_some(), egui::Button::new("↷ Redo"));
        if r.on_hover_text(
            can_redo.as_deref().map_or("Nothing to redo".into(), |l| format!("Redo: {l} (Ctrl+Shift+Z)")),
        )
        .clicked()
        {
            d.redo();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if primary_button(ui, "Done", pal, true).clicked() {
                d.close = true;
            }
            if plain_button(ui, "Export…").on_hover_text("Save the theme as a .zip to share").clicked() {
                d.export = Some(pages::ExportDlg::default());
            }
            if ui
                .add(egui::Button::new("▶ Test in controller mode").min_size(vec2(80.0, 30.0)))
                .on_hover_text("Use this theme in controller mode and switch there (come back with the menu)")
                .clicked()
            {
                d.test = true;
            }
            let busy: Vec<String> = d.jobs.iter().filter(|j| j.label != "Saving…").map(|j| j.label.clone()).collect();
            if let Some(b) = busy.first() {
                crate::ui::widgets::lazy_spinner(ui, 14.0, pal.dim);
                hint_text(ui, b, pal);
            }
        });
    });
    let _ = st;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for p in Page::ALL {
            let mut label = RichText::new(p.label()).size(13.0);
            if p == Page::Check
                && let Some(doc) = &d.doc
            {
                let n = d.pv.findings(doc).iter().filter(|f| f.level != validate::Level::Note).count();
                if n > 0 {
                    label = RichText::new(format!("Check ({n})")).size(13.0);
                    if d.page != Page::Check {
                        label = label.color(pal.danger);
                    }
                }
            }
            if ui.add(egui::Button::selectable(d.page == p, label).min_size(vec2(0.0, 28.0))).clicked() {
                d.page = p;
            }
        }
    });
    ui.separator();
}

fn layout_page(ui: &mut Ui, st: &mut AppState, d: &mut CreatorDlg, pal: &Pal, body_h: f32) {
    let panel_w = 230.0;
    let insp_w = 360.0;
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(vec2(panel_w, body_h), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(panel_w);
            ui.set_min_height(body_h);
            panel::show(ui, st, d, pal, body_h);
        });
        ui.separator();
        let mid_w = (ui.available_width() - insp_w - 16.0).max(320.0);
        ui.allocate_ui_with_layout(vec2(mid_w, body_h), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(mid_w);
            preview::show(ui, st, d, pal, body_h);
        });
        ui.separator();
        ui.allocate_ui_with_layout(vec2(insp_w, body_h), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.set_width(insp_w);
            egui::ScrollArea::both()
                .id_salt("ls_creator_inspector")
                .max_height(body_h)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(insp_w - 14.0);
                    inspector::show(ui, st, d, pal);
                });
        });
    });
}

/// The values picker: every value a theme can show, grouped, with what it is for the game /
/// collection the preview shows now.
fn bindings_window(ctx: &egui::Context, st: &AppState, d: &mut CreatorDlg, pal: &Pal) {
    let Some(target) = d.bind.clone() else { return };
    let mut open = true;
    let mut chosen = None;
    let binder_vals = d.pv.binding_values(st, d.doc.as_ref(), d.view);
    let (_, close) = modal(ctx, "ls_creator_bindings", pal, 560.0, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Insert a value").size(17.0).strong().color(pal.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    open = false;
                }
            });
        });
        hint_text(
            ui,
            "These change with the game or collection that is selected. The value on the right is what the preview shows now.",
            pal,
        );
        ui.add_space(4.0);
        let q_id = egui::Id::unique("ls_creator_bind_q");
        let mut q: String = ui.data(|m| m.get_temp(q_id)).unwrap_or_default();
        if ui.add(egui::TextEdit::singleline(&mut q).hint_text("Search").desired_width(f32::INFINITY)).changed() {
            ui.data_mut(|m| m.insert_temp(q_id, q.clone()));
        }
        let ql = q.to_lowercase();
        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() * 0.62).clamp(300.0, 680.0))
            .min_scrolled_height((ctx.content_rect().height() * 0.62).clamp(300.0, 680.0))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for g in bindings::groups() {
                    let items: Vec<&bindings::Binding> = bindings::ALL
                        .iter()
                        .filter(|b| b.group == g)
                        .filter(|b| target.kinds.is_empty() || target.kinds.contains(&b.kind))
                        .filter(|b| ql.is_empty() || b.label.to_lowercase().contains(&ql) || b.key.contains(&ql))
                        .collect();
                    if items.is_empty() {
                        continue;
                    }
                    widgets::heading(ui, g, pal);
                    for b in items {
                        ui.horizontal(|ui| {
                            let r = ui
                                .add(egui::Button::new(RichText::new(b.label).size(12.5)).min_size(vec2(230.0, 22.0)));
                            ui.label(RichText::new(format!("{{{}}}", b.key)).monospace().size(11.0).color(pal.dim));
                            let v = binder_vals.get(b.key).cloned().unwrap_or_default();
                            let v = if v.is_empty() {
                                "—".to_string()
                            } else if b.kind == bindings::Use::Picture || b.key == "theme.dir" {
                                // Files by their name only.
                                std::path::Path::new(&v)
                                    .file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or(v)
                            } else {
                                v
                            };
                            ui.add(egui::Label::new(RichText::new(v).size(11.5).color(pal.text)).truncate());
                            if r.clicked() {
                                chosen = Some(format!("{{{}}}", b.key));
                            }
                        });
                    }
                }
            });
    });
    if close {
        open = false;
    }
    if let Some(c) = chosen {
        d.bind_pick = Some((target.id, c));
        d.bind = None;
    } else if !open {
        d.bind = None;
    }
}
