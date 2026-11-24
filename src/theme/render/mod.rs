//! Controller-mode renderer: draws a theme `View` into a rect with data bindings.
//!
//! API (used by ui/controller):
//!   * `Renderer::prepare(theme, kind, collection, variant)` → `Arc<Prepared>`: the view with the
//!     variant and per-collection overrides merged in, z-sorted. Cached per (collection, view)
//!     until the theme or variant changes — never rebuilt per frame.
//!   * `Prepared::focus_info(screen)`: what kind of focusable element the view has (grid columns,
//!     page size) so the UI can map navigation.
//!   * `Renderer::draw(ui, screen, theme, &prepared, fonts, images, &scene)` → `Output`
//!     (clicked/hovered item, wheel steps, whether something is animating).
//!
//! Per-frame work is proportional to the number of *visible* items. Smooth scrolling, image
//! cross-fades and text auto-scroll request repaints only while they move.

pub mod bind;
pub mod draw;
mod elements;
pub mod gradient;
mod items;
mod lists;
mod motion;
mod prefetch;
mod prepare;
mod rows;
mod stack;
mod video;

pub use bind::{Binder, CollectionItem};
pub use rows::{RailScene, RowScene, SceneExtra, pick_rows, rail_entries};
// Public: merge_table, merge_element, prepare_view.
pub use prepare::*;

use crate::images::{Adjust, ImageCache, ImgSize};
use crate::library::Collection;
use crate::model::{ControllerSupport, Game};
use crate::theme::fonts::FontRegistry;
use crate::theme::*;
use draw::*;
use egui::{Color32, FontId, Painter, Pos2, Rect, Stroke, StrokeKind, TextureHandle, Vec2, pos2, vec2};
use elements::*;
use lists::*;
use motion::*;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The focusable items a view shows.
#[derive(Clone, Copy)]
pub enum Items<'a> {
    Collections(&'a [CollectionItem]),
    Games { games: &'a [Game], indices: &'a [usize] },
}

impl<'a> Items<'a> {
    pub fn len(&self) -> usize {
        match self {
            Items::Collections(c) => c.len(),
            Items::Games { indices, .. } => indices.len(),
        }
    }
    fn binder(&self, i: usize, base: Binder<'a>) -> Binder<'a> {
        match self {
            Items::Collections(c) => base.with_collection(c.get(i)),
            Items::Games { games, indices } => base.with_game(indices.get(i).and_then(|&gi| games.get(gi))),
        }
    }
}

/// Everything a view needs for one frame.
pub struct Scene<'a> {
    pub kind: ViewKind,
    /// Selected game (games view) or the game shown (details).
    pub game: Option<&'a Game>,
    /// Current collection (games view) or selected collection (collections view).
    pub collection: Option<&'a CollectionItem>,
    pub items: Items<'a>,
    pub selected: usize,
    /// Changes whenever the item list is replaced (other collection, search, sort): the
    /// focused element then jumps to the selection instead of animating through the list.
    pub list_id: u64,
    pub hints: &'a [Hint],
    /// Which controller's labels the hints show.
    pub prompts: crate::input::Prompts,
    /// Whole-view opacity (screen transitions).
    pub opacity: f32,
    /// Mouse clicks / wheel are reported (false while an overlay is open).
    pub interactive: bool,
    /// Navigation rail and cover rows (Playnite Fullscreen-style views); empty otherwise.
    pub extra: SceneExtra<'a>,
}

#[derive(Clone, Debug, Default)]
pub struct Output {
    pub animating: bool,
    /// Item under the mouse pointer.
    pub hovered: Option<usize>,
    /// Item clicked this frame.
    pub clicked: Option<usize>,
    /// Mouse wheel over the focused element, in items (positive = forward).
    pub scroll_steps: i32,
    /// Screen rect of the focused element.
    pub focus_rect: Option<Rect>,
    /// `rows` element: the (row, item) clicked this frame.
    pub row_clicked: Option<(usize, usize)>,
    /// `rail` element: the section (index into `RailScene::shown`) under the pointer / clicked.
    pub rail_hovered: Option<usize>,
    pub rail_clicked: Option<usize>,
}

/// A view ready to draw: overrides merged, elements in draw order.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub view: View,
    /// Element indices sorted by z (stable).
    pub order: Vec<usize>,
    pub focus: Option<usize>,
    /// Collection display name from the theme's override.
    pub display_name: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FocusKind {
    Carousel { vertical: bool },
    Grid { columns: usize },
    List,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusInfo {
    pub kind: FocusKind,
    /// Items per page (PageUp/PageDown).
    pub page: usize,
}

#[derive(Default)]
struct FadeState {
    cur: Option<PathBuf>,
    prev: Option<PathBuf>,
    /// When `cur` became drawable (fade start).
    ready_at: Option<f64>,
}

impl FadeState {
    /// Cross-fade bookkeeping for a frame that shows `chosen` (`loaded`: its texture is
    /// ready): the picture fading out, how far the fade is (0..1), and whether the new
    /// picture is ready (until then nothing moves; the image cache repaints when it is).
    fn step(
        &mut self,
        chosen: &Option<PathBuf>,
        loaded: bool,
        now: f64,
        fade_secs: f64,
    ) -> (Option<PathBuf>, f32, bool) {
        if self.cur != *chosen {
            // Keep the old picture only if it was fully shown or fading in.
            self.prev = if self.ready_at.is_some() { self.cur.take() } else { self.prev.take() };
            self.cur.clone_from(chosen);
            self.ready_at = None;
        }
        if self.ready_at.is_none() && (loaded || chosen.is_none()) {
            self.ready_at = Some(now);
        }
        let t = match self.ready_at {
            Some(r) if fade_secs > 0.0 => ((now - r) / fade_secs).clamp(0.0, 1.0) as f32,
            Some(_) => 1.0,
            None => 0.0,
        };
        if t >= 1.0 {
            self.prev = None;
        }
        (self.prev.clone(), t, self.ready_at.is_some())
    }
}

#[derive(Default)]
struct ElemState {
    list_id: u64,
    init: bool,
    /// Carousel: animated index. Grid/list: animated top row.
    pos: f32,
    /// List: animated selector row.
    sel: f32,
    /// Grid: target top row.
    top: f32,
    fade: FadeState,
    /// (text hash, start time) for auto-scrolling text.
    text: Option<(u64, f64)>,
    /// Lists with a `focus` transition: the focus moving between items.
    focus: FocusAnim,
    /// Carousel with a `focus` transition: the scroll's start value, target and start time.
    tween: Option<(f32, f32, f64)>,
}

/// Move `cur` towards `target` with exponential smoothing. Returns true while still moving.
fn approach(cur: &mut f32, target: f32, dt: f32, rate: f32) -> bool {
    let d = target - *cur;
    if d.abs() < 0.0015 {
        *cur = target;
        return false;
    }
    *cur += d * (1.0 - (-dt * rate).exp());
    true
}

/// Auto-scrolling text runs this many cycles, then rests at the top (no idle frames forever).
const SCROLL_CYCLES: f64 = 3.0;

pub struct Renderer {
    sig: Option<(String, usize, Option<String>)>,
    prepared: HashMap<(ViewKind, String), Arc<Prepared>>,
    exists: HashMap<PathBuf, bool>,
    states: HashMap<(ViewKind, usize), ElemState>,
    /// `rows` elements: horizontal scroll per row (keyed by the row's collection).
    row_states: HashMap<(ViewKind, usize, u64), ElemState>,
    scroll_acc: f32,
    candidates: Vec<String>,
    /// Pictures of the items next to the selection (prefetch.rs), resolved once per selection.
    neighbours: Option<(prefetch::NeighbourKey, Vec<(PathBuf, ImgSize)>)>,
    /// … and of the screen A opens next (`prefetch_view`).
    ahead: Option<(prefetch::NeighbourKey, Vec<(PathBuf, ImgSize)>)>,
    /// Size class a carousel / grid loaded its items at when last drawn, per (view, element).
    item_sizes: HashMap<(ViewKind, usize), ImgSize>,
    /// The user's controller "Look" switches (Settings → Visuals), applied over every theme.
    pub opts: ViewOpts,
    /// The theme's layout for the screen's shape (`ThemeFile::aspect_key`), None = its own.
    aspect_key: Option<String>,
    /// Theme videos playing in this renderer (video.rs).
    videos: crate::media::video::Videos,
    /// Gradient meshes (gradient.rs), reused while their element doesn't change.
    gradients: gradient::GradientCache,
    /// Appear / disappear transitions per element (motion.rs).
    fx_states: HashMap<(ViewKind, usize), FxState>,
}

/// The user's look switches. All on = the theme exactly as designed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewOpts {
    /// Off: blurred pictures are drawn sharp.
    pub blur_background: bool,
    /// Off: full-screen background pictures are drawn at full strength (scrims stay, so text
    /// still reads).
    pub darken_background: bool,
    /// Off: captions over covers are hidden (games without a cover still show their name).
    pub show_titles: bool,
    /// On: covers of games that aren't installed are dimmed.
    pub dim_not_installed: bool,
    /// Off: the theme's clock is hidden.
    pub show_clock: bool,
    /// Off: the theme's battery level is hidden.
    pub show_battery: bool,
    /// Settings → Visuals → Text size: every text the theme draws is this much bigger.
    pub text_scale: f32,
    /// Theme videos play (Settings → Visuals; the controller turns it off while a game runs).
    /// Off: their still pictures show.
    pub videos: bool,
    /// Theme transitions jump to their end state (`[controller] reduce_motion`, else the
    /// desktop's "no animations").
    pub reduce_motion: bool,
}

impl Default for ViewOpts {
    fn default() -> Self {
        ViewOpts {
            blur_background: true,
            darken_background: true,
            show_titles: true,
            dim_not_installed: true,
            show_clock: true,
            show_battery: true,
            text_scale: 1.0,
            videos: true,
            reduce_motion: false,
        }
    }
}

impl ViewOpts {
    pub fn from_config(c: &crate::config::ControllerConfig) -> ViewOpts {
        ViewOpts {
            blur_background: c.blur_background,
            darken_background: c.darken_background,
            show_titles: c.show_titles,
            dim_not_installed: c.darken_not_installed,
            show_clock: c.show_clock,
            show_battery: c.show_battery,
            text_scale: c.text_size.factor(),
            videos: c.theme_videos,
            reduce_motion: c.reduce_motion.unwrap_or_else(desktop_reduces_motion),
        }
    }
}

/// A text that shows the battery level.
fn is_battery_text(t: &str) -> bool {
    t.contains("system.battery") || t.contains("system.charging")
}

impl Default for Renderer {
    fn default() -> Self {
        Renderer::new()
    }
}

struct Frame<'a, 'b> {
    painter: Painter,
    /// What positions and sizes are fractions of: the view's frame (`view_frame`), or an item's
    /// box while its template is drawn.
    screen: Rect,
    /// All of the screen the view is painted on.
    bounds: Rect,
    /// A `viewbox` view's frame when it is smaller than `bounds` (the spare room at its edges
    /// gets the backgrounds that span it, `bleed`).
    letterbox: Option<Rect>,
    theme: &'a Theme,
    fonts: &'a FontRegistry,
    images: &'a mut ImageCache,
    scene: &'a Scene<'b>,
    base: Binder<'b>,
    now: f64,
    dt: f32,
    ppp: f32,
    out: Output,
    hits: Vec<(usize, Rect)>,
    row_hits: Vec<((usize, usize), Rect)>,
    rail_hits: Vec<(usize, Rect)>,
    /// The view's background colour (what glyph cut-outs should show).
    view_bg: Option<Color>,
    /// Text size factor (`ViewOpts::text_scale`): font sizes are `font_size * sh() * ts`.
    ts: f32,
    /// egui's frame number (transitions notice an element that wasn't drawn last frame).
    frame: u64,
    /// The buttons the view's help bars list between them; None when one bar shows every hint.
    bar_buttons: Option<Vec<Button>>,
    /// A help bar listing only some buttons was drawn this frame (`Hint::always`).
    bar_drawn: bool,
}

/// The buttons a view's help bars list between them (theme prompts aside); None when a bar
/// shows every hint of the screen.
fn listed_buttons(view: &View) -> Option<Vec<Button>> {
    let mut out = vec![];
    let mut all = false;
    for el in &view.elements {
        el.for_each_nested(&mut |e| {
            if let Element::HelpBar { buttons, glyphs: false, .. } = e {
                all |= buttons.is_empty();
                out.extend(buttons.iter().filter_map(|b| button_named(b)));
            }
        });
    }
    (!all).then_some(out)
}

/// The part of `screen` a view is laid out in: all of it, or for a `viewbox` view the largest
/// box of the design's shape, centred (one scale for both directions, like WPF's Viewbox).
pub fn view_frame(view: &View, screen: Rect) -> Rect {
    let Some([w, h]) = view.viewbox.filter(|[w, h]| *w > 0.0 && *h > 0.0) else {
        return screen;
    };
    let s = (screen.width() / w).min(screen.height() / h);
    if !(s.is_finite() && s > 0.0) {
        return screen;
    }
    Rect::from_center_size(screen.center(), vec2(w * s, h * s))
}

impl<'a, 'b> Frame<'a, 'b> {
    /// `r` extended into the spare room around a `viewbox` view on the sides it reaches, when
    /// it spans the frame across (a full-width band, a full-height column, the background): a
    /// picture or a fill that fills the theme's screen fills the real one too. Corner pieces
    /// and anything inside an item keep their place.
    fn bleed(&self, r: Rect) -> Rect {
        let Some(fr) = self.letterbox.filter(|l| *l == self.screen) else {
            return r;
        };
        let b = self.bounds;
        let tol = 1.0;
        let (left, right) = (r.left() <= fr.left() + tol, r.right() >= fr.right() - tol);
        let (top, bottom) = (r.top() <= fr.top() + tol, r.bottom() >= fr.bottom() - tol);
        if !(left && right || top && bottom) {
            return r;
        }
        let mut out = r;
        if top {
            out.min.y = out.min.y.min(b.top());
        }
        if bottom {
            out.max.y = out.max.y.max(b.bottom());
        }
        if left {
            out.min.x = out.min.x.min(b.left());
        }
        if right {
            out.max.x = out.max.x.max(b.right());
        }
        out
    }

    fn sh(&self) -> f32 {
        self.screen.height()
    }
    fn sw(&self) -> f32 {
        self.screen.width()
    }
}

/// Item drawing parameters shared by carousel and grid.
/// Image extras (`Element::Image`): a tint that changes across the picture and the text drawn
/// when no picture exists.
#[derive(Clone, Copy, Default)]
pub(super) struct ImageExtra<'e> {
    pub tint_end: Option<Color>,
    pub fallback_text: Option<&'e str>,
    pub fallback_style: Option<&'e TextStyle>,
}

#[derive(Clone, Copy)]
struct ItemSpec<'e> {
    image: &'e str,
    /// Multiply colour for item pictures.
    tint: Option<Color32>,
    fallbacks: &'e [String],
    fit: Fit,
    radius: f32,
    fill: Option<Color32>,
    text: &'e str,
    subtext: Option<&'e str>,
    caption: Option<&'e str>,
    style: &'e TextStyle,
    border: Option<Color32>,
    /// Reference height for text scaling (unscaled item height).
    base_h: f32,
    size: ImgSize,
    /// Mark games with full controller support (`Common::controller_badge`).
    badge: bool,
    /// Mark favourites (`Common::favorite_badge`).
    fav_badge: bool,
    /// The name is drawn under the item (grid `show_labels`): a game without a picture shows
    /// its console's outline instead of its name a second time.
    labelled: bool,
    /// Focus border / glow, not-installed scrim and caption strip (`Common` extras).
    look: FocusLook<'e>,
    /// Elements drawn over each item (`Common::item_elements`) and the list's state key.
    templates: &'e [Element],
    key: usize,
    /// Colour adjustment of the item pictures (`Common::brightness` / `saturation`).
    adjust: Adjust,
}

/// An item's focus look and decorations, in pixels (`Common::selected_*`,
/// `uninstalled_scrim`, `caption_background`). None = Lodestar's defaults.
#[derive(Clone, Copy, Default)]
struct FocusLook<'e> {
    border_width: Option<f32>,
    radius: Option<f32>,
    glow: Option<(Color32, f32)>,
    scrim: Option<Color32>,
    caption_bg: Option<&'e Brush>,
}

impl<'e> FocusLook<'e> {
    fn of(c: &'e Common, sh: f32) -> FocusLook<'e> {
        let px = |v: f32| (v.is_finite() && v >= 0.0).then_some(v * sh);
        FocusLook {
            border_width: c.selected_border_width.and_then(px),
            radius: c.selected_radius.and_then(px),
            glow: c.selected_glow.map(|g| (g.color.to_egui(), (g.blur.max(0.0) * sh).clamp(0.0, 255.0))),
            scrim: c.uninstalled_scrim.map(|c| c.to_egui()),
            caption_bg: c.caption_background.as_ref(),
        }
    }
}

impl Renderer {
    pub fn new() -> Renderer {
        Renderer {
            sig: None,
            prepared: HashMap::new(),
            exists: HashMap::new(),
            states: HashMap::new(),
            row_states: HashMap::new(),
            scroll_acc: 0.0,
            candidates: Vec::new(),
            neighbours: None,
            ahead: None,
            item_sizes: HashMap::new(),
            opts: ViewOpts::default(),
            aspect_key: None,
            videos: Default::default(),
            gradients: Default::default(),
            fx_states: HashMap::new(),
        }
    }

    fn check_sig(&mut self, theme: &Theme, variant: Option<&str>) {
        let sig = (theme.key.clone(), theme as *const Theme as usize, variant.map(str::to_string));
        if self.sig.as_ref() != Some(&sig) {
            self.sig = Some(sig);
            self.prepared.clear();
            self.item_sizes.clear();
            self.states.clear();
            self.row_states.clear();
            self.fx_states.clear();
            self.videos.stop_all();
        }
    }

    /// The view for (kind, collection), with overrides merged. None when the theme has no such
    /// view (only `details` is optional).
    pub fn prepare(
        &mut self,
        theme: &Theme,
        kind: ViewKind,
        coll: Option<&Collection>,
        variant: Option<&str>,
    ) -> Option<Arc<Prepared>> {
        self.check_sig(theme, variant);
        let key = (kind, coll.map(Collection::key).unwrap_or_default());
        if let Some(p) = self.prepared.get(&key) {
            return Some(p.clone());
        }
        let p = Arc::new(prepare_view_for(&theme.file, kind, coll, variant, self.aspect_key.as_deref())?);
        self.prepared.insert(key, p.clone());
        Some(p)
    }

    /// Forget animation state of a view (next draw snaps to the selection).
    pub fn reset_view(&mut self, kind: ViewKind) {
        self.states.retain(|(k, _), _| *k != kind);
        self.row_states.retain(|(k, _, _), _| *k != kind);
        self.fx_states.retain(|(k, _), _| *k != kind);
    }

    fn exists(&mut self, p: &Path) -> bool {
        if let Some(&e) = self.exists.get(p) {
            return e;
        }
        if self.exists.len() > 50_000 {
            self.exists.clear();
        }
        let e = p.is_file();
        self.exists.insert(p.to_path_buf(), e);
        e
    }

    fn to_path(theme: &Theme, s: &str) -> Option<PathBuf> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if s.starts_with('/') {
            return Some(PathBuf::from(s));
        }
        if let Some(home) = s.strip_prefix("~/") {
            return dirs::home_dir().map(|h| h.join(home));
        }
        theme.resolve(s)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        ui: &mut egui::Ui,
        screen: Rect,
        theme: &Theme,
        prepared: &Prepared,
        fonts: &FontRegistry,
        images: &mut ImageCache,
        scene: &Scene,
    ) -> Output {
        let (now, dt, ppp) = ui.input(|i| (i.time, i.stable_dt.clamp(1.0 / 240.0, 1.0 / 20.0), i.pixels_per_point));
        // A theme with layouts for other screen shapes: follow the screen (the views prepared
        // for the old shape are dropped and the next frame uses the new layout).
        if !theme.file.aspects.is_empty() && screen.height() > 0.0 {
            let key = theme.file.aspect_key(screen.width() / screen.height()).map(str::to_string);
            if key != self.aspect_key {
                self.aspect_key = key;
                self.prepared.clear();
                ui.ctx().request_repaint();
            }
        }
        self.gradients.next_frame();
        let frame = view_frame(&prepared.view, screen);
        let mut painter = ui.painter_at(screen);
        painter.multiply_opacity(scene.opacity.clamp(0.0, 1.0));
        let base = Binder {
            game: scene.game,
            collection: scene.collection,
            theme_dir: theme.dir.as_deref(),
            list: Some((scene.selected, scene.items.len())),
            section: scene.extra.section,
            focused: false,
            page: scene.kind == ViewKind::Details,
            collection_art: (!theme.file.collection_art.is_empty()).then_some(&theme.file.collection_art),
        };
        let mut f = Frame {
            painter,
            screen: frame,
            bounds: screen,
            letterbox: (frame != screen).then_some(frame),
            theme,
            fonts,
            images,
            scene,
            base,
            now,
            dt,
            ppp,
            out: Output::default(),
            hits: Vec::new(),
            row_hits: Vec::new(),
            rail_hits: Vec::new(),
            view_bg: prepared.view.background,
            ts: self.opts.text_scale.clamp(0.5, 2.0),
            frame: ui.ctx().cumulative_frame_nr(),
            bar_buttons: listed_buttons(&prepared.view),
            bar_drawn: false,
        };
        if let Some(bg) = prepared.view.background {
            f.painter.rect_filled(screen, 0.0, bg.to_egui());
        }
        let rail_focused = scene.extra.rail.is_some_and(|r| r.focused);
        for &i in &prepared.order {
            let el = &prepared.view.elements[i];
            let visible = f.base.visible(el.common());
            self.draw_fx(&mut f, i, el, prepared.focus == Some(i) && !rail_focused, visible);
        }
        if self.fx_states.len() > 4096 {
            self.fx_states.clear();
        }
        self.prefetch_neighbours(&mut f, prepared);
        // Mouse
        if scene.interactive {
            let (hover, clicked, wheel) =
                ui.input(|i| (i.pointer.hover_pos(), i.pointer.primary_clicked(), i.smooth_scroll_delta));
            if let Some(p) = hover {
                // Last drawn wins (selected items are drawn last, on top).
                f.out.hovered = f.hits.iter().rev().find(|(_, r)| r.contains(p)).map(|(i, _)| *i);
                if clicked {
                    f.out.clicked = f.out.hovered;
                }
                f.out.rail_hovered = f.rail_hits.iter().rev().find(|(_, r)| r.contains(p)).map(|(i, _)| *i);
                if clicked {
                    f.out.rail_clicked = f.out.rail_hovered;
                    if f.out.rail_clicked.is_none() {
                        f.out.row_clicked = f.row_hits.iter().rev().find(|(_, r)| r.contains(p)).map(|(i, _)| *i);
                    }
                }
                if let Some(r) = f.out.focus_rect
                    && r.contains(p)
                {
                    let d = if wheel.y.abs() >= wheel.x.abs() { -wheel.y } else { -wheel.x };
                    self.scroll_acc += d;
                    let step = 48.0;
                    let steps = (self.scroll_acc / step).trunc();
                    self.scroll_acc -= steps * step;
                    f.out.scroll_steps = steps as i32;
                }
            }
        }
        if f.out.animating {
            ui.ctx().request_repaint();
        }
        f.out
    }

    fn state(&mut self, kind: ViewKind, i: usize) -> &mut ElemState {
        self.states.entry((kind, i)).or_default()
    }

    /// Element box. A zero size component is "auto": from the aspect ratio of `natural`
    /// (images, ratings) …
    fn elem_rect(f: &Frame, c: &Common, natural: Vec2) -> Rect {
        Self::elem_rect_mode(f, c, natural, true)
    }

    /// … or the natural extent itself (text).
    fn text_rect(f: &Frame, c: &Common, natural: Vec2) -> Rect {
        Self::elem_rect_mode(f, c, natural, false)
    }

    fn elem_rect_mode(f: &Frame, c: &Common, natural: Vec2, keep_aspect: bool) -> Rect {
        let (sw, sh) = (f.sw(), f.sh());
        let mut w = c.size[0] * sw;
        let mut h = c.size[1] * sh;
        if w <= 0.0 && h <= 0.0 {
            w = natural.x;
            h = natural.y;
        } else if w <= 0.0 {
            w = if !keep_aspect {
                natural.x
            } else if natural.y > 0.0 {
                h * natural.x / natural.y
            } else {
                0.0
            };
        } else if h <= 0.0 {
            h = if !keep_aspect {
                natural.y
            } else if natural.x > 0.0 {
                w * natural.y / natural.x
            } else {
                0.0
            };
        }
        let min = f.screen.min + vec2(c.pos[0] * sw, c.pos[1] * sh) - vec2(c.origin[0] * w, c.origin[1] * h);
        Rect::from_min_size(min, vec2(w.max(0.0), h.max(0.0)))
    }

    fn draw_element(&mut self, f: &mut Frame, idx: usize, el: &Element, focused: bool) {
        let kind = f.scene.kind;
        match el {
            Element::Image {
                common,
                path,
                fallbacks,
                fit,
                tint,
                tile,
                corner_radius,
                gradient,
                tint_end,
                fallback_text,
                fallback_style,
                video,
            } => {
                if let Some(v) = video
                    && self.draw_video(f, common, v, *fit, *tint, *corner_radius)
                {
                    return;
                }
                self.draw_image(
                    f,
                    idx,
                    common,
                    path,
                    fallbacks,
                    *fit,
                    *tint,
                    *tile,
                    *corner_radius,
                    *gradient,
                    ImageExtra {
                        tint_end: *tint_end,
                        fallback_text: fallback_text.as_deref(),
                        fallback_style: fallback_style.as_ref(),
                    },
                );
            }
            Element::Text { common, text, style, background, wrap, max_lines, scroll } => {
                if !self.opts.show_battery && is_battery_text(text) {
                    return;
                }
                if common.chips {
                    // Resolve each part on its own so an empty binding drops its chip.
                    let parts = stack::chip_parts(&f.base, text);
                    if parts.is_empty() {
                        return;
                    }
                    let sh = f.sh();
                    let pad = common.padding.map(|p| vec2(p[0] * sh, p[1] * sh)).unwrap_or(Vec2::ZERO);
                    let family = f.fonts.family(f.theme, style.font.as_deref());
                    let color = fade(style.color.to_egui(), common.opacity);
                    let px = (style.font_size * sh * f.ts).max(4.0);
                    self.draw_chips(f, common, &parts, style, *background, px, pad, family, color);
                    return;
                }
                let t = f.base.resolve(text, false);
                if t.trim().is_empty() {
                    return;
                }
                // Keep leading blank lines: multi-line label/value columns must stay aligned.
                let t = t.trim_end();
                self.draw_text(f, (kind, idx), common, t, style, *background, *wrap, *max_lines, *scroll);
            }
            Element::Rect { common, color, corner_radius, border_color, border_width, gradient, fill, track } => {
                if let Some(fill) = fill {
                    // A progress bar: nothing when the value isn't known.
                    let v = f.base.resolve(fill, true);
                    let Ok(v) = v.trim().parse::<f32>() else { return };
                    let own = Self::elem_rect(f, common, Vec2::ZERO);
                    let a = common.opacity;
                    let rad = corner_radius * f.sh();
                    if let Some(t) = track {
                        f.painter.rect_filled(own, cr(rad), fade(t.to_egui(), a));
                    }
                    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
                    if v > 0.0 {
                        let w = (own.width() * v).max(own.height().min(own.width()));
                        let bar = Rect::from_min_size(own.min, vec2(w, own.height()));
                        f.painter.rect_filled(bar, cr(rad), fade(color.to_egui(), a));
                    }
                    return;
                }
                // The fill reaches into the spare room around a `viewbox` view; a border stays
                // where the theme drew it.
                let own = Self::elem_rect(f, common, Vec2::ZERO);
                let r = f.bleed(own);
                let a = common.opacity;
                let rad = corner_radius * f.sh();
                match (gradient, common.gradient_spec(*gradient)) {
                    (_, Some(g)) => self.gradients.paint(&f.painter, r, [rad; 4], &g, a),
                    (Some([a0, b0]), None) => gradient_rect(
                        &f.painter,
                        r,
                        rad,
                        [fade(a0.to_egui(), a), fade(b0.to_egui(), a)],
                        common.gradient_horizontal,
                    ),
                    (None, None) => {
                        let mut shape = egui::epaint::RectShape::filled(r, cr(rad), fade(color.to_egui(), a));
                        if common.rotation != 0.0 {
                            shape = shape.with_angle(common.rotation.to_radians());
                        }
                        f.painter.add(shape);
                    }
                }
                if let Some(bc) = border_color {
                    let w = (border_width * f.sh()).max(1.0 / f.ppp);
                    f.painter.rect_stroke(own, cr(rad), Stroke::new(w, fade(bc.to_egui(), a)), StrokeKind::Inside);
                }
            }
            Element::Rating { common, value, filled_path, unfilled_path, color } => {
                let v = f.base.resolve(value, true).trim().parse::<f32>().unwrap_or(0.0).clamp(0.0, 1.0);
                let h = common.size[1] * f.sh();
                let h = if h > 0.0 { h } else { common.size[0] * f.sw() / 5.0 };
                let r = Self::elem_rect(f, common, vec2(h * 5.0, h));
                self.draw_rating(f, r, v, filled_path.as_deref(), unfilled_path.as_deref(), *color, common.opacity);
            }
            Element::DateTime { common, value, style, clock, format, background } => {
                if *clock && !self.opts.show_clock {
                    return;
                }
                let text = if *clock {
                    let now = crate::model::now_unix();
                    let secs_left = 60 - now.rem_euclid(60);
                    f.painter.ctx().request_repaint_after(std::time::Duration::from_secs(secs_left.max(1) as u64));
                    let fmt = if format == "%d %b %Y" { "%H:%M" } else { format.as_str() };
                    bind::format_date(now, fmt)
                } else {
                    let raw = f.base.resolve(value, true);
                    match raw.trim().parse::<i64>() {
                        Ok(ts) if ts > 0 => bind::format_date(ts, format),
                        _ => {
                            // Not a timestamp: show the text form ("Never", "Today" …).
                            f.base.resolve(value, false).trim().to_string()
                        }
                    }
                };
                if !text.is_empty() {
                    self.draw_text(f, (kind, idx), common, &text, style, *background, false, Some(1), false);
                }
            }
            Element::HelpBar { .. } => self.draw_help_bar(f, el),
            Element::Badges { .. } => self.draw_badges(f, el),
            Element::Stack { .. } => self.draw_stack(f, idx, el),
            Element::Rail { .. } => self.draw_rail(f, idx, el),
            Element::Rows { .. } => self.draw_rows(f, idx, el, focused),
            Element::Carousel { .. } => self.draw_carousel(f, idx, el, focused),
            Element::Grid { .. } => self.draw_grid(f, idx, el, focused),
            Element::TextList { .. } => self.draw_textlist(f, idx, el, focused),
        }
    }
}

/// The colour adjustment an element asks for its pictures (`Common::brightness` / `saturation`).
fn picture_adjust(c: &Common) -> Adjust {
    Adjust::new(c.brightness, c.saturation)
}

pub fn luminance(c: Color32) -> f32 {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_stack;
#[cfg(test)]
mod tests_viewbox;
