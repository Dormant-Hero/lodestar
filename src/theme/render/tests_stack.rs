//! Tests: the `stack` container, item templates, visibility conditions, transitions and the
//! item-label word fit.

use super::*;
use crate::library::CollectionKind;
use crate::model::Source;

fn game(id: i64, name: &str) -> Game {
    Game {
        id,
        source: Source::Steam,
        source_id: id.to_string(),
        art: crate::model::Art { cover: Some(format!("/nonexistent/{id}.jpg").into()), ..Default::default() },
        meta: crate::model::Metadata { description: Some("Words and more words. ".repeat(40)), ..Default::default() },
        play_count: id as u32,
        favorite: id % 2 == 0,
        ..Game::blank(name)
    }
}

fn all() -> CollectionItem {
    CollectionItem {
        collection: Collection { kind: CollectionKind::All, name: "All".into() },
        name: "All".into(),
        count: 0,
        art: Default::default(),
    }
}

/// Draws a games view; `frame(t, selected)` runs one egui frame at time `t`.
struct Rig {
    ctx: egui::Context,
    images: ImageCache,
    fonts: FontRegistry,
    theme: Theme,
    r: Renderer,
    games: Vec<Game>,
    coll: CollectionItem,
    screen: Rect,
    _dir: tempfile::TempDir,
}

impl Rig {
    fn new(src: &str, games: Vec<Game>) -> Rig {
        let dir = tempfile::tempdir().unwrap();
        let ctx = egui::Context::default();
        let images = ImageCache::new(&ctx, dir.path().to_path_buf());
        Rig {
            images,
            ctx,
            fonts: FontRegistry::new(),
            theme: Theme { key: "t".into(), file: ThemeFile::parse(src).unwrap(), dir: None },
            r: Renderer::new(),
            games,
            coll: all(),
            screen: Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 500.0)),
            _dir: dir,
        }
    }

    fn frame(&mut self, t: f64, selected: usize) -> egui::FullOutput {
        let p = self.r.prepare(&self.theme, ViewKind::Games, Some(&self.coll.collection), None).unwrap();
        let indices: Vec<usize> = (0..self.games.len()).collect();
        let rows = [RowScene { collection: &self.coll, indices: &indices, selected }];
        let scene = Scene {
            kind: ViewKind::Games,
            game: self.games.get(selected),
            collection: Some(&self.coll),
            items: Items::Games { games: &self.games, indices: &indices },
            selected,
            list_id: 1,
            hints: &[],
            prompts: Default::default(),
            opacity: 1.0,
            interactive: false,
            extra: SceneExtra { games: &self.games, rows: &rows, ..Default::default() },
        };
        let input = egui::RawInput { time: Some(t), screen_rect: Some(self.screen), ..Default::default() };
        let (r, theme, fonts, images, screen) = (&mut self.r, &self.theme, &self.fonts, &mut self.images, self.screen);
        let mut out = self.ctx.run_ui(input, |ui| {
            r.draw(ui, screen, theme, &p, fonts, images, &scene);
        });
        out.textures_delta.clear();
        out
    }
}

fn texts(out: &egui::FullOutput) -> Vec<(String, Rect, &egui::epaint::ClippedShape)> {
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Text(t) => Some((t.galley.job.text.clone(), Rect::from_min_size(t.pos, t.galley.size()), c)),
            _ => None,
        })
        .collect()
}

fn text_rect(out: &egui::FullOutput, starts: &str) -> Option<Rect> {
    texts(out).into_iter().find(|(t, _, _)| t.starts_with(starts)).map(|(_, r, _)| r)
}

fn rects(out: &egui::FullOutput) -> Vec<&egui::epaint::RectShape> {
    out.shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) => Some(r),
            _ => None,
        })
        .collect()
}

/// A text's opacity (the painter fades its colour).
fn alpha(t: &egui::epaint::TextShape) -> f32 {
    t.fallback_color.a() as f32 / 255.0
}

fn repaints(out: &egui::FullOutput) -> bool {
    out.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.repaint_delay.as_secs_f64() < 0.1)
}

const STACK: &str = r##"
[[games.elements]]
type = "stack"
name = "info"
pos = [0.1, 0.1]
size = [0.5, 0]
gap = 0.02
padding = [0.01, 0.01]
background = "#102030"
[[games.elements.children]]
type = "text"
text = "TITLE {game.name}"
style = { font_size = 0.05 }
[[games.elements.children]]
type = "text"
text = "LOGO-ONLY"
visible_if = "{art.logo}"
[[games.elements.children]]
type = "image"
path = "{art.logo}"
size = [0.2, 0.1]
[[games.elements.children]]
type = "rect"
size = [0, 0.004]
color = "#ff0000"
margin = [0.0, 0.01, 0.0, 0.0]
[[games.elements.children]]
type = "stack"
direction = "horizontal"
gap = 0.01
align = "center"
[[games.elements.children.children]]
type = "text"
text = "A{game.playcount}"
[[games.elements.children.children]]
type = "text"
text = "{game.developer}"
[[games.elements.children.children]]
type = "text"
text = "B"
style = { font_size = 0.06 }
[[games.elements.children]]
type = "text"
text = "DESC {game.description}"
wrap = true
max_lines = 3
style = { font_size = 0.03 }

[[games.elements]]
type = "stack"
name = "up"
pos = [0.9, 0.9]
origin = [1.0, 1.0]
justify = "end"
[[games.elements.children]]
type = "text"
text = "UPPER"
[[games.elements.children]]
type = "text"
text = "LOWER"
"##;

#[test]
fn stack_parses_and_round_trips() {
    let file = ThemeFile::parse(STACK).unwrap();
    let Element::Stack { children, gap, direction, align, justify, background, .. } = &file.games.elements[0] else {
        panic!()
    };
    assert_eq!(
        (*gap, *direction, *align, *justify),
        (0.02, Direction::Vertical, StackAlign::Stretch, StackAlign::Start)
    );
    assert_eq!(children.len(), 6);
    assert!(matches!(background, Some(Brush::Solid(_))));
    assert_eq!(children[3].common().margin, Some([0.0, 0.01, 0.0, 0.0]));
    assert!(matches!(&children[4], Element::Stack { direction: Direction::Horizontal, align: StackAlign::Center, .. }));
    let again = ThemeFile::parse(&file.to_toml().unwrap()).unwrap();
    assert_eq!(file, again);
    // Unset new fields stay out of saved themes.
    let plain = ThemeFile::parse("[[games.elements]]\ntype = \"text\"\ntext = \"x\"").unwrap().to_toml().unwrap();
    for k in ["transitions", "margin", "item_elements", "children"] {
        assert!(!plain.contains(k), "{k}");
    }
}

#[test]
fn stack_flows_children_and_skips_hidden_ones() {
    let mut rig = Rig::new(STACK, (0..3).map(|i| game(i, &format!("Game {i}"))).collect());
    let mut out = rig.frame(0.0, 1);
    for t in 1..3 {
        out = rig.frame(t as f64 * 0.1, 1);
    }
    let (sw, sh) = (1000.0, 500.0);
    let pad = 0.01 * sh;
    let gap = 0.02 * sh;
    let title = text_rect(&out, "TITLE").expect("title");
    assert!((title.left() - (0.1 * sw + pad)).abs() < 1.0 && (title.top() - (0.1 * sh + pad)).abs() < 1.0, "{title:?}");
    // The hidden text and the missing logo take no space: the red rule is next, one gap and
    // its 5 px top margin below the title, as wide as the stack's inside.
    assert!(text_rect(&out, "LOGO-ONLY").is_none());
    let rule = rects(&out).into_iter().find(|r| r.fill == Color32::from_rgb(255, 0, 0)).expect("rule").rect;
    assert!((rule.top() - (title.bottom() + gap + 0.01 * sh)).abs() < 1.5, "rule {rule:?} title {title:?}");
    assert!((rule.width() - (0.5 * sw - 2.0 * pad)).abs() < 1.0, "{rule:?}");
    assert!((rule.height() - 2.0).abs() < 0.01);
    // Horizontal row: the empty developer text takes no space (one gap between A and B), and
    // the row is centred across: the smaller A sits lower than the top of the bigger B.
    let a = text_rect(&out, "A1").expect("A");
    let b = texts(&out).into_iter().find(|(t, _, _)| t == "B").expect("B").1;
    assert!((b.left() - (a.right() + 0.01 * sh)).abs() < 1.5, "a {a:?} b {b:?}");
    assert!(a.top() > b.top() + 1.0 && (a.center().y - b.center().y).abs() < 2.0);
    assert!(a.top() >= rule.bottom() + gap - 1.0);
    // The description wraps at the stack's width, at most 3 lines, below the row.
    let desc = text_rect(&out, "DESC").expect("desc");
    assert!(desc.top() >= b.bottom() + gap - 1.0);
    assert!(desc.width() <= 0.5 * sw - 2.0 * pad + 1.0);
    let line = 0.03 * sh * 1.2 * 1.12;
    assert!(desc.height() > line * 2.5 && desc.height() < line * 3.5, "{desc:?}");
    // The background covers everything, padding included.
    let bg = rects(&out).into_iter().find(|r| r.fill == Color32::from_rgb(0x10, 0x20, 0x30)).expect("bg").rect;
    assert!((bg.bottom() - (desc.bottom() + pad)).abs() < 1.5, "bg {bg:?} desc {desc:?}");
    assert!((bg.width() - 0.5 * sw).abs() < 0.5);
    // A stack anchored at its bottom-right corner grows up and left from `pos`.
    let lower = text_rect(&out, "LOWER").unwrap();
    let upper = text_rect(&out, "UPPER").unwrap();
    assert!((lower.bottom() - 0.9 * sh).abs() < 2.0, "{lower:?}");
    assert!(upper.bottom() <= lower.top() + 0.5);
    assert!(lower.right() <= 0.9 * sw + 1.0);
}

/// A row of tabs wider than its `max_width` draws its text smaller until it fits, right
/// to left as well as left to right; one that fits is drawn as it is.
#[test]
fn a_row_wider_than_its_max_width_shrinks_its_text_to_fit() {
    let row = |name: &str, x: f32, y: f32, origin: f32, max: f32| {
        format!(
            "[[games.elements]]\ntype = \"stack\"\ndirection = \"horizontal\"\npos = [{x:?}, {y:?}]\n\
             origin = [{origin:?}, 0.5]\ngap = 0.03\nmax_width = {max:?}\n\
             [[games.elements.children]]\ntype = \"text\"\ntext = \"{name}-All Games\"\n\
             style = {{ font_size = 0.05 }}\n\
             [[games.elements.children]]\ntype = \"text\"\ntext = \"{name}-Continue Playing\"\n\
             style = {{ font_size = 0.05, font = \"bold\" }}\n"
        )
    };
    let src = [row("WIDE", 0.1, 0.2, 0.0, 0.0), row("FIT", 0.1, 0.5, 0.0, 0.3), row("END", 0.9, 0.8, 1.0, 0.3)];
    let mut rig = Rig::new(&src.join("\n"), vec![game(1, "G")]);
    let out = rig.frame(0.0, 0);
    let span = |name: &str| {
        let a = text_rect(&out, &format!("{name}-All")).expect("first");
        let b = text_rect(&out, &format!("{name}-Continue")).expect("second");
        (a, b)
    };
    let (wa, wb) = span("WIDE");
    assert!(wb.right() - wa.left() > 0.3 * 1000.0 + 50.0, "unlimited, it is wider than the others' room");
    let (fa, fb) = span("FIT");
    assert!((fa.left() - 100.0).abs() < 1.0 && fb.right() <= 100.0 + 300.0 + 0.5, "{fa:?} {fb:?}");
    assert!(fb.right() > 100.0 + 300.0 * 0.95, "it uses the room: {fb:?}");
    assert!(fa.height() < wa.height(), "the text is smaller");
    // The gap stays as it is.
    assert!((fb.left() - fa.right() - 0.03 * 500.0).abs() < 1.5, "{fa:?} {fb:?}");
    let (ea, eb) = span("END");
    assert!((eb.right() - 900.0).abs() < 1.5 && ea.left() >= 900.0 - 300.0 - 0.5, "{ea:?} {eb:?}");
    // Room to spare: drawn at its own size.
    let src = row("ROOMY", 0.1, 0.5, 0.0, 0.9);
    let mut rig = Rig::new(&src, vec![game(1, "G")]);
    let out = rig.frame(0.0, 0);
    let roomy = text_rect(&out, "ROOMY-All").expect("roomy");
    assert!((roomy.height() - wa.height()).abs() < 0.5, "{roomy:?} {wa:?}");
}

/// A vertical stack of a fixed height gives a wrapping text the lines that are left.
#[test]
fn stack_text_gets_the_room_left() {
    let src = r##"
[[games.elements]]
type = "stack"
pos = [0.0, 0.0]
size = [0.4, 0.2]
[[games.elements.children]]
type = "text"
text = "HEAD"
style = { font_size = 0.05 }
[[games.elements.children]]
type = "text"
text = "BODY {game.description}"
wrap = true
style = { font_size = 0.03 }
"##;
    let mut rig = Rig::new(src, vec![game(1, "G")]);
    let out = rig.frame(0.0, 0);
    let body = text_rect(&out, "BODY").expect("body");
    assert!(body.bottom() <= 0.2 * 500.0 + 1.0, "{body:?}");
    let line = 0.03 * 500.0 * 1.2 * 1.12;
    assert!(body.height() >= line * 2.0, "{body:?}");
}

#[test]
fn conditions() {
    let g = game(4, "Four");
    let c = all();
    let b = Binder { game: Some(&g), collection: Some(&c), ..Default::default() };
    assert!(b.condition("{game.favorite}"));
    assert!(!b.condition("!{game.favorite}"));
    assert!(!b.condition("{art.logo}"));
    assert!(b.condition("!{art.logo}"));
    assert!(b.condition("{game.playcount} > 3"));
    assert!(!b.condition("{game.playcount} >= 5"));
    assert!(b.condition("{game.playcount} == 4.0"));
    assert!(b.condition("{game.name} == 'four'"));
    assert!(b.condition("{game.name} != \"Five\""));
    assert!(!b.condition("{game.name} < 3"), "text isn't a number");
    assert!(b.condition("{game.favorite} && {game.playcount} > 1"));
    assert!(!b.condition("{art.logo} && {game.favorite}"));
    assert!(b.condition("{art.logo} || {game.favorite}"));
    // Flags against true / false compare as truth values ("" = false).
    assert!(b.condition("{game.favorite} == true"));
    assert!(!b.condition("{game.favorite} == False"));
    assert!(b.condition("{art.logo} == false"));
    assert!(!b.condition("{art.logo} != 'false'"));
    assert!(b.condition("{game.favorite} != {art.logo}"));
    assert!(!b.condition("{item.focused}"));
    let f = Binder { focused: true, ..b };
    assert!(f.condition("{item.focused}"));
    // visible_if / hidden_if use them.
    let common = Common { visible_if: Some("{game.playcount} > 10".into()), ..Default::default() };
    assert!(!b.visible(&common));
    let common = Common { hidden_if: Some("!{game.favorite}".into()), ..Default::default() };
    assert!(b.visible(&common));
}

#[test]
fn item_templates_draw_per_item_with_their_own_bindings() {
    let src = r##"
[[games.elements]]
type = "grid"
pos = [0.0, 0.0]
size = [1.0, 1.0]
columns = 4
item_aspect = 0.5
selected_scale = 1.0
[[games.elements.item_elements]]
type = "text"
text = "FAV {game.name}"
visible_if = "{game.favorite}"
pos = [0.0, 0.0]
style = { font_size = 0.2 }
[[games.elements.item_elements]]
type = "rect"
pos = [0.0, 0.9]
size = [1.0, 0.1]
color = "#00ff00"
visible_if = "{item.focused}"
[[games.elements.item_elements]]
type = "stack"
pos = [0.5, 0.5]
origin = [0.5, 0.5]
[[games.elements.item_elements.children]]
type = "text"
text = "N{game.playcount}"
hidden_if = "{game.playcount} == 0"
"##;
    let games: Vec<Game> = (0..4).map(|i| game(i, &format!("G{i}"))).collect();
    let mut rig = Rig::new(src, games);
    let out = rig.frame(0.0, 1);
    let t = texts(&out);
    let favs: Vec<&String> = t.iter().map(|(s, _, _)| s).filter(|s| s.starts_with("FAV")).collect();
    assert_eq!(favs.len(), 2, "{favs:?}");
    assert!(favs.iter().any(|s| *s == "FAV G0") && favs.iter().any(|s| *s == "FAV G2"));
    // Item-relative: the text of item 2 (third column) starts at its item's left edge.
    let g2 = text_rect(&out, "FAV G2").unwrap();
    // 4 columns over 1000 px with 10 px gaps: 242.5 px items, the third starts at 505.
    assert!((g2.left() - 505.0).abs() < 1.0, "{g2:?}");
    // Exactly one item has the focus bar.
    let green = rects(&out).into_iter().filter(|r| r.fill == Color32::from_rgb(0, 255, 0)).count();
    assert_eq!(green, 1);
    // Stacks work inside templates, with hidden children (play count 0 → nothing).
    let ns: Vec<&String> = t.iter().map(|(s, _, _)| s).filter(|s| s.starts_with('N')).collect();
    assert_eq!(ns.len(), 3, "{ns:?}");
}

const APPEAR: &str = r##"
[[games.elements]]
type = "text"
name = "t"
text = "HELLO"
pos = [0.1, 0.1]
transitions = { appear = { opacity = 0.0, translate = [0.0, 0.1], duration_ms = 1000, easing = "linear" } }
[[games.elements]]
type = "text"
name = "fav"
text = "FAVOURITE"
pos = [0.1, 0.5]
visible_if = "{game.favorite}"
transitions = { appear = { scale = 0.5, duration_ms = 400 }, disappear = { opacity = 0.0, duration_ms = 400 } }
"##;

#[test]
fn appear_and_disappear_animate_then_settle() {
    let mut rig = Rig::new(APPEAR, (0..3).map(|i| game(i, &format!("G{i}"))).collect());
    // Start: fully transparent (not drawn at all).
    let out = rig.frame(10.0, 0);
    assert!(!texts(&out).iter().any(|(t, _, _)| t == "HELLO"));
    assert!(repaints(&out));
    rig.frame(10.001, 0);
    let out = rig.frame(10.5, 0);
    let (_, _, s) = texts(&out).into_iter().find(|(t, _, _)| t == "HELLO").unwrap();
    assert!(matches!(&s.shape, egui::Shape::Text(t) if (alpha(t) - 0.5).abs() < 0.02));
    assert!((s.transform_after_tessellation.translation.y - 25.0).abs() < 0.5);
    let out = rig.frame(11.1, 0);
    let (_, _, s) = texts(&out).into_iter().find(|(t, _, _)| t == "HELLO").unwrap();
    assert_eq!(s.transform_after_tessellation, egui::emath::TSTransform::IDENTITY);
    assert!(matches!(&s.shape, egui::Shape::Text(t) if alpha(t) == 1.0));
    // (egui still reports the frame after a repaint request as wanted.)
    assert!(!repaints(&rig.frame(11.15, 0)), "settled: no more frames");
    // Selecting a game that isn't a favourite: FAVOURITE fades out over 0.4 s, then is gone.
    let out = rig.frame(11.2, 1);
    let (_, _, s) = texts(&out).into_iter().find(|(t, _, _)| t == "FAVOURITE").expect("still drawn while leaving");
    assert!(matches!(&s.shape, egui::Shape::Text(t) if alpha(t) > 0.9));
    let out = rig.frame(11.4, 1);
    let (_, _, s) = texts(&out).into_iter().find(|(t, _, _)| t == "FAVOURITE").unwrap();
    assert!(matches!(&s.shape, egui::Shape::Text(t) if alpha(t) < 0.9 && alpha(t) > 0.05));
    let out = rig.frame(11.7, 1);
    assert!(!texts(&out).iter().any(|(t, _, _)| t == "FAVOURITE"));
    // Back on a favourite: it appears again, growing from half size around its centre.
    let out = rig.frame(12.0, 2);
    let (_, r, s) = texts(&out).into_iter().find(|(t, _, _)| t == "FAVOURITE").unwrap();
    let ts = s.transform_after_tessellation;
    assert!((ts.scaling - 0.5).abs() < 0.01);
    let c = r.center();
    let moved = ts * c;
    assert!((moved - c).length() < 1.0, "scaled around its centre");
}

#[test]
fn reduced_motion_shows_end_states_at_once() {
    let mut rig = Rig::new(APPEAR, vec![game(0, "G0")]);
    rig.r.opts.reduce_motion = true;
    // (egui itself asks for two frames at the start.)
    rig.frame(5.0, 0);
    rig.frame(5.01, 0);
    let out = rig.frame(5.1, 0);
    for (_, _, s) in texts(&out) {
        assert_eq!(s.transform_after_tessellation, egui::emath::TSTransform::IDENTITY);
        assert!(matches!(&s.shape, egui::Shape::Text(t) if alpha(t) == 1.0));
    }
    assert!(!repaints(&out));
    assert!(motion::kde_instant_animations("[General]\nx=1\n[KDE]\nAnimationDurationFactor=0\n"));
    assert!(!motion::kde_instant_animations("[KDE]\nAnimationDurationFactor=0.5\n"));
    assert!(!motion::kde_instant_animations("[Other]\nAnimationDurationFactor=0\n"));
}

#[test]
fn grid_focus_transition_grows_and_fades_the_border() {
    let src = r##"
[[games.elements]]
type = "grid"
pos = [0.0, 0.0]
size = [1.0, 1.0]
columns = 4
item_aspect = 0.5
selected_border = "#ff8800"
selected_glow = { color = "#00000000" }
unselected_opacity = 1.0
transitions = { focus = { scale = 1.2, duration_ms = 1000, easing = "linear" } }
"##;
    let mut rig = Rig::new(src, (0..8).map(|i| game(i, &format!("G{i}"))).collect());
    let orange = |out: &egui::FullOutput| {
        rects(out)
            .into_iter()
            .filter(|r| {
                let [cr, cg, cb, _] = r.stroke.color.to_srgba_unmultiplied();
                r.stroke.width > 0.0 && cr > 200 && (100..170).contains(&cg) && cb < 30
            })
            .map(|r| (r.rect, r.stroke.color.a()))
            .collect::<Vec<_>>()
    };
    // First frame: the focus shows at once, full size.
    rig.frame(0.98, 0);
    rig.frame(0.99, 0);
    let out = rig.frame(1.0, 0);
    let first = orange(&out);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].1, 255);
    let full_w = first[0].0.width();
    assert!(!repaints(&out));
    // Moving right: halfway, both items are half focused (border half visible, half grown).
    rig.frame(2.0, 1);
    let out = rig.frame(2.5, 1);
    let mid = orange(&out);
    assert_eq!(mid.len(), 2, "{mid:?}");
    for (r, a) in &mid {
        assert!((120..=135).contains(a), "alpha {a}");
        assert!(r.width() < full_w - 2.0 && r.width() > full_w / 1.2, "{r:?}");
    }
    assert!(repaints(&out));
    let out = rig.frame(3.1, 1);
    let end = orange(&out);
    assert_eq!(end.len(), 1);
    assert!((end[0].0.width() - full_w).abs() < 0.5 && end[0].0.center().x > first[0].0.center().x + 200.0);
    assert!(!repaints(&rig.frame(3.15, 1)));
}

#[test]
fn whole_words_in_item_labels() {
    // Fits at the readable minimum: as before.
    assert_eq!(whole_word_px(20.0, 10.0, 6.0, 100.0, 200.0), (20.0, false));
    let (px, cut) = whole_word_px(20.0, 10.0, 6.0, 300.0, 200.0);
    assert!(!cut && (px - 20.0 * 200.0 / 300.0 * 0.97).abs() < 1e-4);
    // Below the readable minimum: smaller, down to the floor, never broken.
    let (px, cut) = whole_word_px(20.0, 16.0, 8.0, 300.0, 200.0);
    assert!(!cut && px < 16.0 && px > 12.0);
    // Even the floor is too big: cut with "…".
    let (px, cut) = whole_word_px(20.0, 16.0, 8.0, 1000.0, 200.0);
    assert!(cut && px == 8.0);
    // Monospace stand-in: 10 px per character.
    let m = |w: &str| w.chars().count() as f32 * 10.0;
    assert_eq!(elide_words("Brawlhalla Ultimate", 60.0, m), "Brawl… Ultim…");
    assert_eq!(elide_words("The Sims 4", 60.0, m), "The Sims 4");
}

/// Narrow cards without a cover: long names wrap only between words (the PS5-style strip
/// broke "Brawlhalla" into "Brawlh" / "alla").
#[test]
fn narrow_text_cards_never_break_words() {
    let src = r##"
[[games.elements]]
type = "rows"
pos = [0.0, 0.0]
size = [1.0, 0.5]
rows = ["all"]
item_size = [0.045, 0.13]
spacing = 0.006
title = ""
item_text_style = { font_size = 0.022 }
"##;
    let names = ["Brawlhalla", "Radiolight", "The Sims 4", "Supercalifragilisticexpialidocious Deluxe"];
    let games: Vec<Game> = names.iter().enumerate().map(|(i, n)| game(i as i64, n)).collect();
    let mut rig = Rig::new(src, games);
    rig.screen = Rect::from_min_size(Pos2::ZERO, vec2(1920.0, 1080.0));
    let out = rig.frame(0.0, 3);
    let words: std::collections::HashSet<&str> = names.iter().flat_map(|n| n.split_whitespace()).collect();
    let mut seen = 0;
    for c in &out.shapes {
        let egui::Shape::Text(t) = &c.shape else { continue };
        let text = &t.galley.job.text;
        if !names.iter().any(|n| text.get(..3) == n.get(..3)) {
            continue;
        }
        seen += 1;
        for row in &t.galley.rows {
            let row_text: String = row.row.glyphs.iter().map(|g| g.chr).collect();
            for w in row_text.split_whitespace() {
                assert!(words.contains(w) || w.ends_with('…'), "{:?} broke into {row_text:?}", t.galley.job.text);
            }
        }
    }
    assert_eq!(seen, names.len());
}

/// `transitions.focus` on rows: the focus grows onto the new item and off the old one, then
/// the frames stop.
#[test]
fn rows_focus_transition_moves_between_items() {
    let src = r##"
[[games.elements]]
type = "rows"
pos = [0.0, 0.0]
size = [1.0, 0.8]
rows = ["all"]
item_size = [0.1, 0.3]
title = ""
selected_border = "#ff8800"
selected_glow = { color = "#00000000" }
transitions = { focus = { scale = 1.2, duration_ms = 1000, easing = "linear" } }
"##;
    let mut rig = Rig::new(src, (0..6).map(|i| game(i, &format!("G{i}"))).collect());
    let orange = |out: &egui::FullOutput| {
        rects(out)
            .into_iter()
            .filter(|r| {
                let [cr, cg, cb, _] = r.stroke.color.to_srgba_unmultiplied();
                r.stroke.width > 0.0 && cr > 200 && (100..170).contains(&cg) && cb < 30
            })
            .map(|r| (r.rect, r.stroke.color.a()))
            .collect::<Vec<_>>()
    };
    rig.frame(0.98, 0);
    rig.frame(0.99, 0);
    let out = rig.frame(1.0, 0);
    let first = orange(&out);
    assert_eq!(first.len(), 1, "{first:?}");
    let full_w = first[0].0.width();
    assert!(!repaints(&out));
    rig.frame(2.0, 1);
    let out = rig.frame(2.5, 1);
    let mid = orange(&out);
    assert_eq!(mid.len(), 2, "{mid:?}");
    for (r, a) in &mid {
        assert!((110..=145).contains(a), "alpha {a}");
        assert!(r.width() < full_w - 1.0, "{r:?}");
    }
    assert!(repaints(&out));
    let out = rig.frame(3.1, 1);
    let end = orange(&out);
    assert_eq!(end.len(), 1);
    assert!((end[0].0.width() - full_w).abs() < 0.5 && end[0].0.center().x > first[0].0.center().x);
    assert!(!repaints(&rig.frame(3.15, 1)));
}

/// `transitions.focus` on a carousel: the scroll takes the transition's time and easing.
#[test]
fn carousel_focus_transition_tweens_the_scroll() {
    let src = r##"
[[games.elements]]
type = "carousel"
pos = [0.0, 0.0]
size = [1.0, 0.5]
visible_items = 5
transitions = { focus = { scale = 1.3, duration_ms = 1000, easing = "linear" } }
"##;
    let mut rig = Rig::new(src, (0..8).map(|i| game(i, &format!("Game{i}"))).collect());
    let x_of = |out: &egui::FullOutput, name: &str| text_rect(out, name).map(|r| r.center().x);
    rig.frame(0.98, 0);
    rig.frame(0.99, 0);
    let out = rig.frame(1.0, 0);
    let start = x_of(&out, "Game2").expect("Game2");
    assert!(!repaints(&out));
    rig.frame(2.0, 1);
    let out = rig.frame(2.5, 1);
    let mid = x_of(&out, "Game2").unwrap();
    assert!(repaints(&out));
    let out = rig.frame(3.1, 1);
    let end = x_of(&out, "Game2").unwrap();
    assert!(end < start - 50.0, "start {start} end {end}");
    assert!(mid < start - 10.0 && mid > end + 10.0, "start {start} mid {mid} end {end}");
    assert!(!repaints(&rig.frame(3.15, 1)));
}
