//! The live preview: the real renderer drawing the selected view with the user's library, at a
//! chosen screen shape and size; selecting, dragging and resizing elements on it (with snapping
//! and guides), or "Try it" to move through it like in controller mode.

use super::super::Pal;
use super::geom::{self, Guides, Handle, Snap};
use super::ops::{self, ElPath, Layer};
use super::validate::{self, Finding};
use super::{CreatorDlg, doc::Doc, widgets};
use crate::app::AppState;
use crate::library::{CollectionKind, Library};
use crate::theme::render::draw::Hint;
use crate::theme::render::{
    CollectionItem, FocusKind, Items, Prepared, RailScene, Renderer, RowScene, Scene, SceneExtra, ViewOpts,
    prepare_view_for, view_frame,
};
use crate::theme::{Element, Theme, ViewKind};
use egui::{Color32, Key, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2, pos2, vec2};
use std::collections::HashMap;
use std::sync::Arc;

/// Screen shapes to preview at.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shape {
    #[default]
    Wide,
    Deck,
    Classic,
    Ultra,
}

impl Shape {
    pub const ALL: [Shape; 4] = [Shape::Wide, Shape::Deck, Shape::Classic, Shape::Ultra];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Wide => "16:9 TV / monitor",
            Shape::Deck => "16:10 Steam Deck",
            Shape::Classic => "4:3",
            Shape::Ultra => "21:9 ultrawide",
        }
    }

    pub fn ratio(self) -> f32 {
        match self {
            Shape::Wide => 16.0 / 9.0,
            Shape::Deck => 16.0 / 10.0,
            Shape::Classic => 4.0 / 3.0,
            Shape::Ultra => 21.0 / 9.0,
        }
    }

    /// Screen sizes people have at this shape.
    pub fn sizes(self) -> &'static [[u32; 2]] {
        match self {
            Shape::Wide => &[[1280, 720], [1920, 1080], [2560, 1440], [3840, 2160]],
            Shape::Deck => &[[1280, 800], [1920, 1200], [2560, 1600]],
            Shape::Classic => &[[1024, 768], [1600, 1200]],
            Shape::Ultra => &[[2560, 1080], [3440, 1440]],
        }
    }

    pub fn from_key(k: &str) -> Shape {
        match k {
            "deck" | "16:10" => Shape::Deck,
            "4:3" => Shape::Classic,
            "21:9" => Shape::Ultra,
            _ => Shape::Wide,
        }
    }

    /// The `aspects` key for it.
    pub fn key(self) -> &'static str {
        match self {
            Shape::Wide => "16:9",
            Shape::Deck => "16:10",
            Shape::Classic => "4:3",
            Shape::Ultra => "21:9",
        }
    }
}

/// The library as the preview shows it: collections with their games.
struct Data {
    generation: u64,
    colls: Vec<CollectionItem>,
    games: Vec<Vec<usize>>,
}

fn build_data(lib: &Library) -> Data {
    let mut colls = vec![];
    let mut games = vec![];
    for c in lib.collections() {
        let list = lib.collection_games(&c);
        let mut by_recent = list.clone();
        by_recent.sort_by_key(|&i| std::cmp::Reverse(lib.games[i].last_played.unwrap_or(0)));
        let pick = by_recent
            .iter()
            .find(|&&i| lib.games[i].art.hero.is_some())
            .or_else(|| by_recent.iter().find(|&&i| lib.games[i].art.cover.is_some()))
            .copied();
        let mut art = crate::model::Art::default();
        if let Some(i) = pick {
            art = lib.games[i].art.clone();
            if art.cover.is_none() {
                art.cover = by_recent.iter().find_map(|&j| lib.games[j].art.cover.clone());
            }
        }
        let name = match &c.kind {
            CollectionKind::Tag(t) => format!("#{t}"),
            CollectionKind::Platform(id) => crate::ui::platform_icons::console_name(id),
            _ => c.name.clone(),
        };
        colls.push(CollectionItem { count: list.len(), name, collection: c, art });
        games.push(list);
    }
    Data { generation: lib.generation, colls, games }
}

struct Drag {
    top: usize,
    handle: Handle,
    start_box: Rect,
    start_ptr: Pos2,
    snap: Snap,
    gesture: u64,
}

/// What the cached prepared view was made for.
#[derive(Clone, PartialEq)]
struct PrepKey {
    rev: u64,
    view: ViewKind,
    coll: Option<String>,
    variant: Option<String>,
    aspect: Option<String>,
}

pub struct Preview {
    renderer: Renderer,
    theme: Option<Arc<Theme>>,
    theme_for: (u64, Option<String>),
    data: Option<Data>,
    prepared: Option<(PrepKey, Arc<Prepared>)>,
    pub shape: Shape,
    /// Index into `shape.sizes()`; None: as big as fits, drawn sharp.
    pub size: Option<usize>,
    pub outlines: bool,
    pub snap: bool,
    pub try_mode: bool,
    pub coll_sel: usize,
    pub game_coll: usize,
    pub game_sel: usize,
    /// Colour scheme the preview shows (None: the theme's own colours).
    pub scheme: Option<String>,
    /// Variant the preview shows while changes go to the whole theme.
    pub variant: Option<String>,
    drag: Option<Drag>,
    guides: Guides,
    gesture: u64,
    findings: Option<(u64, Vec<Finding>)>,
    /// A preview picture was asked for (the next frame takes it).
    pub capture: bool,
    /// The last frame's view frame on screen.
    pub frame: Rect,
}

impl Default for Preview {
    fn default() -> Self {
        Preview {
            renderer: Renderer::new(),
            theme: None,
            theme_for: (0, None),
            data: None,
            prepared: None,
            shape: Shape::Wide,
            size: None,
            outlines: false,
            snap: true,
            try_mode: false,
            coll_sel: 0,
            game_coll: 0,
            game_sel: 0,
            scheme: None,
            variant: None,
            drag: None,
            guides: Guides::default(),
            gesture: 0,
            findings: None,
            capture: false,
            frame: Rect::NOTHING,
        }
    }
}

impl Preview {
    /// The theme check, cached per change.
    pub fn findings(&mut self, doc: &Doc) -> &[Finding] {
        if self.findings.as_ref().is_none_or(|(r, _)| *r != doc.rev) {
            self.findings = Some((doc.rev, validate::check(&doc.file, Some(&doc.dir))));
        }
        &self.findings.as_ref().expect("just set").1
    }

    /// Files of the theme changed on disk: look again.
    pub fn forget_files(&mut self) {
        self.renderer = Renderer::new();
        self.theme_for.0 = 0;
        self.prepared = None;
    }

    fn ensure_theme(&mut self, doc: &Doc) -> Arc<Theme> {
        let want = (doc.rev, self.scheme.clone());
        if self.theme_for != want || self.theme.is_none() {
            let file = match &self.scheme {
                Some(s) => doc.file.with_color_scheme(s),
                None => doc.file.clone(),
            };
            // The old theme stays alive until the new one exists: the renderer tells themes
            // apart by their address.
            let new = Arc::new(Theme { key: format!("theme-creator:{}", doc.key), file, dir: Some(doc.dir.clone()) });
            self.theme = Some(new);
            self.theme_for = want;
            self.prepared = None;
        }
        self.theme.clone().expect("just set")
    }

    fn ensure_data(&mut self, lib: &Library) {
        if self.data.as_ref().is_none_or(|d| d.generation != lib.generation) {
            self.data = Some(build_data(lib));
        }
    }

    /// The collections the preview can show (names, for the "Showing" choice).
    pub fn collections(&self) -> Vec<(usize, String, String)> {
        self.data
            .as_ref()
            .map(|d| d.colls.iter().enumerate().map(|(i, c)| (i, c.name.clone(), c.collection.key())).collect())
            .unwrap_or_default()
    }

    /// The collection the preview shows on `view`.
    fn coll_index(&self, view: ViewKind) -> usize {
        if view == ViewKind::Collections { self.coll_sel } else { self.game_coll }
    }

    /// Every value of the values picker for what the preview shows now.
    pub fn binding_values(&self, st: &AppState, doc: Option<&Doc>, view: ViewKind) -> HashMap<&'static str, String> {
        let mut out = HashMap::new();
        let Some(data) = &self.data else { return out };
        let ci = self.coll_index(view);
        let coll = data.colls.get(ci);
        let list = data.games.get(self.game_coll).map(Vec::as_slice).unwrap_or(&[]);
        let game =
            if view == ViewKind::Collections { None } else { list.get(self.game_sel).map(|&i| &st.library.games[i]) };
        let b = crate::theme::render::Binder {
            game,
            collection: coll,
            theme_dir: doc.map(|d| d.dir.as_path()),
            list: Some((self.game_sel, list.len())),
            section: None,
            focused: true,
            page: view == ViewKind::Details,
            collection_art: doc.map(|d| &d.file.collection_art),
        };
        for k in super::bindings::ALL {
            let mut s = String::new();
            b.value(k.key, false, &mut s);
            let s: String = s.chars().take(80).collect();
            out.insert(k.key, s.replace('\n', " "));
        }
        out
    }

    /// The view as drawn: base, then the variant, the screen shape's layout and the console's
    /// look the preview is set to (`layer` decides which ones when changes go to one of them).
    fn prepared(&mut self, theme: &Theme, view: ViewKind, layer: &Layer, rev: u64) -> Option<Arc<Prepared>> {
        let data = self.data.as_ref()?;
        let ci = self.coll_index(view);
        let coll = data.colls.get(ci).map(|c| &c.collection);
        let variant = match layer {
            Layer::Variant(v) => Some(v.clone()),
            _ => self.variant.clone(),
        };
        let aspect = match layer {
            Layer::Aspect(a) => Some(a.clone()),
            _ => theme.file.aspect_key(self.shape.ratio()).map(str::to_string),
        };
        let key = PrepKey { rev, view, coll: coll.map(|c| c.key()), variant: variant.clone(), aspect: aspect.clone() };
        if let Some((k, p)) = &self.prepared
            && *k == key
        {
            return Some(p.clone());
        }
        // A console's look: show that console.
        let p = Arc::new(prepare_view_for(&theme.file, view, coll, variant.as_deref(), aspect.as_deref())?);
        self.prepared = Some((key, p.clone()));
        Some(p)
    }

    /// Show the console a console look is for.
    pub fn show_console(&mut self, key: &str, view: ViewKind) {
        let Some(data) = &self.data else { return };
        let found =
            data.colls.iter().position(|c| c.collection.key() == key || super::files::art_key(&c.collection) == key);
        if let Some(i) = found {
            if view == ViewKind::Collections {
                self.coll_sel = i;
            } else if self.game_coll != i {
                self.game_coll = i;
                self.game_sel = 0;
            }
        }
    }
}

fn sample_hints() -> Vec<Hint> {
    use crate::input::style::Button;
    vec![
        Hint::new(Button::A, "Play"),
        Hint::new(Button::B, "Back"),
        Hint::new(Button::X, "Favourite"),
        Hint::new(Button::Y, "Search"),
        Hint::new(Button::Start, "Menu"),
    ]
}

/// The preview with its bars.
pub fn show(ui: &mut Ui, st: &mut AppState, d: &mut CreatorDlg, pal: &Pal, h: f32) {
    if d.doc.is_none() {
        return;
    }
    d.pv.ensure_data(&st.library);
    toolbar(ui, st, d, pal);
    let Some(doc) = d.doc.as_ref() else { return };
    let theme = d.pv.ensure_theme(doc);
    {
        let (dt, ct) = (st.desktop_theme.clone(), st.controller_theme.clone());
        st.fonts.ensure(ui.ctx(), &[&dt, &ct, &theme]);
    }
    if theme.file.view(d.view).is_none() {
        d.view = ViewKind::Games;
    }
    let rev = doc.rev;
    let Some(prepared) = d.pv.prepared(&theme, d.view, &d.layer, rev) else { return };
    // Room: what's left, the shape's proportions, centred.
    let avail = vec2(ui.available_width(), (h - ui.min_rect().height() - 34.0).max(160.0));
    let ratio = d.pv.shape.ratio();
    let size = if avail.x / avail.y > ratio { vec2(avail.y * ratio, avail.y) } else { vec2(avail.x, avail.x / ratio) };
    let (outer, _) = ui.allocate_exact_size(vec2(avail.x, size.y), Sense::hover());
    let screen = Rect::from_center_size(outer.center(), size);
    draw_view(ui, st, d, &theme, &prepared, screen);
    if d.doc.is_some() {
        interact(ui, st, d, &prepared, screen, pal);
    }
    if std::mem::take(&mut d.pv.capture) {
        capture(ui.ctx(), d, screen);
    }
    ui.add_space(6.0);
    let hint = if d.pv.try_mode {
        "Try it: arrow keys move through the list, Enter opens, Backspace goes back. Click “Edit” to change things again."
    } else {
        "Click to select · drag to move · drag the handles to resize (Alt: no snapping) · arrow keys nudge · Delete removes"
    };
    widgets::note(ui, hint, pal);
}

/// The preview without its tools, beside the theme pages (colours, fonts, pictures …).
pub fn mini(ui: &mut Ui, st: &mut AppState, d: &mut CreatorDlg, pal: &Pal) {
    let Some(doc) = d.doc.as_ref() else { return };
    d.pv.ensure_data(&st.library);
    ui.horizontal(|ui| {
        for (kind, name) in validate::VIEW_NAMES {
            if doc.file.view(kind).is_some() && ui.add(egui::Button::selectable(d.view == kind, name)).clicked() {
                d.view = kind;
                d.sel = None;
            }
        }
    });
    let theme = d.pv.ensure_theme(doc);
    {
        let (dt, ct) = (st.desktop_theme.clone(), st.controller_theme.clone());
        st.fonts.ensure(ui.ctx(), &[&dt, &ct, &theme]);
    }
    if theme.file.view(d.view).is_none() {
        d.view = ViewKind::Games;
    }
    let Some(prepared) = d.pv.prepared(&theme, d.view, &d.layer, doc.rev) else { return };
    let w = ui.available_width();
    let size = vec2(w, w / d.pv.shape.ratio());
    let (screen, _) = ui.allocate_exact_size(size, Sense::hover());
    let try_mode = std::mem::replace(&mut d.pv.try_mode, false);
    draw_view(ui, st, d, &theme, &prepared, screen);
    d.pv.try_mode = try_mode;
    ui.add_space(4.0);
    widgets::note(ui, "Live preview — changes show at once.", pal);
}

fn toolbar(ui: &mut Ui, st: &mut AppState, d: &mut CreatorDlg, pal: &Pal) {
    let Some(doc) = d.doc.as_mut() else { return };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for (kind, name) in validate::VIEW_NAMES {
            if doc.file.view(kind).is_none() {
                continue;
            }
            if ui.add(egui::Button::selectable(d.view == kind, name)).clicked() && d.view != kind {
                d.view = kind;
                d.sel = None;
            }
        }
        let missing: Vec<(ViewKind, &str)> = [(ViewKind::Details, "game page"), (ViewKind::Home, "start page")]
            .into_iter()
            .filter(|(k, _)| doc.file.view(*k).is_none())
            .collect();
        if !missing.is_empty() {
            ui.menu_button("＋", |ui| {
                for (k, n) in missing {
                    if ui.button(format!("Add a {n}")).on_hover_text(add_view_hint(k)).clicked() {
                        doc.change("Add a screen", |f| super::panel::add_view(f, k));
                        d.view = k;
                        d.sel = None;
                    }
                }
            })
            .response
            .on_hover_text("Add a screen the theme doesn't have yet");
        }
        ui.separator();
        egui::ComboBox::from_id_salt("ls_creator_shape").selected_text(d.pv.shape.label()).show_ui(ui, |ui| {
            for s in Shape::ALL {
                if ui.selectable_value(&mut d.pv.shape, s, s.label()).changed() {
                    d.pv.size = None;
                }
            }
        });
        let size_label = match d.pv.size.and_then(|i| d.pv.shape.sizes().get(i)) {
            Some([w, h]) => format!("{w}×{h}"),
            None => "Fit".into(),
        };
        egui::ComboBox::from_id_salt("ls_creator_size")
            .selected_text(size_label)
            .width(86.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut d.pv.size, None, "Fit (sharpest)");
                for (i, [w, h]) in d.pv.shape.sizes().iter().enumerate() {
                    ui.selectable_value(&mut d.pv.size, Some(i), format!("{w}×{h}"));
                }
            })
            .response
            .on_hover_text("Draw the preview as a screen of this many pixels, scaled to fit");
        ui.separator();
        ui.toggle_value(&mut d.pv.outlines, "Outlines").on_hover_text("Show every element's box");
        ui.toggle_value(&mut d.pv.snap, "Snap")
            .on_hover_text("Line edges and centres up with the screen and other elements");
        ui.separator();
        if ui.add(egui::Button::selectable(!d.pv.try_mode, "Edit")).clicked() {
            d.pv.try_mode = false;
        }
        if ui
            .add(egui::Button::selectable(d.pv.try_mode, "Try it"))
            .on_hover_text("Move through the preview with the arrow keys")
            .clicked()
        {
            d.pv.try_mode = true;
            d.sel = None;
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(RichText::new("Changes go to").size(12.0).color(pal.dim));
        layer_menu(ui, st, d);
        let Some(doc) = d.doc.as_ref() else { return };
        {
            ui.label(
                RichText::new(if d.view == ViewKind::Collections { "Selected" } else { "Showing" })
                    .size(12.0)
                    .color(pal.dim),
            );
            let colls = d.pv.collections();
            let cur_i = if d.view == ViewKind::Collections { d.pv.coll_sel } else { d.pv.game_coll };
            let cur = colls.get(cur_i).map(|c| c.1.clone()).unwrap_or_else(|| "—".into());
            egui::ComboBox::from_id_salt("ls_creator_coll").selected_text(cur).width(150.0).show_ui(ui, |ui| {
                for (i, n, _) in &colls {
                    if ui.selectable_label(cur_i == *i, n).clicked() {
                        if d.view == ViewKind::Collections {
                            d.pv.coll_sel = *i;
                        } else {
                            d.pv.game_coll = *i;
                            d.pv.game_sel = 0;
                        }
                    }
                }
            });
        }
        if !doc.file.variants.is_empty() && !matches!(d.layer, Layer::Variant(_)) {
            ui.label(RichText::new("Variant").size(12.0).color(pal.dim));
            let cur = d.pv.variant.clone().unwrap_or_else(|| "Default".into());
            egui::ComboBox::from_id_salt("ls_creator_variant").selected_text(cur).show_ui(ui, |ui| {
                ui.selectable_value(&mut d.pv.variant, None, "Default");
                for k in doc.file.variants.keys() {
                    ui.selectable_value(&mut d.pv.variant, Some(k.clone()), k);
                }
            });
        }
        if !doc.file.color_schemes.is_empty() {
            ui.label(RichText::new("Colours").size(12.0).color(pal.dim));
            let cur =
                d.pv.scheme
                    .as_ref()
                    .and_then(|k| doc.file.color_schemes.get(k).map(|s| s.label_or(k).to_string()))
                    .unwrap_or_else(|| "Theme's own".into());
            egui::ComboBox::from_id_salt("ls_creator_scheme").selected_text(cur).show_ui(ui, |ui| {
                ui.selectable_value(&mut d.pv.scheme, None, "Theme's own");
                for (k, s) in &doc.file.color_schemes {
                    ui.selectable_value(&mut d.pv.scheme, Some(k.clone()), s.label_or(k));
                }
            });
        }
        ui.separator();
        if ui
            .button("Snapshot")
            .on_hover_text("Keep this view as one of the theme's preview pictures (Details)")
            .clicked()
        {
            d.pv.capture = true;
        }
    });
    ui.add_space(4.0);
}

fn add_view_hint(k: ViewKind) -> &'static str {
    match k {
        ViewKind::Details => "A page for one game (without one, Lodestar shows its own)",
        ViewKind::Home => "A start page with rows of covers and a navigation rail",
        _ => "",
    }
}

/// "Changes go to": the whole theme, a variant, a screen shape's layout or a console's look.
fn layer_menu(ui: &mut Ui, st: &AppState, d: &mut CreatorDlg) {
    let Some(doc) = d.doc.as_mut() else { return };
    let label = layer_label(&d.layer, st);
    let mut chosen: Option<Layer> = None;
    egui::ComboBox::from_id_salt("ls_creator_layer").selected_text(label).width(210.0).show_ui(ui, |ui| {
        if ui.selectable_label(d.layer == Layer::Base, "The whole theme").clicked() {
            chosen = Some(Layer::Base);
        }
        if !doc.file.variants.is_empty() {
            ui.separator();
            for k in doc.file.variants.keys() {
                let l = Layer::Variant(k.clone());
                if ui.selectable_label(d.layer == l, format!("Variant “{k}” only")).clicked() {
                    chosen = Some(l);
                }
            }
        }
        ui.separator();
        let shape = Layer::Aspect(d.pv.shape.key().to_string());
        let has = doc.file.aspects.contains_key(d.pv.shape.key());
        let txt = if has {
            format!("{} screens only", d.pv.shape.key())
        } else {
            format!("{} screens only (new)", d.pv.shape.key())
        };
        if ui.selectable_label(d.layer == shape, txt).on_hover_text("A layout used on screens of this shape").clicked()
        {
            chosen = Some(shape);
        }
        for k in doc.file.aspects.keys().filter(|k| k.as_str() != d.pv.shape.key()) {
            let l = Layer::Aspect(k.clone());
            if ui.selectable_label(d.layer == l, format!("{k} screens only")).clicked() {
                chosen = Some(l);
            }
        }
        ui.separator();
        let ci = if d.view == ViewKind::Collections { d.pv.coll_sel } else { d.pv.game_coll };
        if let Some((_, name, key)) = d.pv.collections().into_iter().nth(ci) {
            let l = Layer::Console(key.clone());
            if ui
                .selectable_label(d.layer == l, format!("{name} only"))
                .on_hover_text("A look for this console or collection (the one the preview shows)")
                .clicked()
            {
                chosen = Some(l);
            }
        }
        for k in doc.file.overrides.keys() {
            let l = Layer::Console(k.clone());
            if l != Layer::Console(String::new())
                && ui.selectable_label(d.layer == l, format!("{} only", console_label(k, st))).clicked()
            {
                chosen = Some(l);
            }
        }
    });
    if let Some(l) = chosen {
        if let Layer::Console(k) = &l {
            d.pv.show_console(k, d.view);
        }
        if let Layer::Aspect(a) = &l {
            d.pv.shape = Shape::from_key(a);
        }
        d.layer = l;
        d.raw.reset();
    }
    if d.layer != Layer::Base && ui.small_button("✕").on_hover_text("Back to changing the whole theme").clicked() {
        d.layer = Layer::Base;
        d.raw.reset();
    }
}

pub fn layer_label(l: &Layer, st: &AppState) -> String {
    match l {
        Layer::Base => "The whole theme".into(),
        Layer::Variant(k) => format!("Variant “{k}” only"),
        Layer::Aspect(k) => format!("{k} screens only"),
        Layer::Console(k) => format!("{} only", console_label(k, st)),
    }
}

/// A collection key as people read it ("platform:sony_playstation2" → "PlayStation 2").
pub fn console_label(key: &str, st: &AppState) -> String {
    if let Some(id) = key.strip_prefix("platform:") {
        return crate::ui::platform_icons::console_name(id);
    }
    st.library.collections().into_iter().find(|c| c.key() == key).map(|c| c.name).unwrap_or_else(|| key.to_string())
}

fn draw_view(
    ui: &mut Ui,
    st: &mut AppState,
    d: &mut CreatorDlg,
    theme: &Arc<Theme>,
    prepared: &Prepared,
    screen: Rect,
) {
    let Some(data) = d.pv.data.as_ref() else { return };
    let view = d.view;
    // Selections within the library.
    let nc = data.colls.len();
    d.pv.coll_sel = d.pv.coll_sel.min(nc.saturating_sub(1));
    d.pv.game_coll = d.pv.game_coll.min(nc.saturating_sub(1));
    let list: &[usize] = data.games.get(d.pv.game_coll).map(Vec::as_slice).unwrap_or(&[]);
    d.pv.game_sel = d.pv.game_sel.min(list.len().saturating_sub(1));
    let coll = data.colls.get(if view == ViewKind::Collections { d.pv.coll_sel } else { d.pv.game_coll });
    let named =
        prepared.display_name.as_ref().and_then(|n| coll.map(|c| CollectionItem { name: n.clone(), ..c.clone() }));
    let coll = named.as_ref().or(coll);
    let game =
        if view == ViewKind::Collections { None } else { list.get(d.pv.game_sel).map(|&i| &st.library.games[i]) };
    let items = match view {
        ViewKind::Collections => Items::Collections(&data.colls),
        _ => Items::Games { games: &st.library.games, indices: list },
    };
    // Start page rows / the rail.
    let mut row_specs: Option<(Vec<String>, usize, usize)> = None;
    let mut has_rail = false;
    for e in &prepared.view.elements {
        if let Element::Rows { rows, max_rows, row_items, .. } = e {
            row_specs = Some((rows.clone(), *max_rows as usize, *row_items as usize));
        }
        has_rail |= matches!(e, Element::Rail { .. });
    }
    let picked = row_specs
        .as_ref()
        .map(|(s, m, _)| crate::theme::render::pick_rows(s, &data.colls, (*m).max(1)))
        .unwrap_or_default();
    let per_row = row_specs.as_ref().map_or(30, |r| r.2.max(1));
    let rows: Vec<RowScene> = picked
        .iter()
        .filter_map(|&ci| {
            let g = data.games.get(ci)?;
            Some(RowScene { collection: data.colls.get(ci)?, indices: &g[..g.len().min(per_row)], selected: 0 })
        })
        .collect();
    let extra = SceneExtra {
        games: &st.library.games,
        rows: &rows,
        row: 0,
        rail: has_rail.then_some(RailScene { shown: &[], current: Some(0), selected: 0, focused: false }),
        section: None,
    };
    let hints = sample_hints();
    let selected = if view == ViewKind::Collections { d.pv.coll_sel } else { d.pv.game_sel };
    let scaled = d.pv.size.and_then(|i| d.pv.shape.sizes().get(i)).map(|[w, h]| vec2(*w as f32, *h as f32));
    let scene = Scene {
        kind: view,
        game,
        collection: coll,
        items,
        selected,
        list_id: (d.pv.game_coll as u64) << 8 | view as u64,
        hints: &hints,
        prompts: st.prompts(),
        opacity: 1.0,
        interactive: d.pv.try_mode && scaled.is_none(),
        extra,
    };
    d.pv.renderer.opts = ViewOpts { reduce_motion: false, ..ViewOpts::from_config(&st.config.controller) };
    // Keep the renderer's own caches in step with this theme (it resets them per theme).
    let _ = d.pv.renderer.prepare(theme, view, coll.map(|c| &c.collection), None);
    let out = match scaled {
        None => {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(screen));
            child.set_clip_rect(screen.intersect(ui.clip_rect()));
            d.pv.frame = view_frame(&prepared.view, screen);
            d.pv.renderer.draw(&mut child, screen, theme, prepared, &st.fonts, &mut st.images, &scene)
        }
        Some(px) => {
            // A screen of that many points, scaled down to the preview.
            let ppp = ui.ctx().pixels_per_point();
            let virt = Rect::from_min_size(screen.min, px / ppp);
            let k = screen.width() / virt.width();
            let t = egui::emath::TSTransform::new(screen.min.to_vec2() * (1.0 - k), k);
            d.pv.frame = {
                let f = view_frame(&prepared.view, virt);
                Rect::from_min_max(t * f.min, t * f.max)
            };
            ui.with_visual_transform(t, |ui| {
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(virt));
                child.set_clip_rect(virt);
                d.pv.renderer.draw(&mut child, virt, theme, prepared, &st.fonts, &mut st.images, &scene)
            })
            .inner
        }
    };
    if d.pv.try_mode {
        if let Some(i) = out.clicked {
            if view == ViewKind::Collections {
                d.pv.coll_sel = i;
            } else {
                d.pv.game_sel = i;
            }
        }
        if out.scroll_steps != 0 {
            step_selection(d, out.scroll_steps as isize, nc, list.len());
        }
    }
    // A thin frame around the screen.
    ui.painter().rect_stroke(screen, 2.0, Stroke::new(1.0, Color32::from_white_alpha(30)), egui::StrokeKind::Outside);
}

fn step_selection(d: &mut CreatorDlg, by: isize, ncolls: usize, ngames: usize) {
    let (v, n) =
        if d.view == ViewKind::Collections { (&mut d.pv.coll_sel, ncolls) } else { (&mut d.pv.game_sel, ngames) };
    if n == 0 {
        return;
    }
    *v = (*v as isize + by).clamp(0, n as isize - 1) as usize;
}

/// The sizes elements with an automatic size get (measured text, square pictures), as
/// fractions of the frame.
fn natural_sizes(ui: &Ui, st: &AppState, theme: &Theme, prepared: &Prepared, frame: Rect) -> Vec<Vec2> {
    let (fw, fh) = (frame.width().max(1.0), frame.height().max(1.0));
    prepared
        .view
        .elements
        .iter()
        .map(|e| {
            let c = e.common();
            if c.size[0] > 0.0 && c.size[1] > 0.0 {
                return vec2(c.size[0], c.size[1]);
            }
            let text_size = |text: &str, style: &crate::theme::TextStyle| -> Vec2 {
                let family = st.fonts.family(theme, style.font.as_deref());
                let px = (style.font_size * fh).max(4.0);
                let t = if text.contains('{') { "Sample text" } else { text };
                let wrap = if c.size[0] > 0.0 { c.size[0] * fw } else { f32::INFINITY };
                let galley =
                    ui.fonts_mut(|f| f.layout(t.to_string(), egui::FontId::new(px, family), Color32::WHITE, wrap));
                vec2(galley.size().x / fw, galley.size().y / fh)
            };
            match e {
                Element::Text { text, style, .. } => text_size(text, style),
                Element::DateTime { style, .. } => text_size("00:00", style),
                Element::Image { .. } => {
                    let side = if c.size[0] > 0.0 {
                        c.size[0] * fw
                    } else if c.size[1] > 0.0 {
                        c.size[1] * fh
                    } else {
                        0.1 * fh
                    };
                    vec2(side / fw, side / fh)
                }
                Element::Rating { .. } => vec2(0.15 * fh / fw, 0.03),
                _ => vec2(c.size[0].max(0.05), c.size[1].max(0.05)),
            }
        })
        .collect()
}

fn to_screen(frame: Rect, r: Rect) -> Rect {
    Rect::from_min_max(
        pos2(frame.min.x + r.min.x * frame.width(), frame.min.y + r.min.y * frame.height()),
        pos2(frame.min.x + r.max.x * frame.width(), frame.min.y + r.max.y * frame.height()),
    )
}

const HANDLE: f32 = 7.0;

/// Selecting, dragging, resizing; keys; outlines and guides.
fn interact(ui: &mut Ui, st: &mut AppState, d: &mut CreatorDlg, prepared: &Prepared, screen: Rect, pal: &Pal) {
    let Some(theme) = d.pv.theme.clone() else { return };
    let frame = d.pv.frame;
    let id = ui.scope_id().with("ls_creator_canvas");
    let resp = ui.interact(screen, id, if d.pv.try_mode { Sense::click() } else { Sense::click_and_drag() });
    if resp.clicked() || resp.drag_started() {
        resp.request_focus();
    }
    let focused = resp.has_focus() || resp.hovered();
    let els = &prepared.view.elements;
    let naturals = natural_sizes(ui, st, &theme, prepared, frame);
    let boxes: Vec<(usize, Rect)> = ops::boxes(els, &|i| naturals.get(i).copied().unwrap_or(Vec2::splat(0.05)));
    let painter = ui.painter_at(screen);
    if d.pv.try_mode {
        if focused {
            try_keys(ui, st, d, prepared);
        }
        return;
    }
    let ptr = ui.input(|i| i.pointer.interact_pos().or(i.pointer.hover_pos()));
    let hit = |p: Pos2| boxes.iter().rev().find(|(_, r)| to_screen(frame, *r).expand(2.0).contains(p)).map(|(i, _)| *i);
    let sel_top = d.sel.as_ref().map(|p| p.top);
    let sel_box = sel_top.and_then(|t| boxes.iter().find(|(i, _)| *i == t)).map(|(_, r)| *r);
    // Hover cursor over handles.
    if let (Some(p), Some(sb)) = (ptr, sel_box)
        && resp.hovered()
        && d.pv.drag.is_none()
    {
        let sr = to_screen(frame, sb);
        if let Some(h) = Handle::CORNERS_AND_EDGES.into_iter().find(|h| h.point(sr).distance(p) <= HANDLE + 2.0) {
            ui.ctx().set_cursor_icon(h.cursor());
        } else if sr.contains(p) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        }
    }
    // Where the drag began (egui reports it once the pointer has moved a little).
    if resp.drag_started()
        && let Some(p) = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos())
    {
        let mut start: Option<(usize, Handle, Rect)> = None;
        if let (Some(t), Some(sb)) = (sel_top, sel_box) {
            let sr = to_screen(frame, sb);
            if let Some(h) = Handle::CORNERS_AND_EDGES.into_iter().find(|h| h.point(sr).distance(p) <= HANDLE + 2.0) {
                start = Some((t, h, sb));
            } else if sr.contains(p) {
                start = Some((t, Handle::Move, sb));
            }
        }
        if start.is_none()
            && let Some(i) = hit(p)
        {
            d.sel = Some(ElPath::top(i));
            d.raw.reset();
            start = boxes.iter().find(|(j, _)| *j == i).map(|(_, r)| (i, Handle::Move, *r));
        }
        if let Some((top, handle, start_box)) = start {
            let others: Vec<Rect> = boxes.iter().filter(|(j, _)| *j != top).map(|(_, r)| *r).collect();
            let reach = vec2(6.0 / frame.width().max(1.0), 6.0 / frame.height().max(1.0));
            d.pv.gesture += 1;
            d.pv.drag = Some(Drag {
                top,
                handle,
                start_box,
                start_ptr: p,
                snap: Snap::new(&others, reach),
                gesture: d.pv.gesture,
            });
        }
    }
    if let Some(dr) = &d.pv.drag
        && resp.dragged()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let delta =
            vec2((p.x - dr.start_ptr.x) / frame.width().max(1.0), (p.y - dr.start_ptr.y) / frame.height().max(1.0));
        let free = ui.input(|i| i.modifiers.alt) || !d.pv.snap;
        let (nb, guides) = geom::drag(dr.start_box, dr.handle, delta, (!free).then_some(&dr.snap));
        d.pv.guides = guides;
        let (top, handle, gesture) = (dr.top, dr.handle, dr.gesture);
        if let Some(doc) = d.doc.as_mut() {
            let label = if handle == Handle::Move { "Move an element" } else { "Resize an element" };
            let layer = d.layer.clone();
            let view = d.view;
            doc.change_merged(&format!("drag{gesture}"), label, |f| {
                let _ = ops::edit_in_layer(f, &layer, view, &ElPath::top(top), |e| {
                    geom::apply_box(e.common_mut(), nb, [handle.sizes_x(), handle.sizes_y()]);
                });
            });
        }
    }
    if resp.drag_stopped() {
        d.pv.drag = None;
        d.pv.guides = Guides::default();
        if let Some(doc) = d.doc.as_mut() {
            doc.break_merge();
        }
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let i = hit(p);
        let new = i.map(ElPath::top);
        if new.as_ref().map(|p| p.top) != d.sel.as_ref().map(|p| p.top) {
            d.sel = new;
            d.raw.reset();
        }
    }
    if resp.has_focus() {
        edit_keys(ui, d);
    }
    // Outlines.
    let accent = pal.accent;
    if d.pv.outlines {
        for (i, r) in &boxes {
            let sr = to_screen(frame, *r);
            painter.rect_stroke(
                sr,
                0.0,
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 90)),
                egui::StrokeKind::Inside,
            );
            let name = els[*i].common().name.as_str();
            if !name.is_empty() {
                painter.text(
                    sr.min + vec2(3.0, 2.0),
                    egui::Align2::LEFT_TOP,
                    name,
                    egui::FontId::proportional(10.0),
                    Color32::from_white_alpha(170),
                );
            }
        }
    }
    if let (Some(p), None) = (ptr, &d.pv.drag)
        && resp.hovered()
        && let Some(i) = hit(p)
        && Some(i) != sel_top
        && let Some((_, r)) = boxes.iter().find(|(j, _)| *j == i)
    {
        painter.rect_stroke(
            to_screen(frame, *r),
            0.0,
            Stroke::new(1.0, crate::ui::widgets::with_alpha(accent, 150)),
            egui::StrokeKind::Outside,
        );
    }
    if let Some(sb) = sel_box {
        let sr = to_screen(frame, sb);
        painter.rect_stroke(sr, 0.0, Stroke::new(1.5, accent), egui::StrokeKind::Outside);
        for h in Handle::CORNERS_AND_EDGES {
            let c = h.point(sr);
            let hr = Rect::from_center_size(c, Vec2::splat(HANDLE));
            painter.rect_filled(hr, 1.5, Color32::WHITE);
            painter.rect_stroke(hr, 1.5, Stroke::new(1.0, accent), egui::StrokeKind::Inside);
        }
        // Where it is, in the preview's pixels.
        let px =
            d.pv.size
                .and_then(|i| d.pv.shape.sizes().get(i))
                .map_or([1920.0, 1920.0 / d.pv.shape.ratio()], |s| [s[0] as f32, s[1] as f32]);
        let txt = format!(
            "{} · {:.0}, {:.0} · {:.0} × {:.0} px",
            sel_name(els, d.sel.as_ref()),
            sb.min.x * px[0],
            sb.min.y * px[1],
            sb.width() * px[0],
            sb.height() * px[1]
        );
        let at = pos2(sr.min.x, (sr.min.y - 4.0).max(screen.min.y + 12.0));
        let galley = painter.layout_no_wrap(txt, egui::FontId::proportional(11.0), Color32::WHITE);
        let bg = Rect::from_min_size(at - vec2(0.0, galley.size().y + 2.0), galley.size() + vec2(8.0, 2.0));
        painter.rect_filled(bg, 3.0, crate::ui::widgets::with_alpha(accent, 230));
        painter.galley(bg.min + vec2(4.0, 1.0), galley, Color32::WHITE);
    }
    let guide = Color32::from_rgb(255, 80, 200);
    for x in &d.pv.guides.xs {
        let sx = frame.min.x + x * frame.width();
        painter.line_segment([pos2(sx, screen.min.y), pos2(sx, screen.max.y)], Stroke::new(1.0, guide));
    }
    for y in &d.pv.guides.ys {
        let sy = frame.min.y + y * frame.height();
        painter.line_segment([pos2(screen.min.x, sy), pos2(screen.max.x, sy)], Stroke::new(1.0, guide));
    }
}

fn sel_name(els: &[Element], p: Option<&ElPath>) -> String {
    let Some(e) = p.and_then(|p| els.get(p.top)) else { return String::new() };
    let n = e.common().name.trim();
    if n.is_empty() { ops::kind_word(e).to_string() } else { n.to_string() }
}

/// Edit mode keys: arrows nudge (Shift: more), Delete removes, Ctrl+D duplicates.
fn edit_keys(ui: &mut Ui, d: &mut CreatorDlg) {
    let Some(sel) = d.sel.clone() else { return };
    let (dx, dy, del, dup) = ui.input_mut(|i| {
        let big = i.modifiers.shift;
        let s = if big { 0.01 } else { 0.001 };
        let mut v = (0.0, 0.0);
        for (k, dv) in [
            (Key::ArrowLeft, (-s, 0.0)),
            (Key::ArrowRight, (s, 0.0)),
            (Key::ArrowUp, (0.0, -s)),
            (Key::ArrowDown, (0.0, s)),
        ] {
            if i.consume_key(egui::Modifiers::NONE, k) || i.consume_key(egui::Modifiers::SHIFT, k) {
                v = dv;
            }
        }
        let del = i.consume_key(egui::Modifiers::NONE, Key::Delete);
        let dup = i.consume_key(egui::Modifiers::COMMAND, Key::D);
        (v.0, v.1, del, dup)
    });
    let ctx = ui.ctx().clone();
    if dx != 0.0 || dy != 0.0 {
        let top = ElPath::top(sel.top);
        if let Some(doc) = d.doc.as_mut() {
            super::edit_element(doc, &ctx, &d.layer.clone(), d.view, &top, &mut d.banner, |e| {
                geom::nudge(e.common_mut(), vec2(dx, dy))
            });
        }
    }
    if del {
        super::panel::delete_selected(d);
    }
    if dup {
        super::panel::duplicate_selected(d);
    }
}

/// "Try it" keys: move through the focused list like the controller does.
fn try_keys(ui: &mut Ui, st: &AppState, d: &mut CreatorDlg, prepared: &Prepared) {
    let info = prepared.focus_info(d.pv.frame);
    let (l, r, u, dn, enter, back) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, Key::ArrowLeft),
            i.consume_key(egui::Modifiers::NONE, Key::ArrowRight),
            i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
            i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, Key::Enter),
            i.consume_key(egui::Modifiers::NONE, Key::Backspace) || i.consume_key(egui::Modifiers::NONE, Key::Escape),
        )
    });
    let (nc, ng) =
        d.pv.data.as_ref().map_or((0, 0), |x| (x.colls.len(), x.games.get(d.pv.game_coll).map_or(0, Vec::len)));
    let step = match info.kind {
        FocusKind::Grid { columns } => {
            let c = columns as isize;
            (if l { -1 } else { 0 }) + (if r { 1 } else { 0 }) + (if u { -c } else { 0 }) + (if dn { c } else { 0 })
        }
        FocusKind::Carousel { vertical: true } | FocusKind::List => (if u { -1 } else { 0 }) + (if dn { 1 } else { 0 }),
        _ => (if l || u { -1 } else { 0 }) + (if r || dn { 1 } else { 0 }),
    };
    if step != 0 {
        step_selection(d, step, nc, ng);
    }
    let has_details = d.doc.as_ref().is_some_and(|x| x.file.details.is_some());
    if enter {
        match d.view {
            ViewKind::Collections => {
                d.pv.game_coll = d.pv.coll_sel;
                d.pv.game_sel = 0;
                d.view = ViewKind::Games;
            }
            ViewKind::Games if has_details => d.view = ViewKind::Details,
            ViewKind::Home => d.view = ViewKind::Games,
            _ => {}
        }
    }
    if back {
        d.view = match d.view {
            ViewKind::Details => ViewKind::Games,
            ViewKind::Games => ViewKind::Collections,
            v => v,
        };
    }
    let _ = st;
}

/// Take a picture of the preview for the theme's preview pictures (`previews/<view>.png`).
fn capture(ctx: &egui::Context, d: &mut CreatorDlg, screen: Rect) {
    let Some(doc) = &d.doc else { return };
    let dir = doc.dir.clone();
    let name = match d.view {
        ViewKind::Collections => "collections",
        ViewKind::Games => "games",
        ViewKind::Details => "details",
        ViewKind::Home => "home",
    };
    let ppp = ctx.pixels_per_point();
    let (tx, rx) = crossbeam_channel::bounded(1);
    let c2 = ctx.clone();
    ctx.request_screenshot(move |img| {
        let name = name.to_string();
        let _ = std::thread::Builder::new().name("theme-preview-picture".into()).spawn(move || {
            let r = save_capture(&img, screen, ppp, &dir, &name).map_err(|e| format!("{e:#}"));
            let _ = tx.send(super::Done::Captured(r));
            c2.request_repaint();
        });
    });
    ctx.request_repaint();
    d.jobs.push(super::Job { rx, label: "Taking a picture…".into() });
}

fn save_capture(
    img: &egui::ColorImage,
    screen: Rect,
    ppp: f32,
    dir: &std::path::Path,
    name: &str,
) -> anyhow::Result<String> {
    let [w, h] = img.size;
    let x0 = ((screen.min.x * ppp).round().max(0.0) as usize).min(w);
    let y0 = ((screen.min.y * ppp).round().max(0.0) as usize).min(h);
    let x1 = ((screen.max.x * ppp).round() as usize).clamp(x0, w);
    let y1 = ((screen.max.y * ppp).round() as usize).clamp(y0, h);
    anyhow::ensure!(x1 > x0 + 10 && y1 > y0 + 10, "the preview isn't on screen");
    let mut buf = image::RgbaImage::new((x1 - x0) as u32, (y1 - y0) as u32);
    for y in y0..y1 {
        for x in x0..x1 {
            let c = img.pixels[y * w + x].to_srgba_unmultiplied();
            buf.put_pixel((x - x0) as u32, (y - y0) as u32, image::Rgba([c[0], c[1], c[2], 255]));
        }
    }
    // At most 1280 wide: a catalogue picture, not a wallpaper.
    let out = if buf.width() > 1280 {
        let nh = (buf.height() as f32 * 1280.0 / buf.width() as f32).round() as u32;
        image::imageops::resize(&buf, 1280, nh.max(1), image::imageops::FilterType::Triangle)
    } else {
        buf
    };
    let rel = format!("previews/{name}.png");
    let mut bytes = std::io::Cursor::new(Vec::new());
    out.write_to(&mut bytes, image::ImageFormat::Png)?;
    crate::fsutil::write_atomic(&dir.join(&rel), bytes.get_ref())?;
    Ok(rel)
}

/// Small live pictures of themes (the start screen's templates): each theme's Games screen
/// with the user's own library, drawn by the real renderer.
#[derive(Default)]
pub struct Thumbs {
    data: Option<Data>,
    items: HashMap<String, (Arc<Theme>, Renderer)>,
}

impl Thumbs {
    /// Load `file` (from `dir`) under `id` once.
    pub fn ensure(
        &mut self,
        id: &str,
        load: impl FnOnce() -> Option<(crate::theme::ThemeFile, Option<std::path::PathBuf>)>,
    ) {
        if self.items.contains_key(id) {
            return;
        }
        if let Some((file, dir)) = load() {
            let t = Arc::new(Theme { key: format!("theme-creator-thumb:{id}"), file, dir });
            self.items.insert(id.to_string(), (t, Renderer::new()));
        }
    }

    /// The themes, for registering their fonts.
    pub fn themes(&self) -> Vec<Arc<Theme>> {
        self.items.values().map(|(t, _)| t.clone()).collect()
    }

    /// Draw theme `id`'s Games screen into `rect`.
    pub fn draw(&mut self, ui: &mut Ui, st: &mut AppState, id: &str, rect: Rect) {
        if self.data.as_ref().is_none_or(|d| d.generation != st.library.generation) {
            self.data = Some(build_data(&st.library));
        }
        let (Some(data), Some((theme, renderer))) = (self.data.as_ref(), self.items.get_mut(id)) else { return };
        // The collection with the most games (a full screen), else the first.
        let ci = (0..data.colls.len()).max_by_key(|&i| data.games[i].len().min(40)).unwrap_or(0);
        let Some(coll) = data.colls.get(ci) else { return };
        let list = &data.games[ci];
        let Some(prepared) = renderer.prepare(theme, ViewKind::Games, Some(&coll.collection), None) else { return };
        let hints = sample_hints();
        let scene = Scene {
            kind: ViewKind::Games,
            game: list.first().map(|&i| &st.library.games[i]),
            collection: Some(coll),
            items: Items::Games { games: &st.library.games, indices: list },
            selected: 0,
            list_id: 1,
            hints: &hints,
            prompts: st.prompts(),
            opacity: 1.0,
            interactive: false,
            extra: SceneExtra { games: &st.library.games, ..Default::default() },
        };
        renderer.opts = ViewOpts { reduce_motion: true, videos: false, ..ViewOpts::default() };
        // Drawn as a 1280-point screen scaled down: the theme's proportions, small.
        let virt = Rect::from_min_size(rect.min, vec2(1280.0, 1280.0 * rect.height() / rect.width().max(1.0)));
        let k = rect.width() / virt.width();
        let t = egui::emath::TSTransform::new(rect.min.to_vec2() * (1.0 - k), k);
        ui.with_visual_transform(t, |ui| {
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(virt));
            child.set_clip_rect(virt);
            renderer.draw(&mut child, virt, theme, &prepared, &st.fonts, &mut st.images, &scene);
        });
    }
}
