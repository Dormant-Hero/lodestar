//! Tests: views scaled as one piece (`View::viewbox`), and help bars (theme prompts, hints that
//! must show).

use super::*;
use crate::library::CollectionKind;

fn all() -> CollectionItem {
    CollectionItem {
        collection: Collection { kind: CollectionKind::All, name: "All".into() },
        name: "All".into(),
        count: 0,
        art: Default::default(),
    }
}

/// One frame of the games view of `src` on a `size` screen with `hints`.
fn frame(src: &str, size: Vec2, hints: &[Hint]) -> egui::FullOutput {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut images = ImageCache::new(&ctx, dir.path().to_path_buf());
    let fonts = FontRegistry::new();
    let theme = Theme { key: "t".into(), file: ThemeFile::parse(src).unwrap(), dir: None };
    let coll = all();
    let mut r = Renderer::new();
    let p = r.prepare(&theme, ViewKind::Games, Some(&coll.collection), None).unwrap();
    let games = vec![Game::blank("A game")];
    let indices = vec![0];
    let scene = Scene {
        kind: ViewKind::Games,
        game: games.first(),
        collection: Some(&coll),
        items: Items::Games { games: &games, indices: &indices },
        selected: 0,
        list_id: 1,
        hints,
        prompts: Default::default(),
        opacity: 1.0,
        interactive: false,
        extra: Default::default(),
    };
    let screen = Rect::from_min_size(Pos2::ZERO, size);
    let input = egui::RawInput { time: Some(0.0), screen_rect: Some(screen), ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| {
        r.draw(ui, screen, &theme, &p, &fonts, &mut images, &scene);
    });
    out.textures_delta.clear();
    out
}

fn rect_filled(out: &egui::FullOutput, c: Color32) -> Option<Rect> {
    out.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Rect(r) if r.fill == c => Some(r.rect),
        _ => None,
    })
}

fn text_shape<'a>(out: &'a egui::FullOutput, starts: &str) -> Option<&'a egui::epaint::TextShape> {
    out.shapes.iter().find_map(|s| match &s.shape {
        egui::Shape::Text(t) if t.galley.job.text.starts_with(starts) => Some(t),
        _ => None,
    })
}

fn labels(out: &egui::FullOutput) -> Vec<String> {
    out.shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) if !t.galley.job.text.is_empty() => Some(t.galley.job.text.clone()),
            _ => None,
        })
        .collect()
}

const BOXED: &str = r##"
[games]
viewbox = [1920.0, 1080.0]
[[games.elements]]
type = "rect"
name = "bg"
size = [1.0, 1.0]
color = "#ff0000"
[[games.elements]]
type = "rect"
name = "band"
size = [1.0, 0.1]
color = "#00ff00"
border_color = "#ffffff"
border_width = 0.004
z = 1
[[games.elements]]
type = "rect"
name = "corner"
size = [0.1, 0.1]
color = "#0000ff"
z = 2
[[games.elements]]
type = "text"
name = "label"
text = "Last Played 20 Mar 2023"
pos = [0.5, 0.5]
size = [0.2, 0.0]
max_lines = 1
style = { font_size = 0.03 }
z = 3
"##;

#[test]
fn a_viewbox_view_scales_as_one_piece_on_a_16_10_screen() {
    // 1280 × 800: one scale (1280 / 1920), the 1280 × 720 canvas centred (40 px above and below).
    let out = frame(BOXED, vec2(1280.0, 800.0), &[]);
    let corner = rect_filled(&out, Color32::from_rgb(0, 0, 255)).expect("corner");
    assert!((corner.top() - 40.0).abs() < 0.5 && (corner.width() - 128.0).abs() < 0.5, "{corner:?}");
    assert!((corner.height() - 72.0).abs() < 0.5, "same scale both ways: {corner:?}");
    // Texts keep the proportions of their boxes: one that fits at 1920 × 1080 fits here too.
    let label = text_shape(&out, "Last Played").expect("label");
    assert_eq!(label.galley.job.text, "Last Played 20 Mar 2023");
    assert!(!label.galley.elided, "fits its box as designed");
    assert!((label.galley.job.sections[0].format.font_id.size - 0.03 * 720.0).abs() < 0.01);
    // The background and the full-width band reach into the room above and below; the
    // band's border stays on the canvas.
    let bg = rect_filled(&out, Color32::from_rgb(255, 0, 0)).expect("bg");
    assert_eq!((bg.top(), bg.bottom()), (0.0, 800.0));
    let band = rect_filled(&out, Color32::from_rgb(0, 255, 0)).expect("band");
    assert!(band.top() == 0.0 && (band.bottom() - 112.0).abs() < 0.5, "{band:?}");
    let border = out
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Rect(r) if r.stroke.width > 0.0 => Some(r.rect),
            _ => None,
        })
        .expect("border");
    assert!((border.top() - 40.0).abs() < 0.5, "{border:?}");
}

#[test]
fn views_without_a_viewbox_still_stretch_with_the_screen() {
    let src = BOXED.replace("viewbox = [1920.0, 1080.0]", "");
    let out = frame(&src, vec2(1280.0, 800.0), &[]);
    let corner = rect_filled(&out, Color32::from_rgb(0, 0, 255)).expect("corner");
    assert_eq!(corner, Rect::from_min_size(Pos2::ZERO, vec2(128.0, 80.0)));
}

#[test]
fn the_view_frame_is_centred_and_keeps_the_designs_shape() {
    let mut v = View { viewbox: Some([1920.0, 1080.0]), ..Default::default() };
    let deck = view_frame(&v, Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0)));
    assert_eq!(deck, Rect::from_min_max(pos2(0.0, 40.0), pos2(1280.0, 760.0)));
    let wide = view_frame(&v, Rect::from_min_size(Pos2::ZERO, vec2(2560.0, 1080.0)));
    assert_eq!(wide, Rect::from_min_max(pos2(320.0, 0.0), pos2(2240.0, 1080.0)));
    v.viewbox = None;
    let any = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 800.0));
    assert_eq!(view_frame(&v, any), any);
}

const BARS: &str = r##"
[[games.elements]]
type = "helpbar"
name = "only_a"
pos = [0.05, 0.9]
buttons = ["a", "start"]
style = { font_size = 0.03 }
[[games.elements]]
type = "helpbar"
name = "prompt"
pos = [0.5, 0.1]
size = [0.03, 0.03]
buttons = ["y"]
glyphs = true
style = { font_size = 0.03, align = "center" }
"##;

#[test]
fn a_must_show_hint_joins_a_bar_that_lists_other_buttons() {
    let hints = [
        Hint::new(Button::A, "Open"),
        Hint::new(Button::B, "Game menu").always(),
        Hint::new(Button::X, "Favourite"),
        Hint::new(Button::Start, "Menu"),
    ];
    let shown = labels(&frame(BARS, vec2(1280.0, 720.0), &hints));
    assert!(shown.contains(&"Open".to_string()) && shown.contains(&"Game menu".to_string()), "{shown:?}");
    // Other hints the bar doesn't list stay out.
    assert!(!shown.contains(&"Favourite".to_string()), "{shown:?}");
    // Listed by another bar of the view: shown there only, once.
    let both = format!(
        "{BARS}\n[[games.elements]]\ntype = \"helpbar\"\nname = \"only_b\"\npos = [0.6, 0.9]\nbuttons = [\"b\"]\n"
    );
    let shown = labels(&frame(&both, vec2(1280.0, 720.0), &hints));
    assert_eq!(shown.iter().filter(|l| *l == "Game menu").count(), 1, "{shown:?}");
    // Without the flag it is left out as before.
    let plain = [Hint::new(Button::A, "Open"), Hint::new(Button::B, "Back")];
    let shown = labels(&frame(BARS, vec2(1280.0, 720.0), &plain));
    assert!(!shown.contains(&"Back".to_string()), "{shown:?}");
}

#[test]
fn a_theme_prompt_draws_its_button_without_a_label_whatever_the_hints() {
    let src = BARS.replace("buttons = [\"a\", \"start\"]", "buttons = [\"x\"]");
    let out = frame(&src, vec2(1280.0, 720.0), &[]);
    // Xbox style: Y is a disc with its letter; no label next to it.
    let discs = out.shapes.iter().filter(|s| matches!(s.shape, egui::Shape::Circle(_))).count();
    assert!(discs >= 1, "{:?}", out.shapes);
    assert_eq!(labels(&out), ["Y"]);
}
