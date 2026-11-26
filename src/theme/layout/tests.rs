use super::*;

const INF: f32 = f32::INFINITY;

/// The cases are Playnite Fullscreen views, laid out on the screen they are designed for.
fn layout<T>(root: &mut Node<T>, m: &mut dyn Measurer) {
    super::layout(root, [1920.0, 1080.0], m);
}

type N = Node<&'static str>;

fn node(name: &'static str) -> N {
    Node::new(name)
}

fn find<'a>(n: &'a N, name: &str) -> &'a N {
    let mut out = None;
    walk(n, &mut |x, _| {
        if x.data == name && out.is_none() {
            out = Some(x as *const N);
        }
    });
    unsafe { &*out.expect(name) }
}

fn fixed(name: &'static str, w: f32, h: f32) -> N {
    let mut n = node(name);
    n.content = Content::Fixed([w, h]);
    n.ha = Al::Start;
    n.va = Al::Start;
    n
}

#[test]
fn playnite_main_bars_and_star_row() {
    // Playnite's default Main: rows Auto / * / Auto, a 110 px top bar, a 100 px bottom bar.
    let mut root = node("root");
    root.panel =
        Panel::Grid { rows: vec![Def::new(Len::Auto), Def::new(Len::Star(1.0)), Def::new(Len::Auto)], cols: vec![] };
    let mut top = node("top");
    top.height = Some(110.0);
    let mut list = node("list");
    list.row = 1;
    list.margin = [25.0, 0.0, 25.0, 0.0];
    let mut bottom = node("bottom");
    bottom.row = 2;
    bottom.height = Some(100.0);
    root.children = vec![top, list, bottom];
    layout(&mut root, &mut Rough);
    assert_eq!(find(&root, "top").rect, Rect::new(0.0, 0.0, 1920.0, 110.0));
    assert_eq!(find(&root, "list").rect, Rect::new(25.0, 110.0, 1870.0, 870.0));
    assert_eq!(find(&root, "bottom").rect, Rect::new(0.0, 980.0, 1920.0, 100.0));
}

#[test]
fn grid_columns_pixel_auto_star_and_spans() {
    // GameDetails: columns * / 350 / 6* / *, rows 3* / 4*.
    let mut root = node("root");
    root.panel = Panel::Grid {
        rows: vec![Def::new(Len::Star(3.0)), Def::new(Len::Star(4.0))],
        cols: vec![
            Def::new(Len::Star(1.0)),
            Def::new(Len::Px(350.0)),
            Def::new(Len::Star(6.0)),
            Def::new(Len::Star(1.0)),
        ],
    };
    let mut band = node("band");
    band.row = 1;
    band.col_span = 4;
    let mut cover = fixed("cover", 200.0, 300.0);
    cover.row = 1;
    cover.col = 1;
    cover.ha = Al::End;
    cover.margin = [14.0; 4];
    let mut text = node("text");
    text.row = 1;
    text.col = 2;
    root.children = vec![band, cover, text];
    layout(&mut root, &mut Rough);
    let star = (1920.0 - 350.0) / 8.0;
    assert_eq!(find(&root, "band").rect, Rect::new(0.0, 1080.0 * 3.0 / 7.0, 1920.0, 1080.0 * 4.0 / 7.0));
    let c = find(&root, "cover").rect;
    assert!((c.x - (star + 350.0 - 14.0 - 200.0)).abs() < 0.01, "{c:?}");
    assert!((c.y - (1080.0 * 3.0 / 7.0 + 14.0)).abs() < 0.01);
    let t = find(&root, "text").rect;
    assert!((t.x - (star + 350.0)).abs() < 0.01 && (t.w - star * 6.0).abs() < 0.01);
}

#[test]
fn auto_columns_take_their_content() {
    let mut root = node("root");
    root.panel = Panel::Grid {
        rows: vec![],
        cols: vec![Def::new(Len::Px(100.0)), Def::new(Len::Star(1.0)), Def::new(Len::Auto), Def::new(Len::Px(100.0))],
    };
    root.height = Some(110.0);
    root.va = Al::Start;
    let mut clock = fixed("clock", 80.0, 30.0);
    clock.col = 2;
    clock.va = Al::Center;
    clock.margin = [20.0, 0.0, 20.0, 0.0];
    root.children = vec![clock];
    layout(&mut root, &mut Rough);
    let c = find(&root, "clock").rect;
    assert_eq!(c, Rect::new(1920.0 - 100.0 - 120.0 + 20.0, 40.0, 80.0, 30.0));
}

#[test]
fn stack_dock_and_collapsed_children() {
    let mut dock = node("dock");
    dock.panel = Panel::Dock { fill_last: true };
    dock.margin = [20.0; 4];
    let mut title = fixed("title", 400.0, 50.0);
    title.dock = Dock::Top;
    let mut gone = fixed("gone", 400.0, 500.0);
    gone.dock = Dock::Top;
    gone.vis = Vis::Collapsed;
    let mut hidden = fixed("hidden", 10.0, 30.0);
    hidden.dock = Dock::Top;
    hidden.vis = Vis::Hidden;
    let mut row = node("row");
    row.panel = Panel::Stack { horizontal: true };
    row.dock = Dock::Top;
    row.children = vec![fixed("a", 100.0, 40.0), fixed("b", 120.0, 60.0)];
    let fill = node("fill");
    dock.children = vec![title, gone, hidden, row, fill];
    layout(&mut dock, &mut Rough);
    assert_eq!(find(&dock, "title").rect, Rect::new(20.0, 20.0, 400.0, 50.0));
    assert_eq!(find(&dock, "hidden").rect.y, 70.0);
    assert_eq!(find(&dock, "a").rect, Rect::new(20.0, 100.0, 100.0, 40.0));
    assert_eq!(find(&dock, "b").rect, Rect::new(120.0, 100.0, 120.0, 60.0));
    assert_eq!(find(&dock, "fill").rect, Rect::new(20.0, 160.0, 1880.0, 900.0));
}

#[test]
fn alignment_size_limits_canvas_viewbox_and_overflow() {
    let mut root = node("root");
    let mut capped = node("capped");
    capped.max = [600.0, INF];
    let mut big = fixed("big", 10.0, 10.0);
    big.width = Some(3000.0);
    big.ha = Al::Center;
    let mut canvas = node("canvas");
    canvas.panel = Panel::Canvas;
    let mut dot = fixed("dot", 10.0, 10.0);
    dot.canvas = [None, None, Some(30.0), Some(40.0)];
    canvas.children = vec![dot];
    let mut vb = node("vb");
    vb.panel = Panel::Viewbox { stretch: Scale::Uniform };
    vb.width = Some(200.0);
    vb.height = Some(100.0);
    vb.ha = Al::Start;
    vb.va = Al::Start;
    vb.children = vec![fixed("inside", 400.0, 100.0)];
    let mut moved = fixed("moved", 10.0, 10.0);
    moved.translate = [5.0, -5.0];
    root.children = vec![capped, big, canvas, vb, moved];
    layout(&mut root, &mut Rough);
    // Stretch with a Max centres.
    assert_eq!(find(&root, "capped").rect, Rect::new(660.0, 0.0, 600.0, 1080.0));
    // Wider than the screen, centred: overflows on both sides.
    assert_eq!(find(&root, "big").rect.x, -540.0);
    assert_eq!(find(&root, "dot").rect, Rect::new(1880.0, 1030.0, 10.0, 10.0));
    let i = find(&root, "inside");
    assert_eq!(i.rect, Rect::new(0.0, 25.0, 200.0, 50.0));
    assert!((i.scale - 0.5).abs() < 1e-6);
    assert_eq!(find(&root, "moved").rect, Rect::new(5.0, -5.0, 10.0, 10.0));
}

#[test]
fn text_and_images_measure() {
    let mut root = node("root");
    root.panel = Panel::Stack { horizontal: false };
    root.width = Some(300.0);
    root.ha = Al::Start;
    let mut t = node("t");
    t.content = Content::Text(TextSpec { text: "x".repeat(40), size: 20.0, bold: false, wrap: true, max_lines: None });
    let mut img = node("img");
    img.content = Content::Image { natural: Some([600.0, 900.0]), stretch: Scale::Uniform };
    let mut none = node("none");
    none.content = Content::Image { natural: None, stretch: Scale::Uniform };
    root.children = vec![t, img, none];
    layout(&mut root, &mut Rough);
    let t = find(&root, "t").rect;
    assert_eq!(t.w, 300.0);
    // 40 chars at 10.4 px a char → 28 a line → 2 lines.
    assert!((t.h - 2.0 * 26.6).abs() < 0.01, "{t:?}");
    let i = find(&root, "img").rect;
    assert_eq!((i.w, i.h), (300.0, 450.0));
    assert_eq!(find(&root, "none").rect.h, 0.0);
}

#[test]
fn grid_star_rows_with_minimum() {
    let mut root = node("root");
    root.panel = Panel::Grid {
        rows: vec![Def { len: Len::Star(1.0), min: 700.0, max: INF }, Def::new(Len::Star(1.0))],
        cols: vec![],
    };
    let a = node("a");
    let mut b = node("b");
    b.row = 1;
    root.children = vec![a, b];
    layout(&mut root, &mut Rough);
    assert_eq!(find(&root, "a").rect.h, 700.0);
    assert_eq!(find(&root, "b").rect, Rect::new(0.0, 700.0, 1920.0, 380.0));
}

#[test]
fn grids_without_definitions_in_a_stack_size_to_their_content() {
    // A vertical StackPanel of label Grids (each a centred 27 px text, 15 px apart): the
    // Grids' star row is Auto in the unbounded direction, so the labels don't pile up.
    let mut stack = node("stack");
    stack.panel = Panel::Stack { horizontal: false };
    stack.ha = Al::Start;
    stack.va = Al::Start;
    let cell = |g: &'static str, t: &'static str| {
        let mut grid = node(g);
        grid.panel = Panel::Grid { rows: vec![], cols: vec![] };
        grid.margin = [0.0, 0.0, 0.0, 15.0];
        let mut text = fixed(t, 100.0, 27.0);
        text.va = Al::Center;
        grid.children = vec![text];
        grid
    };
    stack.children = vec![cell("g1", "t1"), cell("g2", "t2")];
    layout(&mut stack, &mut Rough);
    assert_eq!(find(&stack, "g1").rect, Rect::new(0.0, 0.0, 100.0, 27.0));
    assert_eq!(find(&stack, "t2").rect, Rect::new(0.0, 42.0, 100.0, 27.0));
    assert_eq!(find(&stack, "stack").rect.h, 84.0);
}
