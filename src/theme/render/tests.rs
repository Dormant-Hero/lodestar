use super::*;
use crate::library::CollectionKind;
use crate::model::Source;

const SRC: &str = r##"
[games]
[[games.elements]]
type = "image"
name = "bg"
path = "{art.hero}"
size = [1.0, 1.0]
[[games.elements]]
type = "grid"
name = "grid"
columns = 5
z = 2
pos = [0.0, 0.2]
size = [1.0, 0.8]
[[games.elements]]
type = "text"
name = "title"
z = 1
text = "{game.name}"
style = { font_size = 0.05, align = "center" }

[overrides.pc.games.title]
style = { font_size = 0.08 }
[overrides.steam.games.grid]
columns = 7
[variants.big.games.grid]
columns = 3
"##;

#[test]
fn overrides_merge_and_cache() {
    let file = ThemeFile::parse(SRC).unwrap();
    let steam = Collection { kind: CollectionKind::Source(Source::Steam), name: "Steam".into() };
    let desktop = Collection { kind: CollectionKind::Pc, name: "PC Games".into() };
    let p = prepare_view(&file, ViewKind::Games, Some(&steam), None).unwrap();
    // Steam matches "steam" first; only that override applies.
    match &p.view.elements[1] {
        Element::Grid { columns, .. } => assert_eq!(*columns, 7),
        _ => panic!(),
    }
    assert_eq!(p.order, vec![0, 2, 1]);
    assert_eq!(p.focus, Some(1));
    // PC Games → key "pc" → its override: font size changes, align kept.
    let p = prepare_view(&file, ViewKind::Games, Some(&desktop), None).unwrap();
    match &p.view.elements[2] {
        Element::Text { style, .. } => {
            assert_eq!(style.font_size, 0.08);
            assert_eq!(style.align, Align::Center);
        }
        _ => panic!(),
    }
    // Variant first, then collection override on top.
    let p = prepare_view(&file, ViewKind::Games, Some(&desktop), Some("big")).unwrap();
    match &p.view.elements[1] {
        Element::Grid { columns, .. } => assert_eq!(*columns, 3),
        _ => panic!(),
    }
    let p = prepare_view(&file, ViewKind::Games, Some(&steam), Some("big")).unwrap();
    match &p.view.elements[1] {
        Element::Grid { columns, .. } => assert_eq!(*columns, 7),
        _ => panic!(),
    }
    assert!(prepare_view(&file, ViewKind::Details, None, None).is_none());

    let theme = Theme { key: "t".into(), file, dir: None };
    let mut r = Renderer::new();
    let a = r.prepare(&theme, ViewKind::Games, Some(&steam), None).unwrap();
    let b = r.prepare(&theme, ViewKind::Games, Some(&steam), None).unwrap();
    assert!(Arc::ptr_eq(&a, &b), "prepared views are cached");
    let fi = a.focus_info(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0)));
    assert_eq!(fi.kind, FocusKind::Grid { columns: 7 });
    assert!(fi.page >= 7);
}

#[test]
fn bad_override_is_ignored() {
    let src = r##"
[[games.elements]]
type = "grid"
name = "grid"
columns = 5
[overrides.all.games.grid]
columns = "lots"
"##;
    let file = ThemeFile::parse(src).unwrap();
    let all = Collection { kind: CollectionKind::All, name: "All".into() };
    let p = prepare_view(&file, ViewKind::Games, Some(&all), None).unwrap();
    match &p.view.elements[0] {
        Element::Grid { columns, .. } => assert_eq!(*columns, 5),
        _ => panic!(),
    }
}

#[test]
fn approach_converges() {
    let mut v = 0.0;
    let mut frames = 0;
    while approach(&mut v, 10.0, 1.0 / 60.0, 13.0) {
        frames += 1;
        assert!(frames < 200);
    }
    assert_eq!(v, 10.0);
}

fn fake_game(id: i64, name: &str) -> Game {
    Game {
        id,
        source: Source::Steam,
        source_id: id.to_string(),
        art: crate::model::Art { cover: Some(format!("/nonexistent/{id}.jpg").into()), ..Default::default() },
        meta: crate::model::Metadata {
            description: Some("Words. ".repeat(200)),
            // Exercise the controller badge / chip icons on some games.
            controller: [Some(ControllerSupport::Full), Some(ControllerSupport::Partial), None]
                [id.rem_euclid(3) as usize],
            deck: (id % 2 == 1).then_some(crate::model::DeckRating::Verified),
            ..Default::default()
        },
        playtime_secs: 3600,
        last_played: Some(1_700_000_000),
        play_count: 1,
        favorite: id % 2 == 0,
        user_rating: Some(3),
        ..Game::blank(name)
    }
}

/// Draw every view of every built-in theme (and the wheel fixture) for a few frames
/// headlessly: nothing may panic, with or without fonts/images loaded.
#[test]
fn renders_all_builtin_views_headless() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut images = ImageCache::new(&ctx, dir.path().to_path_buf());
    let fonts = FontRegistry::new();
    let games: Vec<Game> = (0..40).map(|i| fake_game(i, &format!("Game {i}"))).collect();
    let indices: Vec<usize> = (0..games.len()).collect();
    let colls: Vec<CollectionItem> =
        [CollectionKind::All, CollectionKind::Source(Source::Steam), CollectionKind::Favorites]
            .into_iter()
            .map(|k| CollectionItem {
                collection: Collection { kind: k, name: "C".into() },
                name: "C".into(),
                count: 40,
                art: Default::default(),
            })
            .collect();
    let mut themes: Vec<Theme> =
        crate::theme::builtin::all().into_iter().map(|(k, file)| Theme { key: k.into(), file, dir: None }).collect();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/controller/wheeltest");
    let text = std::fs::read_to_string(fixture.join("theme.toml")).unwrap();
    themes.push(Theme { key: "wheeltest".into(), file: ThemeFile::parse(&text).unwrap(), dir: Some(fixture) });
    // Every button in every prompt style (and swapped) goes through the help bars.
    let hints: Vec<Hint> =
        [Button::A, Button::B, Button::X, Button::Y, Button::LB, Button::RB, Button::Start, Button::Select]
            .into_iter()
            .map(|b| Hint::new(b, format!("{b:?}")))
            .collect();
    use crate::input::{ButtonStyle, Prompts};
    let styles = [ButtonStyle::Xbox, ButtonStyle::PlayStation, ButtonStyle::Nintendo];
    for theme in &themes {
        let mut r = Renderer::new();
        for kind in [ViewKind::Collections, ViewKind::Games, ViewKind::Home] {
            for (frame, sel) in [0usize, 5, 39, 1000].into_iter().enumerate() {
                let coll = &colls[frame % colls.len()];
                let Some(p) = r.prepare(theme, kind, Some(&coll.collection), None) else {
                    assert_eq!(kind, ViewKind::Home, "{}", theme.key);
                    continue;
                };
                // Rows and rail data (the home page): two rows, the rail focused on odd frames.
                let rows: Vec<RowScene> = colls
                    .iter()
                    .take(2)
                    .map(|c| RowScene { collection: c, indices: &indices, selected: sel })
                    .collect();
                let shown = [0usize, 2, 5];
                let extra = SceneExtra {
                    games: &games,
                    rows: &rows,
                    row: frame,
                    rail: Some(RailScene {
                        shown: &shown,
                        current: Some(1),
                        selected: sel % 3,
                        focused: frame % 2 == 1,
                    }),
                    section: Some("Platforms"),
                };
                let items = match kind {
                    ViewKind::Collections => Items::Collections(&colls),
                    _ => Items::Games { games: &games, indices: &indices },
                };
                let scene = Scene {
                    kind,
                    game: games.get(sel),
                    collection: Some(coll),
                    items,
                    selected: sel,
                    list_id: 1,
                    hints: &hints,
                    prompts: Prompts::new(styles[(frame + sel) % 3], sel % 2 == 1),
                    opacity: 1.0,
                    interactive: true,
                    extra,
                };
                let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                    let screen = ui.max_rect();
                    r.draw(ui, screen, theme, &p, &fonts, &mut images, &scene);
                });
                out.textures_delta.clear();
            }
        }
    }
    // The fixture's Steam override renames the collection and recolours an element.
    let fx = themes.last().unwrap();
    let steam = Collection { kind: CollectionKind::Source(Source::Steam), name: "Steam".into() };
    let p = prepare_view(&fx.file, ViewKind::Collections, Some(&steam), None).unwrap();
    assert_eq!(p.display_name.as_deref(), Some("Steam Library"));
    let FocusInfo { kind, .. } = p.focus_info(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0)));
    assert_eq!(kind, FocusKind::Carousel { vertical: true });
}

/// Event-loop simulation: draw only when egui asks for a frame (immediately or after a
/// delay), like eframe's reactive mode. Returns the frame times.
fn simulate_frames(theme: &Theme, kind: ViewKind, games: &[Game], indices: &[usize], until: f64) -> Vec<f64> {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut images = ImageCache::new(&ctx, dir.path().to_path_buf());
    let fonts = FontRegistry::new();
    let coll = CollectionItem {
        collection: Collection { kind: CollectionKind::All, name: "All".into() },
        name: "All".into(),
        count: games.len(),
        art: Default::default(),
    };
    let colls = vec![coll.clone()];
    let mut r = Renderer::new();
    let p = r.prepare(theme, kind, Some(&coll.collection), None).unwrap();
    let mut t = 0.0f64;
    let mut times = vec![];
    while t < until {
        times.push(t);
        let input = egui::RawInput {
            time: Some(t),
            predicted_dt: 1.0 / 60.0,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0))),
            ..Default::default()
        };
        let items = match kind {
            ViewKind::Collections => Items::Collections(&colls),
            _ => Items::Games { games, indices },
        };
        let scene = Scene {
            kind,
            game: games.first(),
            collection: Some(&coll),
            items,
            selected: 0,
            list_id: 1,
            hints: &[],
            prompts: Default::default(),
            opacity: 1.0,
            interactive: false,
            extra: Default::default(),
        };
        let mut out = ctx.run_ui(input, |ui| {
            let screen = ui.max_rect();
            r.draw(ui, screen, theme, &p, &fonts, &mut images, &scene);
        });
        out.textures_delta.clear();
        let delay = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.repaint_delay.as_secs_f64())
            .unwrap_or(f64::INFINITY);
        t += delay.max(1.0 / 60.0);
    }
    times
}

/// Sitting on a games view whose theme auto-scrolls a long description must not keep
/// drawing forever, and while it scrolls it draws per pixel of movement, not per vsync.
#[test]
fn autoscroll_settles_and_is_throttled() {
    let classic = crate::theme::builtin::all()
        .into_iter()
        .find(|(k, _)| *k == "Classic")
        .map(|(k, file)| Theme { key: k.into(), file, dir: None })
        .unwrap();
    let games: Vec<Game> = (0..3).map(|i| fake_game(i, &format!("Game {i}"))).collect();
    let indices: Vec<usize> = (0..games.len()).collect();
    let times = simulate_frames(&classic, ViewKind::Games, &games, &indices, 900.0);
    let late = times.iter().filter(|&&t| t > 400.0).count();
    // Only the clock (once a minute) may wake an idle screen.
    assert!(late <= 10, "{late} frames between 400 s and 900 s of idling");
    let first_minute = times.iter().filter(|&&t| t < 60.0).count();
    assert!(
        first_minute < 60 * 30,
        "{first_minute} frames in the first minute (scrolling should not run at vsync rate)"
    );
}

/// The collections carousel loops: with the first item selected its left neighbour is the
/// last item, and no item is drawn twice when there are only a few.
#[test]
fn carousel_wraps_without_duplicates() {
    // No wrap: clamped to the items.
    assert_eq!(carousel_slots(0.0, 3.0, 10, false), (0, 3));
    assert_eq!(carousel_slots(9.0, 3.0, 10, false), (6, 9));
    // Wrap: slot -1 (item 9) sits left of item 0.
    let (lo, hi) = carousel_slots(0.0, 3.0, 10, true);
    assert_eq!((lo, hi), (-3, 3));
    assert_eq!((-1i64).rem_euclid(10), 9);
    // Three items: one each side of the selection, never repeated.
    let (lo, hi) = carousel_slots(0.0, 4.5, 3, true);
    assert_eq!(hi - lo + 1, 3);
    assert!(lo <= -1 && hi >= 1);
    let items: std::collections::HashSet<i64> = (lo..=hi).map(|s| s.rem_euclid(3)).collect();
    assert_eq!(items.len(), 3);
    // Mid-animation between slots keeps the count bounded too.
    let (lo, hi) = carousel_slots(-0.4, 4.5, 4, true);
    assert!(hi - lo < 4);
}

/// Text lists show whole rows; labels shrink so the longest word fits on a line.
#[test]
fn textlist_rows_and_label_word_fit() {
    // pixel: 0.611 of 720 px high, 0.03 font, 1.5 line spacing: 13 whole rows, not 13.6.
    let rh = 0.03 * 720.0 * 1.5;
    assert_eq!(whole_rows(0.611 * 720.0, rh), 13.0);
    assert_eq!(whole_rows(100.0, 30.0), 3.0);
    assert_eq!(whole_rows(90.0, 30.0), 3.0);
    assert_eq!(whole_rows(10.0, 30.0), 1.0);
    assert_eq!(word_fit_px(40.0, 100.0, 200.0), 40.0);
    let px = word_fit_px(40.0, 300.0, 200.0);
    assert!(px < 40.0 * 200.0 / 300.0 + 0.01 && px > 20.0);
}

/// Clocks can sit on a pill: DateTime takes a background like Text does.
#[test]
fn datetime_background_parses() {
    let el: Element = toml::from_str(
        "type = \"datetime\"\nclock = true\nbackground = \"#00000080\"\npadding = [0.01, 0.005]\nradius = 0.01",
    )
    .unwrap();
    match el {
        Element::DateTime { background, common, .. } => {
            assert_eq!(background, Some(Color([0, 0, 0, 0x80])));
            assert_eq!(common.padding, Some([0.01, 0.005]));
        }
        _ => panic!("not a datetime"),
    }
    let plain: Element = toml::from_str("type = \"datetime\"\nclock = true").unwrap();
    assert!(matches!(plain, Element::DateTime { background: None, .. }));
}

#[test]
fn textlist_selector_geometry() {
    let list = Rect::from_min_size(pos2(100.0, 50.0), vec2(400.0, 600.0));
    // Default: the whole row.
    let full = selector_rect(list, 80.0, 30.0, None, 0.0, 1280.0);
    assert_eq!(full, Rect::from_min_size(pos2(100.0, 80.0), vec2(400.0, 30.0)));
    // A thin side marker: 0.0035 of 1280 px wide, offset 0.003 of the width.
    let thin = selector_rect(list, 80.0, 30.0, Some(0.0035), 0.003, 1280.0);
    assert!((thin.left() - 103.84).abs() < 0.01 && (thin.width() - 4.48).abs() < 0.01);
    // Never wider than the list, never outside it.
    let wide = selector_rect(list, 80.0, 30.0, Some(2.0), 0.1, 1280.0);
    assert!(wide.left() >= list.left() && wide.right() <= list.right() + 0.01);
    // The fields parse from a theme and default sensibly.
    let el: Element = toml::from_str("type = \"textlist\"\nselector_width = 0.004\nselector_offset = 0.002").unwrap();
    match el {
        Element::TextList { selector_width, selector_offset, .. } => {
            assert_eq!(selector_width, Some(0.004));
            assert_eq!(selector_offset, 0.002);
        }
        _ => panic!("not a textlist"),
    }
}

#[test]
fn favourite_star_reads_on_its_dark_disc() {
    use egui::Color32;
    let lum = |c: Color32| {
        let [r, g, b, _] = c.to_array();
        (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32) / 255.0
    };
    // A bright accent stays as it is (opaque).
    let gold = Color32::from_rgb(0xff, 0xc4, 0x4d);
    assert_eq!(draw::favorite_star_color(gold), gold);
    assert_eq!(draw::favorite_star_color(Color32::from_rgba_unmultiplied(0xff, 0xc4, 0x4d, 80)).a(), 255);
    // A dark accent (deep blue, dark red) is lifted towards white so the star stays readable,
    // keeping its hue.
    for dark in [Color32::from_rgb(0x1a, 0x3a, 0x8c), Color32::from_rgb(0x80, 0x10, 0x10), Color32::BLACK] {
        let c = draw::favorite_star_color(dark);
        assert!(lum(c) >= 0.67, "{dark:?} → {c:?}");
    }
    let blue = draw::favorite_star_color(Color32::from_rgb(0x1a, 0x3a, 0x8c));
    assert!(blue.b() > blue.r());
}

#[test]
fn favourite_badge_sits_inside_the_top_right_corner() {
    let item = egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(200.0, 300.0));
    let (c, r) = draw::favorite_badge_spot(item, 30.0);
    assert_eq!(r, 15.0);
    let disc = egui::Rect::from_center_size(c, egui::vec2(30.0, 30.0));
    assert!(item.contains_rect(disc));
    // Top-right: inset by the same small margin from both edges.
    assert!((item.right() - disc.right() - (disc.top() - item.top())).abs() < 1e-3);
    assert!(disc.right() > item.center().x && disc.bottom() < item.center().y);
}

#[test]
fn look_switches_follow_the_controller_settings() {
    let mut c = crate::config::ControllerConfig::default();
    assert_eq!(ViewOpts::from_config(&c), ViewOpts::default(), "defaults = the theme as designed");
    c.blur_background = false;
    c.show_titles = false;
    c.show_battery = false;
    let o = ViewOpts::from_config(&c);
    assert!(!o.blur_background && !o.show_titles && !o.show_battery && o.show_clock);
    assert_eq!(o.text_scale, 1.0);
    c.text_size = crate::config::TextSize::Large;
    assert!(ViewOpts::from_config(&c).text_scale > 1.1, "Text size reaches what the theme draws");
    assert!(super::is_battery_text("{system.battery}"));
    assert!(super::is_battery_text("{system.charging}"));
    assert!(!super::is_battery_text("{game.name}"));
}

#[test]
fn games_without_art_get_a_steady_colour_per_console() {
    use crate::model::{EmuRef, Game, Source};
    let emu = |id: i64, p: &str| Game {
        id,
        source: Source::Emulator,
        emu: Some(EmuRef { platform: p.into(), ..Default::default() }),
        ..Game::blank("x")
    };
    let a = draw::placeholder_tint(&emu(1, "sony_playstation2"));
    // Same console = same colour, whatever the game; a fixed value across runs (FNV, not a
    // randomly seeded hash), so it never changes between starts.
    assert_eq!(a, draw::placeholder_tint(&emu(2, "sony_playstation2")));
    assert_eq!(a, draw::placeholder_tint(&emu(1, "sony_playstation2")));
    // PC games go by their store.
    let steam = Game { source: Source::Steam, ..Game::blank("y") };
    let _ = draw::placeholder_tint(&steam);
    // Some consoles differ (8 colours, so not all).
    let all: std::collections::HashSet<_> =
        ["nintendo_snes", "sega_genesis", "sony_psp", "nintendo_64", "sony_playstation"]
            .iter()
            .map(|p| draw::placeholder_tint(&emu(3, p)))
            .collect();
    assert!(all.len() > 1);
    // An image can opt in from a theme file.
    let t = crate::theme::model::ThemeFile::parse(crate::theme::builtin::SOURCES[4].1).unwrap();
    let hero = t.home.as_ref().unwrap().elements.iter().find(|e| e.common().name == "hero").unwrap();
    assert!(hero.common().placeholder);
}

#[test]
fn badges_show_only_what_applies() {
    use crate::model::{CompletionStatus, ControllerSupport, Game};
    let mut g = Game::blank("Celeste");
    assert!(!badge_active("favorite", &g) && !badge_active("completed", &g) && !badge_active("controller", &g));
    g.favorite = true;
    g.edits.completion = Some(CompletionStatus::Beaten);
    g.meta.controller = Some(ControllerSupport::Full);
    assert!(badge_active("favorite", &g) && badge_active("completed", &g) && badge_active("controller", &g));
    g.edits.completion = Some(CompletionStatus::Playing);
    g.meta.controller = Some(ControllerSupport::Partial);
    assert!(!badge_active("completed", &g) && !badge_active("controller", &g));
    // No data in Lodestar: never shown.
    for s in ["kidgame", "broken", "altemulator", "collection", "folder", "manual"] {
        assert!(!badge_active(s, &g), "{s}");
    }
}

#[test]
fn tiny_tiles_fill_big_screens() {
    use crate::theme::render::draw::tile_grid;
    use egui::vec2;
    // A tinted 16 px white square over a 1080p panel: every tile drawn (a cap of 2500
    // tiles left Colorful's per-system colours missing at 1080p).
    let (ts, nx, ny) = tile_grid(vec2(770.0, 1080.0), vec2(16.0, 16.0));
    assert_eq!((ts, nx, ny), (vec2(16.0, 16.0), 49, 68));
    // A whole 4K screen: bigger tiles, bounded work per frame, still covering it all.
    let (ts, nx, ny) = tile_grid(vec2(3840.0, 2160.0), vec2(16.0, 16.0));
    assert!(nx * ny <= 20_000, "{nx}x{ny}");
    assert!(ts.x * nx as f32 >= 3840.0 && ts.y * ny as f32 >= 2160.0);
}

/// A theme using every look field (hand-written, Playnite-like shapes).
const LOOK: &str = r##"
[theme]
name = "Look"
[games]
[[games.elements]]
type = "rect"
name = "bar"
size = [1.0, 0.1]
gradient_stops = [["#ff0000", 0.0], ["#00ff00", 0.5], ["#0000ff", 1.0]]
gradient_angle = 30
[[games.elements]]
type = "rect"
name = "glowbg"
pos = [0.0, 0.1]
size = [1.0, 0.3]
corner_radius = 0.02
gradient_stops = [["#ffffff", 0.0], ["#000000", 1.0]]
gradient_radial = [0.5, 0.5, 0.5, 0.5]
[[games.elements]]
type = "rect"
name = "turned"
pos = [0.0, 0.1]
size = [0.2, 0.1]
gradient = ["#000000", "#ffffff"]
gradient_angle = 0
[[games.elements]]
type = "grid"
name = "grid"
z = 2
pos = [0.0, 0.4]
size = [1.0, 0.6]
columns = 6
corner_radius = 0.01
item_caption = "{game.name}"
selected_border = "#ff8800"
selected_border_width = 0.01
selected_radius = 0.02
selected_glow = { color = "#ff880080", blur = 0.03 }
uninstalled_scrim = "#00000099"
caption_background = { stops = [["#00000000", 0.0], ["#000000cc", 1.0]] }
item_text_style = { shadow = "#000000" }
[[games.elements]]
type = "textlist"
name = "list"
pos = [0.0, 0.0]
size = [0.3, 0.3]
style = { shadow = "#00000080" }
[[games.elements]]
type = "helpbar"
name = "help"
pos = [0.5, 0.95]
style = { shadow = "#00000080" }
[[games.elements]]
type = "rows"
name = "rows"
pos = [0.0, 0.5]
size = [1.0, 0.5]
caption_background = "#202020"
selected_glow = { color = "#00000000" }
"##;

#[test]
fn look_fields_parse_and_round_trip() {
    let file = ThemeFile::parse(LOOK).unwrap();
    let c = file.games.elements[3].common();
    assert_eq!(c.selected_border_width, Some(0.01));
    assert_eq!(c.selected_radius, Some(0.02));
    assert_eq!(c.selected_glow, Some(Glow { color: Color::parse("#ff880080").unwrap(), blur: 0.03 }));
    assert_eq!(c.uninstalled_scrim, Color::parse("#00000099"));
    assert!(c.caption_background.as_ref().unwrap().gradient().is_some());
    let bar = file.games.elements[0].common();
    let g = bar.gradient_spec(None).unwrap();
    assert_eq!((g.stops.len(), g.angle, g.radial), (3, Some(30.0), None));
    let radial = file.games.elements[1].common().gradient_spec(None).unwrap();
    assert_eq!(radial.radial, Some([0.5, 0.5, 0.5, 0.5]));
    // A two-colour gradient with an angle goes through the mesh too; without one it doesn't.
    let two = [Color::BLACK, Color::WHITE];
    assert_eq!(file.games.elements[2].common().gradient_spec(Some(two)).unwrap().angle, Some(0.0));
    assert!(Common::default().gradient_spec(Some(two)).is_none());
    // Rows: a solid caption strip and a glow turned off.
    let rows = file.games.elements[6].common();
    assert!(matches!(rows.caption_background, Some(Brush::Solid(_))));
    assert_eq!(rows.selected_glow.unwrap().blur, 0.02);
    let again = ThemeFile::parse(&file.to_toml().unwrap()).unwrap();
    assert_eq!(file, again);
    // Unset fields don't appear in a saved theme (old themes stay byte-identical).
    let plain = ThemeFile::parse(SRC).unwrap().to_toml().unwrap();
    for k in ["gradient_stops", "selected_glow", "uninstalled_scrim", "caption_background", "selected_radius"] {
        assert!(!plain.contains(k), "{k}");
    }
}

#[test]
fn look_fields_render() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = egui::Context::default();
    let mut images = ImageCache::new(&ctx, dir.path().to_path_buf());
    let fonts = FontRegistry::new();
    let mut games: Vec<Game> = (0..12).map(|i| fake_game(i, &format!("Game {i}"))).collect();
    for g in &mut games {
        g.installed = g.id % 2 == 0;
    }
    let indices: Vec<usize> = (0..games.len()).collect();
    let theme = Theme { key: "look".into(), file: ThemeFile::parse(LOOK).unwrap(), dir: None };
    let coll = CollectionItem {
        collection: Collection { kind: CollectionKind::All, name: "All".into() },
        name: "All".into(),
        count: 12,
        art: Default::default(),
    };
    let mut r = Renderer::new();
    let p = r.prepare(&theme, ViewKind::Games, Some(&coll.collection), None).unwrap();
    let hints = [Hint::new(Button::A, "Play")];
    let rows = [RowScene { collection: &coll, indices: &indices, selected: 1 }];
    let scene = Scene {
        kind: ViewKind::Games,
        game: games.get(1),
        collection: Some(&coll),
        items: Items::Games { games: &games, indices: &indices },
        selected: 1,
        list_id: 1,
        hints: &hints,
        prompts: Default::default(),
        opacity: 1.0,
        interactive: true,
        extra: SceneExtra { games: &games, rows: &rows, ..Default::default() },
    };
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0));
    let mut shapes = vec![];
    for _ in 0..3 {
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let out = ctx.run_ui(input, |ui| {
            r.draw(ui, screen, &theme, &p, &fonts, &mut images, &scene);
        });
        shapes = out.shapes;
    }
    let meshes: Vec<&egui::epaint::Mesh> = shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Mesh(m) => Some(&**m),
            _ => None,
        })
        .collect();
    // The 3-stop bar: its middle stop is a vertex colour.
    let green = Color32::from_rgb(0, 255, 0);
    assert!(meshes.iter().any(|m| m.vertices.iter().any(|v| v.color == green)), "3-stop gradient");
    // The radial background: white centre, black outside, AA fringe on its rounded corners.
    assert!(meshes.iter().any(|m| {
        m.vertices.iter().any(|v| v.color == Color32::WHITE)
            && m.vertices.iter().any(|v| v.color == Color32::BLACK)
            && m.vertices.iter().any(|v| v.color == Color32::TRANSPARENT)
    }));
    let rects: Vec<&egui::epaint::RectShape> = shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) => Some(r),
            _ => None,
        })
        .collect();
    // Focus border 0.01 × 900 = 9 px in the border colour, with the 0.02 × 900 radius.
    let orange = Color32::from_rgb(255, 136, 0);
    assert!(
        rects
            .iter()
            .any(|r| r.stroke.color == orange && (r.stroke.width - 9.0).abs() < 0.01 && r.corner_radius.nw == 18),
        "focus border"
    );
    // The theme's glow reaches 0.03 × 900 = 27 px past the border (a 54 px blur).
    assert!(rects.iter().any(|r| r.blur_width >= 53.0 && r.blur_width <= 55.0), "glow");
    // Not-installed games get the theme's scrim (instead of grey dimming).
    let scrim = Color::parse("#00000099").unwrap().to_egui();
    assert!(rects.iter().any(|r| r.fill == scrim), "scrim");
}

#[test]
fn gradient_meshes_are_reused_between_frames() {
    let ctx = egui::Context::default();
    let dir = tempfile::tempdir().unwrap();
    let mut images = ImageCache::new(&ctx, dir.path().to_path_buf());
    let fonts = FontRegistry::new();
    let theme = Theme { key: "look".into(), file: ThemeFile::parse(LOOK).unwrap(), dir: None };
    let mut r = Renderer::new();
    let p = r.prepare(&theme, ViewKind::Games, None, None).unwrap();
    let scene = Scene {
        kind: ViewKind::Games,
        game: None,
        collection: None,
        items: Items::Games { games: &[], indices: &[] },
        selected: 0,
        list_id: 1,
        hints: &[],
        prompts: Default::default(),
        opacity: 1.0,
        interactive: false,
        extra: Default::default(),
    };
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 720.0));
    let mut ptrs: Vec<Vec<usize>> = vec![];
    for _ in 0..2 {
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let out = ctx.run_ui(input, |ui| {
            r.draw(ui, screen, &theme, &p, &fonts, &mut images, &scene);
        });
        ptrs.push(
            out.shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    egui::Shape::Mesh(m) => Some(Arc::as_ptr(m) as usize),
                    _ => None,
                })
                .collect(),
        );
    }
    assert!(ptrs[0].len() >= 3, "{:?}", ptrs[0]);
    assert_eq!(ptrs[0], ptrs[1], "same meshes, no rebuild");
}
