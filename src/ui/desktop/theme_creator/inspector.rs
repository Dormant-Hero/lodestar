//! The inspector: every setting of the selected element with a fitting control — place and
//! size, colours (palette or own), fonts, text with values from the picker (and what they show
//! now), pictures (a file brought into the theme, or the game's art), list settings, motion,
//! when it shows, and the element as TOML for anything else.

use super::super::Pal;
use super::bindings::Use;
use super::files::Asset;
use super::ops::{self, ElPath, Layer, Nest};
use super::widgets::{self, color, heading, note, opt_color, pct, row};
use super::{AssetFor, BindTarget, CreatorDlg, PickFor};
use crate::app::AppState;
use crate::theme::*;
use egui::{Id, RichText, Ui, vec2};
use std::collections::HashMap;

/// Which file setting a picked file goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Path,
    Video,
    ItemImage,
    SelectorImage,
    StarFilled,
    StarEmpty,
}

/// Put theme path `rel` into slot `s` of `e`.
pub fn set_slot(e: &mut Element, s: Slot, rel: String) {
    match (s, e) {
        (Slot::Path, Element::Image { path, .. }) => *path = rel,
        (Slot::Video, Element::Image { video, .. }) => *video = Some(rel),
        (Slot::ItemImage, Element::Carousel { item_image, .. })
        | (Slot::ItemImage, Element::Grid { item_image, .. })
        | (Slot::ItemImage, Element::Rows { item_image, .. }) => *item_image = rel,
        (Slot::SelectorImage, Element::TextList { selector_image, .. }) => *selector_image = Some(rel),
        (Slot::StarFilled, Element::Rating { filled_path, .. }) => *filled_path = Some(rel),
        (Slot::StarEmpty, Element::Rating { unfilled_path, .. }) => *unfilled_path = Some(rel),
        _ => {}
    }
}

/// The selected element's TOML being edited.
#[derive(Default)]
pub struct RawPane {
    pub open: bool,
    /// (what it was made for, text, error).
    text: Option<(String, String)>,
    error: Option<String>,
}

impl RawPane {
    pub fn reset(&mut self) {
        self.text = None;
        self.error = None;
    }
}

/// What the inspector's controls share.
struct Ed<'a> {
    pal: &'a Pal,
    palette: &'a Palette,
    bind: &'a mut Option<BindTarget>,
    pick: &'a mut Option<(Id, String)>,
    fonts: Vec<String>,
    values: &'a HashMap<&'static str, String>,
    files: Vec<(Asset, Slot, &'static str)>,
    view: ViewKind,
    path: ElPath,
}

impl Ed<'_> {
    /// A text that may hold values: the field, "＋ Value" (the values picker), and what it shows
    /// now under it.
    fn text(&mut self, ui: &mut Ui, label: &str, s: &mut String, kinds: &[Use], multi: bool, id: Id) -> bool {
        let mut changed = false;
        if let Some((pid, v)) = self.pick.as_ref()
            && *pid == id
        {
            s.push_str(v);
            *self.pick = None;
            changed = true;
        }
        ui.label(RichText::new(label).size(12.5).color(self.pal.dim));
        ui.horizontal(|ui| {
            let w = ui.available_width() - 84.0;
            let edit = if multi {
                egui::TextEdit::multiline(s).desired_rows(2).desired_width(w).id(id)
            } else {
                egui::TextEdit::singleline(s).desired_width(w).id(id)
            };
            changed |= ui.add(edit).changed();
            if ui.button("＋ Value").on_hover_text("Insert a value like the game's name").clicked() {
                *self.bind = Some(BindTarget { id, kinds: kinds.to_vec() });
            }
        });
        if s.contains('{') {
            let shown = resolve(self.values, s);
            let shown = if shown.trim().is_empty() {
                "(empty for this game)".to_string()
            } else if kinds == [Use::Picture] {
                // A picture: its file name, not the whole path.
                let name = std::path::Path::new(shown.trim()).file_name().map(|n| n.to_string_lossy().into_owned());
                format!("a picture ({})", name.unwrap_or_default())
            } else {
                shown
            };
            ui.label(
                RichText::new(format!("Now: {}", shown.chars().take(120).collect::<String>()))
                    .size(11.5)
                    .color(self.pal.dim),
            );
        }
        changed
    }

    /// A picture source: game / collection art, or a theme file (pick one: it is copied into
    /// the theme folder).
    fn picture(&mut self, ui: &mut Ui, label: &str, s: &mut String, slot: Slot, id: Id) -> bool {
        let before = s.clone();
        let games = self.view != ViewKind::Collections;
        let mut options: Vec<(&str, &str)> = if games {
            vec![
                ("{art.cover}", "Cover"),
                ("{art.hero}", "Background"),
                ("{art.logo}", "Logo"),
                ("{art.banner}", "Banner"),
                ("{art.icon}", "Icon"),
                ("{game.consoleart.mark}", "Its console's name badge"),
                ("{game.consoleart.device}", "Its console's device"),
            ]
        } else {
            vec![
                ("{collection.hero}", "A recent game's background"),
                ("{collection.cover}", "A recent game's cover"),
                ("{collection.logo}", "A recent game's logo"),
            ]
        };
        options.extend([
            ("{collection.art.card}", "Console card"),
            ("{collection.art.hero}", "Console background"),
            ("{collection.art.mark}", "Console name badge"),
            ("{collection.art.device}", "Console device"),
        ]);
        row(ui, label, self.pal, |ui| {
            let cur = options.iter().find(|(k, _)| k == s).map(|(_, l)| l.to_string()).unwrap_or_else(|| {
                if s.trim().is_empty() {
                    "None".into()
                } else if s.contains('{') {
                    "Custom".into()
                } else {
                    "A file".into()
                }
            });
            egui::ComboBox::from_id_salt(id.with("src")).selected_text(cur).width(150.0).show_ui(ui, |ui| {
                for (k, l) in &options {
                    if ui.selectable_label(s == k, *l).clicked() {
                        *s = k.to_string();
                    }
                }
                ui.separator();
                if ui.selectable_label(false, "A file…").clicked() {
                    self.files.push((Asset::Picture, slot, "Choose a picture"));
                }
                if ui.selectable_label(false, "None").clicked() {
                    s.clear();
                }
            });
            if ui.small_button("File…").on_hover_text("Pick a picture file (copied into the theme)").clicked() {
                self.files.push((Asset::Picture, slot, "Choose a picture"));
            }
        });
        self.text(ui, "", s, &[Use::Picture], false, id);
        *s != before
    }

    fn style(&mut self, ui: &mut Ui, st: &mut TextStyle, id: Id) -> bool {
        let before = st.clone();
        row(ui, "Font", self.pal, |ui| {
            let cur = st.font.clone().unwrap_or_else(|| "regular".into());
            egui::ComboBox::from_id_salt(id.with("font")).selected_text(font_label(&cur)).width(150.0).show_ui(
                ui,
                |ui| {
                    for f in &self.fonts {
                        if ui.selectable_label(cur == *f, font_label(f)).clicked() {
                            st.font = if f == "regular" { None } else { Some(f.clone()) };
                        }
                    }
                },
            );
        });
        row(ui, "Size", self.pal, |ui| {
            ui.spacing_mut().slider_width = 110.0;
            let mut px = st.font_size * 1080.0;
            if ui
                .add(egui::Slider::new(&mut px, 8.0..=200.0).suffix(" px").fixed_decimals(0).logarithmic(true))
                .changed()
            {
                st.font_size = super::geom::tidy(px / 1080.0);
            }
        });
        row(ui, "Colour", self.pal, |ui| color(ui, &mut st.color, self.palette, self.pal));
        row(ui, "Align", self.pal, |ui| {
            widgets::choice(
                ui,
                id.with("al"),
                &mut st.align,
                &[(Align::Left, "Left"), (Align::Center, "Centre"), (Align::Right, "Right")],
            );
            widgets::choice(
                ui,
                id.with("va"),
                &mut st.valign,
                &[(VAlign::Top, "Top"), (VAlign::Center, "Middle"), (VAlign::Bottom, "Bottom")],
            );
        });
        row(ui, "Letters", self.pal, |ui| {
            ui.checkbox(&mut st.uppercase, "CAPITALS");
            ui.add(
                egui::DragValue::new(&mut st.letter_spacing)
                    .speed(0.005)
                    .range(-0.2..=1.0)
                    .prefix("spacing ")
                    .fixed_decimals(2),
            );
        });
        row(ui, "Line height", self.pal, |ui| {
            ui.add(
                egui::DragValue::new(&mut st.line_spacing).speed(0.01).range(0.6..=3.0).fixed_decimals(2).suffix("×"),
            );
        });
        row(ui, "Shadow", self.pal, |ui| opt_color(ui, &mut st.shadow, Color([0, 0, 0, 140]), self.palette, self.pal));
        *st != before
    }
}

fn font_label(f: &str) -> String {
    match f {
        "regular" => "Regular".into(),
        "bold" => "Bold".into(),
        "title" => "Title".into(),
        other => other.to_string(),
    }
}

/// `text` with its values as the preview shows them.
pub fn resolve(values: &HashMap<&'static str, String>, text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                out.push_str(values.get(after[..close].trim()).map(String::as_str).unwrap_or(""));
                rest = &after[close + 1..];
            }
            None => {
                out.push_str(&rest[open..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// A "when does it show" control for `visible_if` / `hidden_if`.
fn condition(ui: &mut Ui, ed: &mut Ed, c: &mut Common, id: Id) -> bool {
    let before = (c.visible_if.clone(), c.hidden_if.clone());
    #[derive(PartialEq, Clone, Copy)]
    enum Mode {
        Always,
        Has,
        Lacks,
        Custom,
    }
    let single = |s: &str| {
        let t = s.trim();
        (t.starts_with('{') && t.ends_with('}') && t.matches('{').count() == 1).then(|| t[1..t.len() - 1].to_string())
    };
    let mode = match (&c.visible_if, &c.hidden_if) {
        (None, None) => Mode::Always,
        (Some(v), None) if single(v).is_some() => Mode::Has,
        (None, Some(h)) if single(h).is_some() => Mode::Lacks,
        _ => Mode::Custom,
    };
    let mut m = mode;
    row(ui, "Shows", ed.pal, |ui| {
        widgets::choice(
            ui,
            id.with("mode"),
            &mut m,
            &[
                (Mode::Always, "Always"),
                (Mode::Has, "When a value is there"),
                (Mode::Lacks, "When a value is missing"),
                (Mode::Custom, "When… (a condition)"),
            ],
        );
    });
    if m != mode {
        match m {
            Mode::Always => {
                c.visible_if = None;
                c.hidden_if = None;
            }
            Mode::Has => {
                c.visible_if =
                    Some(c.visible_if.clone().or(c.hidden_if.clone()).unwrap_or_else(|| "{art.logo}".into()));
                c.hidden_if = None;
            }
            Mode::Lacks => {
                c.hidden_if = Some(c.hidden_if.clone().or(c.visible_if.clone()).unwrap_or_else(|| "{art.logo}".into()));
                c.visible_if = None;
            }
            Mode::Custom => {
                c.visible_if = Some(c.visible_if.clone().unwrap_or_else(|| "{game.achievementstotal} > 0".into()));
            }
        }
    }
    match m {
        Mode::Always => {}
        Mode::Has | Mode::Lacks => {
            let field = if m == Mode::Has { &mut c.visible_if } else { &mut c.hidden_if };
            let mut s = field.clone().unwrap_or_default();
            if ed.text(ui, "Value", &mut s, &[], false, id.with("cond")) {
                *field = (!s.trim().is_empty()).then_some(s);
            }
        }
        Mode::Custom => {
            let mut s = c.visible_if.clone().unwrap_or_default();
            if ed.text(ui, "Shows when", &mut s, &[], false, id.with("vis")) {
                c.visible_if = (!s.trim().is_empty()).then_some(s);
            }
            let mut h = c.hidden_if.clone().unwrap_or_default();
            if ed.text(ui, "Hidden when", &mut h, &[], false, id.with("hid")) {
                c.hidden_if = (!h.trim().is_empty()).then_some(h);
            }
            note(ui, "Like {game.year} > 2010, {game.platform} == PC, !{art.logo}; combine with && and ||.", ed.pal);
        }
    }
    (c.visible_if.clone(), c.hidden_if.clone()) != before
}

fn place(ui: &mut Ui, ed: &mut Ed, c: &mut Common, nested: bool, id: Id) {
    heading(ui, "Place", ed.pal);
    if nested {
        note(ui, "Inside a stack or a list item: the stack places it; size 0 = from its content.", ed.pal);
    } else {
        row(ui, "Position", ed.pal, |ui| {
            pct(ui, &mut c.pos[0], -1.0..=2.0).on_hover_text("From the left");
            pct(ui, &mut c.pos[1], -1.0..=2.0).on_hover_text("From the top");
        });
    }
    row(ui, "Size", ed.pal, |ui| {
        for (i, tip) in [(0, "Width (0 = automatic)"), (1, "Height (0 = automatic)")] {
            pct(ui, &mut c.size[i], 0.0..=2.0).on_hover_text(tip);
        }
    });
    if !nested {
        row(ui, "Anchor", ed.pal, |ui| {
            // 3 × 3 anchor picker: which point of the element sits at the position.
            let (rect, _) = ui.allocate_exact_size(vec2(42.0, 42.0), egui::Sense::hover());
            for y in 0..3 {
                for x in 0..3 {
                    let o = [x as f32 * 0.5, y as f32 * 0.5];
                    let ctr = rect.min + vec2(7.0 + x as f32 * 14.0, 7.0 + y as f32 * 14.0);
                    let r = egui::Rect::from_center_size(ctr, vec2(11.0, 11.0));
                    let resp = ui.interact(r, id.with(("anchor", x, y)), egui::Sense::click());
                    let on = (c.origin[0] - o[0]).abs() < 0.01 && (c.origin[1] - o[1]).abs() < 0.01;
                    ui.painter().rect_filled(
                        r,
                        2.0,
                        if on {
                            ed.pal.accent
                        } else if resp.hovered() {
                            ed.pal.hover
                        } else {
                            ed.pal.surface_alt
                        },
                    );
                    if resp.clicked() && !on {
                        // Keep the element where it is on screen.
                        let w = c.size[0];
                        let h = c.size[1];
                        c.pos[0] = super::geom::tidy(c.pos[0] + (o[0] - c.origin[0]) * w);
                        c.pos[1] = super::geom::tidy(c.pos[1] + (o[1] - c.origin[1]) * h);
                        c.origin = o;
                    }
                }
            }
            note(ui, "the point that sits at the position", ed.pal);
        });
        row(ui, "Layer (z)", ed.pal, |ui| {
            ui.add(egui::DragValue::new(&mut c.z).speed(0.1)).on_hover_text("Higher is drawn in front");
        });
    } else {
        let mut m = c.margin.unwrap_or([0.0; 4]);
        row(ui, "Room around", ed.pal, |ui| {
            for v in m.iter_mut() {
                pct(ui, v, 0.0..=0.5);
            }
        });
        if m != c.margin.unwrap_or([0.0; 4]) {
            c.margin = (m != [0.0; 4]).then_some(m);
        }
        row(ui, "Gives way", ed.pal, |ui| ui.checkbox(&mut c.shrink, "to what follows it"));
    }
    row(ui, "Opacity", ed.pal, |ui| ui.add(egui::Slider::new(&mut c.opacity, 0.0..=1.0).fixed_decimals(2)));
    row(ui, "Turn", ed.pal, |ui| {
        ui.add(egui::DragValue::new(&mut c.rotation).speed(0.5).range(-360.0..=360.0).suffix("°"))
    });
}

fn transitions(ui: &mut Ui, ed: &mut Ed, c: &mut Common, list: bool, id: Id) {
    heading(ui, "Motion", ed.pal);
    let mut t = c.transitions.as_deref().cloned().unwrap_or_default();
    let before = t.clone();
    let slots: Vec<(&str, &mut Option<Transition>)> = if list {
        vec![
            ("When an item gets the focus", &mut t.focus),
            ("When it loses it", &mut t.unfocus),
            ("When it appears", &mut t.appear),
        ]
    } else {
        vec![("When it appears", &mut t.appear), ("When it goes", &mut t.disappear)]
    };
    for (i, (label, slot)) in slots.into_iter().enumerate() {
        let mut on = slot.is_some();
        if ui.checkbox(&mut on, label).changed() {
            *slot = on.then(|| Transition {
                scale: list.then_some(1.08),
                opacity: (!list).then_some(0.0),
                duration_ms: 220,
                easing: Easing::CubicOut,
                ..Default::default()
            });
        }
        if let Some(tr) = slot {
            ui.indent(id.with(("tr", i)), |ui| {
                let mut sc = tr.scale.is_some();
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut sc, "Grow to").changed() {
                        tr.scale = sc.then_some(1.08);
                    }
                    if let Some(s) = &mut tr.scale {
                        ui.add(egui::DragValue::new(s).speed(0.01).range(0.2..=3.0).suffix("×"));
                    }
                });
                let mut op = tr.opacity.is_some();
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut op, "Fade from / to").changed() {
                        tr.opacity = op.then_some(0.0);
                    }
                    if let Some(o) = &mut tr.opacity {
                        ui.add(egui::Slider::new(o, 0.0..=1.0).fixed_decimals(2));
                    }
                });
                let mut mv = tr.translate.is_some();
                ui.horizontal(|ui| {
                    if ui.checkbox(&mut mv, "Slide by").changed() {
                        tr.translate = mv.then_some([0.0, 0.03]);
                    }
                    if let Some(m) = &mut tr.translate {
                        widgets::pct2(ui, m, -1.0..=1.0);
                    }
                });
                ui.horizontal(|ui| {
                    ui.add(egui::DragValue::new(&mut tr.duration_ms).speed(5).range(0..=5000).suffix(" ms"));
                    ui.add(
                        egui::DragValue::new(&mut tr.delay_ms).speed(5).range(0..=5000).prefix("after ").suffix(" ms"),
                    );
                    widgets::choice(
                        ui,
                        id.with(("ease", i)),
                        &mut tr.easing,
                        &[
                            (Easing::Linear, "Steady"),
                            (Easing::EaseOut, "Ease out"),
                            (Easing::CubicOut, "Ease out (strong)"),
                            (Easing::EaseInOut, "Ease in and out"),
                            (Easing::EaseIn, "Ease in"),
                        ],
                    );
                });
            });
        }
    }
    if t != before {
        c.transitions = (t != Transitions::default()).then(|| Box::new(t));
    }
}

/// Settings that only lists have (carousel, grid, rows).
fn list_look(ui: &mut Ui, ed: &mut Ed, c: &mut Common, games: bool, id: Id) {
    row(ui, "Pictures", ed.pal, |ui| {
        let mut fit = c.item_fit.unwrap_or_default();
        if widgets::choice(
            ui,
            id.with("fit"),
            &mut fit,
            &[(Fit::Contain, "Whole picture"), (Fit::Cover, "Fill (crop)"), (Fit::Stretch, "Stretch")],
        ) {
            c.item_fit = Some(fit);
        }
    });
    row(ui, "Corners", ed.pal, |ui| widgets::opt_num(ui, &mut c.radius, 0.012, true, 0.0..=0.2, ed.pal));
    row(ui, "Card colour", ed.pal, |ui| opt_color(ui, &mut c.item_fill, ed.palette.surface_alt, ed.palette, ed.pal));
    let mut cap = c.item_caption.clone();
    let mut s = cap.clone().unwrap_or_default();
    if ed.text(ui, "Caption over pictures", &mut s, &[Use::Text], false, id.with("cap")) {
        cap = (!s.trim().is_empty()).then_some(s);
        c.item_caption = cap;
    }
    let mut sub = c.item_subtext.clone().unwrap_or_default();
    if ed.text(ui, "Second line", &mut sub, &[Use::Text], false, id.with("sub")) {
        c.item_subtext = (!sub.trim().is_empty()).then_some(sub);
    }
    row(ui, "Focus border", ed.pal, |ui| {
        widgets::opt_num(ui, &mut c.selected_border_width, 0.004, true, 0.0..=0.05, ed.pal)
    });
    if games {
        row(ui, "Badges", ed.pal, |ui| {
            ui.checkbox(&mut c.favorite_badge, "★ favourites");
            ui.checkbox(&mut c.controller_badge, "🎮 controller");
        });
    }
    row(ui, "Brightness", ed.pal, |ui| ui.add(egui::Slider::new(&mut c.brightness, -2.0..=2.0).fixed_decimals(2)));
    row(ui, "Colourfulness", ed.pal, |ui| ui.add(egui::Slider::new(&mut c.saturation, 0.0..=2.0).fixed_decimals(2)));
}

/// The settings of one kind of element.
fn kind_settings(ui: &mut Ui, ed: &mut Ed, e: &mut Element, id: Id) {
    let games = ed.view != ViewKind::Collections;
    let pal = ed.pal;
    match e {
        Element::Image {
            common,
            path,
            fallbacks,
            fit,
            tint,
            tile,
            corner_radius,
            gradient,
            fallback_text,
            video,
            ..
        } => {
            heading(ui, "Picture", pal);
            ed.picture(ui, "Shows", path, Slot::Path, id.with("path"));
            let mut fb = fallbacks.join("\n");
            ui.label(RichText::new("If it's missing, try (one per line)").size(12.5).color(pal.dim));
            if ui.add(egui::TextEdit::multiline(&mut fb).desired_rows(1).desired_width(f32::INFINITY)).changed() {
                *fallbacks = fb.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
            }
            row(ui, "Fit", pal, |ui| {
                widgets::choice(
                    ui,
                    id.with("fit"),
                    fit,
                    &[(Fit::Contain, "Whole picture"), (Fit::Cover, "Fill (crop)"), (Fit::Stretch, "Stretch")],
                )
            });
            row(ui, "Corners", pal, |ui| pct(ui, corner_radius, 0.0..=0.3));
            row(ui, "Tint", pal, |ui| opt_color(ui, tint, Color::WHITE, ed.palette, pal));
            row(ui, "Repeat", pal, |ui| ui.checkbox(tile, "tile it"));
            row(ui, "Blur", pal, |ui| pct(ui, &mut common.blur, 0.0..=0.1));
            row(ui, "Brightness", pal, |ui| {
                ui.add(egui::Slider::new(&mut common.brightness, -2.0..=2.0).fixed_decimals(2))
            });
            row(ui, "Colourfulness", pal, |ui| {
                ui.add(egui::Slider::new(&mut common.saturation, 0.0..=2.0).fixed_decimals(2))
            });
            let mut fade_edges = common.mask.unwrap_or([0.0; 4]);
            row(ui, "Fade edges", pal, |ui| {
                for (v, tip) in fade_edges.iter_mut().zip(["left", "top", "right", "bottom"]) {
                    pct(ui, v, 0.0..=1.0).on_hover_text(tip);
                }
            });
            if fade_edges != common.mask.unwrap_or([0.0; 4]) {
                common.mask = (fade_edges != [0.0; 4]).then_some(fade_edges);
            }
            row(ui, "Cross-fade", pal, |ui| widgets::opt_num(ui, &mut common.fade, 0.3, false, 0.0..=3.0, pal));
            let mut g = gradient.is_some();
            row(ui, "Shade over it", pal, |ui| {
                if ui.checkbox(&mut g, "").changed() {
                    *gradient =
                        g.then_some([ed.palette.background.with_alpha(0), ed.palette.background.with_alpha(230)]);
                }
                if let Some([a, b]) = gradient {
                    ui.vertical(|ui| {
                        color(ui, a, ed.palette, pal);
                        color(ui, b, ed.palette, pal);
                    });
                }
            });
            if path.contains("{art.") || path.contains("{collection.") {
                row(ui, "No art", pal, |ui| ui.checkbox(&mut common.placeholder, "draw a soft stand-in"));
            }
            let mut ft = fallback_text.clone().unwrap_or_default();
            if ed.text(ui, "Text when there's no picture", &mut ft, &[Use::Text], false, id.with("ftext")) {
                *fallback_text = (!ft.trim().is_empty()).then_some(ft);
            }
            heading(ui, "Video", pal);
            row(ui, "Video", pal, |ui| {
                let label = video.clone().filter(|v| !v.is_empty()).unwrap_or_else(|| "none".into());
                ui.add(egui::Label::new(RichText::new(label).size(12.0)).truncate());
                if ui.small_button("File…").on_hover_text("Pick a video file (copied into the theme)").clicked() {
                    ed.files.push((Asset::Video, Slot::Video, "Choose a video"));
                }
                if video.is_some() && ui.small_button("✕").clicked() {
                    *video = None;
                }
            });
            note(ui, "Plays in the box when ffmpeg is installed; the picture shows meanwhile.", pal);
        }
        Element::Text { common, text, style, background, wrap, max_lines, scroll } => {
            heading(ui, "Text", pal);
            let tid = if ed.path.sub.is_empty() { Id::unique("ls_creator_text_field") } else { id.with("text") };
            ed.text(ui, "Says", text, &[Use::Text], true, tid);
            row(ui, "Long text", pal, |ui| {
                ui.checkbox(wrap, "wraps");
                ui.checkbox(scroll, "scrolls slowly");
            });
            row(ui, "At most", pal, |ui| {
                let mut on = max_lines.is_some();
                if ui.checkbox(&mut on, "").changed() {
                    *max_lines = on.then_some(2);
                }
                if let Some(n) = max_lines {
                    ui.add(egui::DragValue::new(n).range(1..=40).suffix(" lines"));
                }
            });
            row(ui, "As pills", pal, |ui| ui.checkbox(&mut common.chips, "each part between | is a pill"));
            heading(ui, "Look", pal);
            ed.style(ui, style, id.with("style"));
            row(ui, "Box behind", pal, |ui| {
                opt_color(ui, background, ed.palette.surface.with_alpha(200), ed.palette, pal)
            });
            if background.is_some() || common.chips {
                row(ui, "Padding", pal, |ui| {
                    let mut p = common.padding.unwrap_or([0.0; 2]);
                    if widgets::pct2(ui, &mut p, 0.0..=0.2) {
                        common.padding = Some(p);
                    }
                });
                row(ui, "Corners", pal, |ui| widgets::opt_num(ui, &mut common.radius, 0.01, true, 0.0..=0.2, pal));
            }
        }
        Element::Rect { common, color: c, corner_radius, border_color, border_width, gradient, fill, track } => {
            heading(ui, "Shape", pal);
            row(ui, "Colour", pal, |ui| color(ui, c, ed.palette, pal));
            row(ui, "Corners", pal, |ui| pct(ui, corner_radius, 0.0..=0.5));
            row(ui, "Border", pal, |ui| opt_color(ui, border_color, ed.palette.border, ed.palette, pal));
            if border_color.is_some() {
                row(ui, "Border width", pal, |ui| pct(ui, border_width, 0.0..=0.05));
            }
            let mut g = gradient.is_some();
            row(ui, "Gradient", pal, |ui| {
                if ui.checkbox(&mut g, "").changed() {
                    *gradient = g.then_some([c.with_alpha(0), *c]);
                }
                if let Some([a, b]) = gradient {
                    ui.vertical(|ui| {
                        color(ui, a, ed.palette, pal);
                        color(ui, b, ed.palette, pal);
                        ui.checkbox(&mut common.gradient_horizontal, "left to right");
                    });
                }
            });
            let mut bar = fill.clone().unwrap_or_default();
            heading(ui, "Progress bar", pal);
            if ed.text(ui, "Filled by (0 to 1)", &mut bar, &[Use::Share], false, id.with("fill")) {
                *fill = (!bar.trim().is_empty()).then_some(bar);
            }
            if fill.is_some() {
                row(ui, "Track", pal, |ui| opt_color(ui, track, ed.palette.surface_alt, ed.palette, pal));
            }
        }
        Element::Carousel {
            common,
            orientation,
            item_size,
            visible_items,
            item_image,
            item_text,
            item_text_style,
            selected_scale,
            unselected_opacity,
            item_align,
            selected_position,
            wheel_angle,
            selected_border,
            background,
            ..
        } => {
            heading(ui, "Carousel", pal);
            row(ui, "Runs", pal, |ui| {
                widgets::choice(
                    ui,
                    id.with("or"),
                    orientation,
                    &[
                        (Orientation::Horizontal, "Across"),
                        (Orientation::Vertical, "Down"),
                        (Orientation::Wheel, "On a curve"),
                        (Orientation::VerticalWheel, "Wheel, down"),
                        (Orientation::HorizontalWheel, "Wheel, across"),
                    ],
                )
            });
            row(ui, "Item size", pal, |ui| widgets::pct2(ui, item_size, 0.01..=1.0));
            row(ui, "Items shown", pal, |ui| {
                ui.add(egui::DragValue::new(visible_items).speed(0.05).range(1.0..=30.0).fixed_decimals(1))
            });
            row(ui, "Selection at", pal, |ui| {
                ui.add(egui::Slider::new(selected_position, 0.0..=1.0).fixed_decimals(2))
            });
            row(ui, "Items line up", pal, |ui| ui.add(egui::Slider::new(item_align, 0.0..=1.0).fixed_decimals(2)));
            row(ui, "Selected grows", pal, |ui| {
                ui.add(egui::DragValue::new(selected_scale).speed(0.01).range(0.3..=3.0).suffix("×"))
            });
            row(ui, "Others", pal, |ui| ui.add(egui::Slider::new(unselected_opacity, 0.0..=1.0)));
            if orientation.is_arc_wheel() || *orientation == Orientation::Wheel {
                row(ui, "Wheel angle", pal, |ui| {
                    ui.add(egui::DragValue::new(wheel_angle).speed(0.2).range(-90.0..=90.0).suffix("°"))
                });
            }
            row(ui, "Focus colour", pal, |ui| opt_color(ui, selected_border, ed.palette.selection, ed.palette, pal));
            row(ui, "Strip colour", pal, |ui| opt_color(ui, background, ed.palette.surface, ed.palette, pal));
            heading(ui, "Items", pal);
            ed.picture(ui, "Item picture", item_image, Slot::ItemImage, id.with("ii"));
            ed.text(ui, "Item text (no picture)", item_text, &[Use::Text], false, id.with("it"));
            ed.style(ui, item_text_style, id.with("its"));
            list_look(ui, ed, common, games, id);
        }
        Element::Grid {
            common,
            columns,
            item_aspect,
            spacing,
            item_image,
            item_text,
            item_text_style,
            show_labels,
            selected_scale,
            selected_border,
            item_background,
            corner_radius,
            unselected_opacity,
            ..
        } => {
            heading(ui, "Grid", pal);
            row(ui, "Columns", pal, |ui| ui.add(egui::DragValue::new(columns).range(1..=20)));
            row(ui, "Item shape", pal, |ui| {
                ui.add(egui::DragValue::new(item_aspect).speed(0.01).range(0.2..=4.0).fixed_decimals(2))
                    .on_hover_text("Height ÷ width (1.5 = a cover)");
                if ui.small_button("Cover").clicked() {
                    *item_aspect = 1.5;
                }
                if ui.small_button("Wide").clicked() {
                    *item_aspect = 0.47;
                }
                if ui.small_button("Square").clicked() {
                    *item_aspect = 1.0;
                }
            });
            row(ui, "Gap", pal, |ui| pct(ui, spacing, 0.0..=0.2));
            row(ui, "Selected grows", pal, |ui| {
                ui.add(egui::DragValue::new(selected_scale).speed(0.01).range(0.3..=3.0).suffix("×"))
            });
            row(ui, "Others", pal, |ui| ui.add(egui::Slider::new(unselected_opacity, 0.0..=1.0)));
            row(ui, "Focus colour", pal, |ui| opt_color(ui, selected_border, ed.palette.selection, ed.palette, pal));
            row(ui, "Card colour", pal, |ui| opt_color(ui, item_background, ed.palette.surface_alt, ed.palette, pal));
            row(ui, "Corners", pal, |ui| pct(ui, corner_radius, 0.0..=0.2));
            row(ui, "Names", pal, |ui| ui.checkbox(show_labels, "under every item"));
            heading(ui, "Items", pal);
            ed.picture(ui, "Item picture", item_image, Slot::ItemImage, id.with("ii"));
            ed.text(ui, "Item text", item_text, &[Use::Text], false, id.with("it"));
            ed.style(ui, item_text_style, id.with("its"));
            list_look(ui, ed, common, games, id);
        }
        Element::TextList {
            item_text,
            style,
            selected_color,
            selector_color,
            row_height,
            margin,
            favorite_color,
            corner_radius,
            selector_width,
            selector_image,
            ..
        } => {
            heading(ui, "List", pal);
            ed.text(ui, "Each row says", item_text, &[Use::Text], false, id.with("it"));
            row(ui, "Row height", pal, |ui| {
                ui.add(egui::DragValue::new(row_height).speed(0.01).range(0.8..=5.0).suffix("× text"))
            });
            row(ui, "Inner margin", pal, |ui| pct(ui, margin, 0.0..=0.2));
            row(ui, "Selected text", pal, |ui| opt_color(ui, selected_color, ed.palette.accent_text, ed.palette, pal));
            row(ui, "Selection bar", pal, |ui| opt_color(ui, selector_color, ed.palette.accent, ed.palette, pal));
            row(ui, "Bar width", pal, |ui| widgets::opt_num(ui, selector_width, 0.006, true, 0.0..=1.0, pal));
            row(ui, "Bar corners", pal, |ui| pct(ui, corner_radius, 0.0..=0.1));
            row(ui, "Favourites", pal, |ui| opt_color(ui, favorite_color, ed.palette.favorite, ed.palette, pal));
            row(ui, "Bar picture", pal, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(selector_image.clone().unwrap_or_else(|| "none".into())).size(12.0))
                        .truncate(),
                );
                if ui.small_button("File…").on_hover_text("A picture stretched over the selected row").clicked() {
                    ed.files.push((Asset::Picture, Slot::SelectorImage, "Choose a picture for the selection bar"));
                }
                if selector_image.is_some() && ui.small_button("✕").clicked() {
                    *selector_image = None;
                }
            });
            heading(ui, "Look", pal);
            ed.style(ui, style, id.with("style"));
        }
        Element::Rows {
            rows,
            max_rows,
            row_items,
            item_size,
            spacing,
            row_spacing,
            title,
            title_style,
            item_image,
            selected_scale,
            selected_border,
            unselected_opacity,
            common,
            ..
        } => {
            heading(ui, "Cover rows", pal);
            ui.label(RichText::new("Rows, in order").size(12.5).color(pal.dim));
            for (k, l) in [
                ("recent", "Recently played"),
                ("favorites", "Favourites"),
                ("installed", "Installed"),
                ("controller", "Controller-friendly"),
                ("platforms", "One per console"),
                ("sources", "One per store"),
                ("collections", "Your collections"),
                ("all", "All games"),
            ] {
                let mut on = rows.iter().any(|r| r == k);
                if ui.checkbox(&mut on, l).changed() {
                    if on {
                        rows.push(k.to_string());
                    } else {
                        rows.retain(|r| r != k);
                    }
                }
            }
            row(ui, "At most", pal, |ui| {
                ui.add(egui::DragValue::new(max_rows).range(1..=30).suffix(" rows"));
                ui.add(egui::DragValue::new(row_items).range(1..=200).suffix(" games each"));
            });
            row(ui, "Item size", pal, |ui| widgets::pct2(ui, item_size, 0.01..=1.0));
            row(ui, "Gaps", pal, |ui| {
                pct(ui, spacing, 0.0..=0.2).on_hover_text("Between covers");
                pct(ui, row_spacing, 0.0..=0.3).on_hover_text("Between rows");
            });
            row(ui, "Selected grows", pal, |ui| {
                ui.add(egui::DragValue::new(selected_scale).speed(0.01).range(0.3..=3.0).suffix("×"))
            });
            row(ui, "Other rows", pal, |ui| ui.add(egui::Slider::new(unselected_opacity, 0.0..=1.0)));
            row(ui, "Focus colour", pal, |ui| opt_color(ui, selected_border, ed.palette.selection, ed.palette, pal));
            ed.text(ui, "Row title", title, &[Use::Text], false, id.with("title"));
            ed.style(ui, title_style, id.with("ts"));
            ed.picture(ui, "Item picture", item_image, Slot::ItemImage, id.with("ii"));
            list_look(ui, ed, common, true, id);
        }
        Element::Rail { style, row_height, current_color, selector_color, background, compact, entries, .. } => {
            heading(ui, "Navigation rail", pal);
            row(ui, "Labels", pal, |ui| ui.checkbox(compact, "only while it has the focus"));
            row(ui, "Row height", pal, |ui| {
                ui.add(egui::DragValue::new(row_height).speed(0.01).range(1.0..=6.0).suffix("× text"))
            });
            row(ui, "Open section", pal, |ui| opt_color(ui, current_color, ed.palette.accent, ed.palette, pal));
            row(ui, "Selection", pal, |ui| {
                opt_color(ui, selector_color, ed.palette.accent.with_alpha(60), ed.palette, pal)
            });
            row(ui, "Panel", pal, |ui| opt_color(ui, background, ed.palette.surface, ed.palette, pal));
            ui.label(RichText::new("Sections").size(12.5).color(pal.dim));
            let mut keys: Vec<String> =
                crate::theme::render::rail_entries(entries).iter().map(|e| e.key.clone()).collect();
            let before = keys.clone();
            for k in RailEntry::KEYS {
                let mut on = keys.iter().any(|x| x == k);
                let label = RailEntry { key: k.into(), label: String::new(), icon: None }.label().to_string();
                if ui.checkbox(&mut on, label).changed() {
                    if on {
                        keys.push(k.into());
                    } else {
                        keys.retain(|x| x != k);
                    }
                }
            }
            if keys != before {
                *entries = keys.into_iter().map(|key| RailEntry { key, label: String::new(), icon: None }).collect();
            }
            heading(ui, "Look", pal);
            ed.style(ui, style, id.with("style"));
        }
        Element::Rating { value, color: c, .. } => {
            heading(ui, "Stars", pal);
            ed.text(ui, "Value (0 to 1)", value, &[Use::Share], false, id.with("v"));
            row(ui, "Colour", pal, |ui| color(ui, c, ed.palette, pal));
            row(ui, "Star pictures", pal, |ui| {
                if ui.small_button("Filled…").clicked() {
                    ed.files.push((Asset::Picture, Slot::StarFilled, "Choose a filled star"));
                }
                if ui.small_button("Empty…").clicked() {
                    ed.files.push((Asset::Picture, Slot::StarEmpty, "Choose an empty star"));
                }
            });
        }
        Element::DateTime { value, style, clock, format, background, .. } => {
            heading(ui, if *clock { "Clock" } else { "Date" }, pal);
            row(ui, "Shows", pal, |ui| {
                ui.radio_value(clock, true, "the time");
                ui.radio_value(clock, false, "a date");
            });
            if !*clock {
                ed.text(ui, "Date of", value, &[Use::Text], false, id.with("v"));
            }
            row(ui, "Format", pal, |ui| {
                let opts: &[(&str, &str)] = if *clock {
                    &[("%H:%M", "13:45"), ("%I:%M %p", "1:45 PM")]
                } else {
                    &[
                        ("relative", "3 days ago"),
                        ("%d %b %Y", "24 Nov 2026"),
                        ("%Y-%m-%d", "2026-11-24"),
                        ("%b %Y", "Nov 2026"),
                    ]
                };
                let mut f = format.clone();
                egui::ComboBox::from_id_salt(id.with("fmt"))
                    .selected_text(opts.iter().find(|(k, _)| *k == f).map_or(f.as_str(), |(_, l)| *l))
                    .show_ui(ui, |ui| {
                        for (k, l) in opts {
                            ui.selectable_value(&mut f, k.to_string(), *l);
                        }
                    });
                *format = f;
            });
            row(ui, "Box behind", pal, |ui| opt_color(ui, background, ed.palette.surface, ed.palette, pal));
            ed.style(ui, style, id.with("style"));
        }
        Element::Badges { slots, per_line, align, color: c, .. } => {
            heading(ui, "Badges", pal);
            for (k, l) in [("favorite", "Favourite"), ("completed", "Completed"), ("controller", "Controller support")]
            {
                let mut on = slots.iter().any(|s| s == k);
                if ui.checkbox(&mut on, l).changed() {
                    if on {
                        slots.push(k.into());
                    } else {
                        slots.retain(|s| s != k);
                    }
                }
            }
            row(ui, "Per row", pal, |ui| ui.add(egui::DragValue::new(per_line).range(1..=10)));
            row(ui, "Start at", pal, |ui| {
                widgets::choice(ui, id.with("al"), align, &[(Align::Left, "Left"), (Align::Right, "Right")])
            });
            row(ui, "Tint", pal, |ui| opt_color(ui, c, Color::WHITE, ed.palette, pal));
        }
        Element::Stack { direction, gap, align, justify, background, corner_radius, common, .. } => {
            heading(ui, "Stack", pal);
            row(ui, "Flows", pal, |ui| {
                widgets::choice(
                    ui,
                    id.with("dir"),
                    direction,
                    &[(Direction::Vertical, "Down"), (Direction::Horizontal, "Across")],
                )
            });
            row(ui, "Gap", pal, |ui| pct(ui, gap, 0.0..=0.2));
            let aligns = [
                (StackAlign::Start, "Start"),
                (StackAlign::Center, "Centre"),
                (StackAlign::End, "End"),
                (StackAlign::Stretch, "Stretch"),
            ];
            row(ui, "Parts across", pal, |ui| widgets::choice(ui, id.with("al"), align, &aligns));
            row(ui, "Parts along", pal, |ui| widgets::choice(ui, id.with("ju"), justify, &aligns[..3]));
            let mut bg = background.as_ref().map(|b| b.average());
            row(ui, "Background", pal, |ui| {
                if opt_color(ui, &mut bg, ed.palette.surface.with_alpha(200), ed.palette, pal) {
                    *background = bg.map(Brush::Solid);
                }
            });
            row(ui, "Corners", pal, |ui| pct(ui, corner_radius, 0.0..=0.2));
            row(ui, "Padding", pal, |ui| {
                let mut p = common.padding.unwrap_or([0.0; 2]);
                if widgets::pct2(ui, &mut p, 0.0..=0.2) {
                    common.padding = Some(p);
                }
            });
            note(ui, "Its parts are listed under it in the elements panel; empty ones take no room.", pal);
        }
        Element::HelpBar { style, icon_color, background, buttons, .. } => {
            heading(ui, "Button hints", pal);
            row(ui, "Button colour", pal, |ui| opt_color(ui, icon_color, ed.palette.text, ed.palette, pal));
            row(ui, "Box behind", pal, |ui| opt_color(ui, background, ed.palette.surface, ed.palette, pal));
            ui.label(RichText::new("Only these buttons (none ticked: all)").size(12.5).color(pal.dim));
            ui.horizontal_wrapped(|ui| {
                for b in ["a", "b", "x", "y", "start", "select", "lb", "rb"] {
                    let mut on = buttons.iter().any(|x| x == b);
                    if ui.checkbox(&mut on, b.to_uppercase()).changed() {
                        if on {
                            buttons.push(b.into());
                        } else {
                            buttons.retain(|x| x != b);
                        }
                    }
                }
            });
            ed.style(ui, style, id.with("style"));
        }
    }
}

pub fn show(ui: &mut Ui, st: &mut AppState, d: &mut CreatorDlg, pal: &Pal) {
    let ctx = ui.ctx().clone();
    ui.spacing_mut().slider_width = 100.0;
    let values = d.pv.binding_values(st, d.doc.as_ref(), d.view);
    let Some(doc) = d.doc.as_ref() else { return };
    let Some(p) = d.sel.clone() else {
        heading(ui, "Nothing selected", pal);
        note(ui, "Click an element in the preview or in the list to change it. ＋ Add puts in new ones.", pal);
        ui.add_space(8.0);
        view_settings(ui, d, pal);
        return;
    };
    let Some(view) = doc.file.view(d.view) else { return };
    let Some(base_top) = view.elements.get(p.top) else {
        d.sel = None;
        return;
    };
    // The element as the current layer has it.
    let patch = d.layer.set(&doc.file).and_then(|o| ops::tables(o, d.view).get(&base_top.common().name)).cloned();
    let shown_top = match &patch {
        Some(t) => crate::theme::render::merge_element(base_top, t).unwrap_or_else(|_| base_top.clone()),
        None => base_top.clone(),
    };
    let sub = ElPath { top: 0, sub: p.sub.clone() };
    let Some(shown) = ops::get(std::slice::from_ref(&shown_top), &sub).cloned() else {
        d.sel = None;
        return;
    };
    let mut fonts: Vec<String> = vec!["regular".into(), "bold".into(), "title".into()];
    fonts.extend(doc.file.fonts.extra.keys().cloned());
    let palette = doc.file.palette.clone();
    let z_front = view.elements.iter().map(|e| e.common().z).fold(0.0f32, f32::max) + 1.0;
    let z_back = view.elements.iter().map(|e| e.common().z).fold(0.0f32, f32::min) - 1.0;
    let mut el = shown.clone();
    let id = Id::unique(("ls_creator_insp", d.view, &p));
    let nested = !p.sub.is_empty();
    // Header.
    let (mut del, mut dup) = (false, false);
    ui.horizontal(|ui| {
        ui.label(RichText::new(ops::kind_word(&el)).size(15.0).strong().color(pal.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            del = crate::ui::widgets::icon_button(
                ui,
                crate::ui::widgets::Icon::Trash,
                false,
                22.0,
                "Delete (Delete key)",
            )
            .clicked();
            dup = ui.small_button("Duplicate").on_hover_text("Ctrl+D").clicked();
        });
    });
    if del {
        super::panel::delete_selected(d);
        return;
    }
    if dup {
        super::panel::duplicate_selected(d);
        return;
    }
    let mut requests: Vec<(Asset, Slot, &'static str)> = vec![];
    {
        let mut ed = Ed {
            pal,
            palette: &palette,
            bind: &mut d.bind,
            pick: &mut d.bind_pick,
            fonts,
            values: &values,
            files: vec![],
            view: d.view,
            path: p.clone(),
        };
        row(ui, "Name", pal, |ui| {
            let mut n = el.common().name.clone();
            if widgets::line(ui, &mut n, "a name (used by variants and consoles)", 170.0).changed()
                && d.layer == Layer::Base
            {
                el.common_mut().name = n.trim().to_string();
            }
        });
        if !nested {
            ui.horizontal(|ui| {
                if ui.small_button("Bring to front").clicked() {
                    el.common_mut().z = z_front;
                }
                if ui.small_button("Send to back").clicked() {
                    el.common_mut().z = z_back;
                }
                ui.checkbox(&mut el.common_mut().visible, "Shown");
            });
        }
        if let Some(_t) = &patch {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("● Changed for {}", super::preview::layer_label(&d.layer, st)))
                        .size(12.0)
                        .color(pal.accent),
                );
            });
        }
        kind_settings(ui, &mut ed, &mut el, id);
        place(ui, &mut ed, el.common_mut(), nested, id.with("place"));
        heading(ui, "When it shows", pal);
        condition(ui, &mut ed, el.common_mut(), id.with("cond"));
        let list = matches!(el, Element::Carousel { .. } | Element::Grid { .. } | Element::Rows { .. });
        transitions(ui, &mut ed, el.common_mut(), list, id.with("motion"));
        requests = std::mem::take(&mut ed.files);
    }
    if matches!(el, Element::Carousel { .. } | Element::Grid { .. } | Element::Rows { .. }) && !nested {
        heading(ui, "Parts over each item", pal);
        note(
            ui,
            &format!("{} part(s): a title strip, a badge … drawn over every item.", el.common().item_elements.len()),
            pal,
        );
        ui.horizontal(|ui| {
            if ui.small_button("＋ Title strip").clicked() {
                el.common_mut().item_elements.push(item_title(&palette));
            }
            if ui.small_button("＋ Focus glow text").clicked() {
                el.common_mut().item_elements.push(item_focus_text(&palette));
            }
        });
    }
    if let Element::Stack { children, .. } = &mut el
        && !nested
    {
        ui.horizontal(|ui| {
            if ui.small_button("＋ Text part").clicked() {
                children.push(Element::Text {
                    common: Common::default(),
                    text: "{game.name}".into(),
                    style: TextStyle { font_size: 0.03, color: palette.text, ..Default::default() },
                    background: None,
                    wrap: true,
                    max_lines: None,
                    scroll: false,
                });
            }
        });
    }
    if nested {
        ui.horizontal(|ui| {
            if ui.small_button("▲ Earlier").clicked() {
                d.sel = shift(d, &p, -1);
            }
            if ui.small_button("▼ Later").clicked() {
                d.sel = shift(d, &p, 1);
            }
        });
    }
    // Patch reset.
    if patch.is_some()
        && !nested
        && ui.button(format!("Undo the changes for {}", super::preview::layer_label(&d.layer, st))).clicked()
        && let Some(doc) = d.doc.as_mut()
    {
        let (layer, view, name) = (d.layer.clone(), d.view, base_top_name(doc, d.view, p.top));
        doc.change("Reset an element", |f| {
            ops::reset_in_layer(f, &layer, view, &name);
        });
        return;
    }
    raw_pane(ui, d, &shown, pal);
    if el != shown
        && widgets::user_acted(&ctx)
        && let Some(doc) = d.doc.as_mut()
    {
        let newel = el.clone();
        super::edit_element(doc, &ctx, &d.layer.clone(), d.view, &p, &mut d.banner, move |e| *e = newel);
    }
    for (kind, slot, title) in requests {
        d.pick(&ctx, PickFor::Asset(kind, AssetFor::Element(d.view, p.clone(), slot)), title);
    }
    let _ = ElPath::top;
    let _ = Nest::Children;
}

fn base_top_name(doc: &super::doc::Doc, view: ViewKind, top: usize) -> String {
    doc.file.view(view).and_then(|v| v.elements.get(top)).map(|e| e.common().name.clone()).unwrap_or_default()
}

fn shift(d: &mut CreatorDlg, p: &ElPath, by: isize) -> Option<ElPath> {
    let doc = d.doc.as_mut()?;
    let view = d.view;
    let mut np = None;
    doc.change("Move a part", |f| {
        if let Some(v) = super::panel::view_mut(f, view) {
            np = ops::shift_nested(&mut v.elements, p, by);
        }
    });
    np.or(Some(p.clone()))
}

fn item_title(pal: &Palette) -> Element {
    Element::Text {
        common: Common {
            name: "item_title".into(),
            pos: [0.0, 1.0],
            origin: [0.0, 1.0],
            size: [1.0, 0.0],
            padding: Some([0.06, 0.03]),
            ..Default::default()
        },
        text: "{game.name}".into(),
        style: TextStyle { font: Some("bold".into()), font_size: 0.08, color: pal.text, ..Default::default() },
        background: Some(pal.background.with_alpha(200)),
        wrap: false,
        max_lines: Some(1),
        scroll: false,
    }
}

fn item_focus_text(pal: &Palette) -> Element {
    Element::Text {
        common: Common {
            name: "item_focus".into(),
            pos: [0.5, 0.5],
            origin: [0.5, 0.5],
            visible_if: Some("{item.focused}".into()),
            ..Default::default()
        },
        text: "▶".into(),
        style: TextStyle { font_size: 0.2, color: pal.accent, align: Align::Center, ..Default::default() },
        background: None,
        wrap: false,
        max_lines: None,
        scroll: false,
    }
}

/// The view's own settings (shown with nothing selected).
fn view_settings(ui: &mut Ui, d: &mut CreatorDlg, pal: &Pal) {
    let Some(doc) = d.doc.as_mut() else { return };
    let palette = doc.file.palette.clone();
    let vk = d.view;
    let Some(view) = doc.file.view(vk).cloned() else { return };
    let mut v = view.clone();
    heading(ui, &format!("The {} screen", super::validate::view_name(vk)), pal);
    row(ui, "Background", pal, |ui| opt_color(ui, &mut v.background, palette.background, &palette, pal));
    let mut vb = v.viewbox.is_some();
    row(ui, "Scaling", pal, |ui| {
        if ui
            .checkbox(&mut vb, "keep proportions on every screen")
            .on_hover_text("Laid out for 1920×1080 and scaled as one piece; other screen shapes get room at the edges")
            .changed()
        {
            v.viewbox = vb.then_some([1920.0, 1080.0]);
        }
    });
    if vk == ViewKind::Games || vk == ViewKind::Collections {
        let mut start_games = doc.file.theme.start == "games";
        if vk == ViewKind::Games
            && doc.file.home.is_none()
            && ui.checkbox(&mut start_games, "Controller mode opens here (All games)").changed()
        {
            doc.change("Start screen", |f| f.theme.start = if start_games { "games".into() } else { String::new() });
        }
    }
    if v != view && widgets::user_acted(ui.ctx()) {
        doc.change("Change the screen", |f| {
            if let Some(x) = super::panel::view_mut(f, vk) {
                *x = v;
            }
        });
    }
}

/// The element as TOML, to change anything else (applied with "Apply").
fn raw_pane(ui: &mut Ui, d: &mut CreatorDlg, shown: &Element, pal: &Pal) {
    ui.add_space(8.0);
    let open = egui::CollapsingHeader::new(RichText::new("Advanced: this element as TOML").size(12.5))
        .id_salt("ls_creator_raw")
        .default_open(d.raw.open)
        .show(ui, |ui| {
            let current = element_toml(shown);
            if d.raw.text.as_ref().is_none_or(|(made, _)| *made != current) && d.raw.error.is_none() {
                d.raw.text = Some((current.clone(), current.clone()));
            }
            let Some((_, text)) = d.raw.text.as_mut() else { return };
            ui.add(
                egui::TextEdit::multiline(text)
                    .font(egui::TextStyle::Monospace)
                    .desired_rows(10)
                    .desired_width(f32::INFINITY),
            );
            let unknown: Vec<String> = shown.common().extra.keys().cloned().collect();
            if !unknown.is_empty() {
                note(ui, &format!("Kept but not used by Lodestar: {}", unknown.join(", ")), pal);
            }
            if let Some(e) = &d.raw.error {
                ui.label(RichText::new(e).size(12.0).color(pal.danger));
            }
            let mut apply = None;
            let mut revert = false;
            ui.horizontal(|ui| {
                if ui.button("Apply").clicked() {
                    apply = Some(text.clone());
                }
                revert = ui.button("Revert").clicked();
            });
            if revert {
                d.raw.text = None;
                d.raw.error = None;
            }
            if let Some(t) = apply {
                match parse_element(&t) {
                    Ok(e) => {
                        d.raw.error = None;
                        let ctx = ui.ctx().clone();
                        if let (Some(doc), Some(p)) = (d.doc.as_mut(), d.sel.clone()) {
                            super::edit_element(doc, &ctx, &d.layer.clone(), d.view, &p, &mut d.banner, move |x| {
                                *x = e
                            });
                        }
                        d.raw.text = None;
                    }
                    Err(e) => d.raw.error = Some(e),
                }
            }
        });
    d.raw.open = open.fully_open();
}

/// One element as TOML (tidy: defaults left out, like the theme file).
pub fn element_toml(e: &Element) -> String {
    let mut f = ThemeFile::default();
    f.games.elements.push(e.clone());
    let text = f.to_theme_toml(None).unwrap_or_default();
    let body = text.split("[[games.elements]]\n").nth(1).unwrap_or("");
    // Nested parts keep their headers, relative to the element.
    body.replace("[[games.elements.", "[[").trim_end().to_string()
}

/// Parse what `element_toml` wrote (or the user typed).
pub fn parse_element(t: &str) -> Result<Element, String> {
    let wrapped = format!(
        "[[games.elements]]\n{}",
        t.replace("\n[[", "\n[[games.elements.").replace("[[games.elements.games.elements]]", "[[games.elements]]")
    );
    let mut f: ThemeFile = ThemeFile::parse(&wrapped).map_err(|e| format!("Not quite right: {e:#}"))?;
    match f.games.elements.len() {
        1 => Ok(f.games.elements.remove(0)),
        _ => Err("Write one element (it starts with type = \"…\").".into()),
    }
}
