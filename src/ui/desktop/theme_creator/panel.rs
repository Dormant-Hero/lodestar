//! The elements panel: the view's elements front to back (drag to reorder, the eye hides one),
//! the parts inside stacks and list items, and the Add menu with every kind of element.

use super::super::Pal;
use super::ops::{self, ElPath, Kind, Layer, Nest};
use super::{AssetFor, CreatorDlg, PickFor, files::Asset, widgets};
use crate::app::AppState;
use crate::theme::*;
use egui::{Color32, RichText, Sense, Ui, vec2};

/// The view of a file, mutable.
pub fn view_mut(f: &mut ThemeFile, k: ViewKind) -> Option<&mut View> {
    match k {
        ViewKind::Collections => Some(&mut f.collections),
        ViewKind::Games => Some(&mut f.games),
        ViewKind::Details => f.details.as_mut(),
        ViewKind::Home => f.home.as_mut(),
    }
}

/// Add a game page or a start page, from Lodestar's own layouts.
pub fn add_view(f: &mut ThemeFile, k: ViewKind) {
    let source = match k {
        ViewKind::Details => crate::theme::builtin::all().into_iter().find_map(|(_, t)| t.details),
        ViewKind::Home => crate::theme::builtin::all().into_iter().find_map(|(_, t)| t.home),
        _ => None,
    };
    let v = source.unwrap_or_default();
    match k {
        ViewKind::Details => f.details = Some(v),
        ViewKind::Home => f.home = Some(v),
        _ => {}
    }
}

/// Set the screen's background: a picture or video file (`path`), else the game's art. Uses
/// the element called "background" (made at the back of the screen when missing). Returns it.
pub fn set_background(f: &mut ThemeFile, k: ViewKind, path: Option<String>, video: bool) -> Option<ElPath> {
    let pal = f.palette.clone();
    let view = view_mut(f, k)?;
    let i = match view
        .elements
        .iter()
        .position(|e| e.common().name == "background" && matches!(e, Element::Image { .. }))
    {
        Some(i) => i,
        None => {
            let min_z = view.elements.iter().map(|e| e.common().z).fold(0.0f32, f32::min);
            let mut e = ops::new_element(Kind::Backdrop, k, &pal, "background".into());
            e.common_mut().z = min_z - 1.0;
            view.elements.insert(0, e);
            0
        }
    };
    if let Element::Image { path: p, fallbacks, video: v, common, .. } = &mut view.elements[i] {
        common.visible = true;
        match (path, video) {
            (Some(file), true) => {
                *v = Some(file);
            }
            (Some(file), false) => {
                *p = file;
                fallbacks.clear();
                common.blur = 0.0;
                common.opacity = 1.0;
            }
            (None, _) => {
                let games = k != ViewKind::Collections;
                *p = if games { "{art.hero}" } else { "{collection.hero}" }.into();
                *fallbacks = vec![if games { "{art.cover}" } else { "{collection.cover}" }.into()];
                *v = None;
            }
        }
    }
    Some(ElPath::top(i))
}

pub fn delete_selected(d: &mut CreatorDlg) {
    let (Some(doc), Some(p)) = (d.doc.as_mut(), d.sel.clone()) else { return };
    let view = d.view;
    if d.layer != Layer::Base {
        // A layer can't take an element away: it hides it there.
        let ctx = egui::Context::default();
        super::edit_element(doc, &ctx, &d.layer.clone(), view, &p, &mut d.banner, |e| e.common_mut().visible = false);
        return;
    }
    doc.change("Delete an element", |f| {
        if let Some(v) = view_mut(f, view) {
            ops::delete(&mut v.elements, &p);
        }
    });
    d.sel = None;
    d.raw.reset();
}

pub fn duplicate_selected(d: &mut CreatorDlg) {
    let (Some(doc), Some(p)) = (d.doc.as_mut(), d.sel.clone()) else { return };
    if d.layer != Layer::Base {
        d.banner = Some(super::Banner {
            text: "Copies are made in the theme itself: switch “Changes go to” to “The whole theme”.".into(),
            error: true,
        });
        return;
    }
    let view = d.view;
    let mut np = None;
    doc.change("Duplicate an element", |f| {
        if let Some(v) = view_mut(f, view) {
            np = ops::duplicate(&mut v.elements, &p);
        }
    });
    if np.is_some() {
        d.sel = np;
        d.raw.reset();
    }
}

/// Add an element of `kind` (in front of everything; lists go where the old list was).
fn add(d: &mut CreatorDlg, kind: Kind) {
    let Some(doc) = d.doc.as_mut() else { return };
    let view = d.view;
    let layer = d.layer.clone();
    let pal = doc.file.palette.clone();
    let mut new_path = None;
    doc.change("Add an element", |f| {
        let Some(v) = view_mut(f, view) else { return };
        let name = ops::unique_name(&v.elements, kind.base_name());
        let mut e = ops::new_element(kind, view, &pal, name.clone());
        let top_z = v.elements.iter().map(|e| e.common().z).fold(0.0f32, f32::max);
        if !matches!(kind, Kind::Backdrop | Kind::Fade | Kind::Panel) {
            e.common_mut().z = e.common().z.max(top_z);
        }
        if layer != Layer::Base {
            // Only shown where the change goes: hidden in the theme, shown in the layer.
            e.common_mut().visible = false;
        }
        v.elements.push(e);
        let i = v.elements.len() - 1;
        new_path = Some(ElPath::top(i));
        if layer != Layer::Base {
            let _ = ops::edit_in_layer(f, &layer, view, &ElPath::top(i), |e| e.common_mut().visible = true);
        }
    });
    if new_path.is_some() {
        d.sel = new_path;
        d.raw.reset();
    }
}

pub fn show(ui: &mut Ui, st: &mut AppState, d: &mut CreatorDlg, pal: &Pal, h: f32) {
    let ctx = ui.ctx().clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Elements").size(14.0).strong().color(pal.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.menu_button(RichText::new("＋ Add").strong(), |ui| {
                ui.set_min_width(200.0);
                for (group, kinds) in Kind::GROUPS {
                    ui.menu_button(group, |ui| {
                        ui.set_min_width(230.0);
                        for k in kinds.iter().copied() {
                            if ui.button(k.label()).on_hover_text(k.hint()).clicked() {
                                let view = d.view;
                                add(d, k);
                                if k == Kind::Picture
                                    && let Some(p) = d.sel.clone()
                                {
                                    d.pick(
                                        &ctx,
                                        PickFor::Asset(
                                            Asset::Picture,
                                            AssetFor::Element(view, p, super::inspector::Slot::Path),
                                        ),
                                        "Choose a picture",
                                    );
                                }
                                ui.close();
                            }
                        }
                    });
                }
                ui.separator();
                let view = d.view;
                ui.menu_button("This screen's background", |ui| {
                    if ui.button("Picture…").on_hover_text("A picture behind everything on this screen").clicked() {
                        d.pick(
                            &ctx,
                            PickFor::Asset(Asset::Picture, AssetFor::Background(view, false)),
                            "Choose a background picture",
                        );
                        ui.close();
                    }
                    if ui
                        .button("Video…")
                        .on_hover_text("A video behind everything (plays when ffmpeg is installed)")
                        .clicked()
                    {
                        d.pick(
                            &ctx,
                            PickFor::Asset(Asset::Video, AssetFor::Background(view, true)),
                            "Choose a background video",
                        );
                        ui.close();
                    }
                    if ui.button("The game's art").clicked() {
                        if let Some(doc) = d.doc.as_mut() {
                            let mut sel = None;
                            doc.change("Set the background", |f| sel = set_background(f, view, None, false));
                            d.sel = sel;
                        }
                        ui.close();
                    }
                });
            });
        });
    });
    let Some(doc) = d.doc.as_ref() else { return };
    let Some(view) = doc.file.view(d.view) else { return };
    // In a layer, show the elements as that layer has them (hidden ones dimmed).
    let shown: Vec<Element> = match d.layer.set(&doc.file) {
        Some(o) => {
            let t = ops::tables(o, d.view);
            view.elements
                .iter()
                .map(|e| {
                    t.get(&e.common().name)
                        .and_then(|p| crate::theme::render::merge_element(e, p).ok())
                        .unwrap_or_else(|| e.clone())
                })
                .collect()
        }
        None => view.elements.clone(),
    };
    let patched: Vec<bool> = match d.layer.set(&doc.file) {
        Some(o) => view.elements.iter().map(|e| ops::tables(o, d.view).contains_key(&e.common().name)).collect(),
        None => vec![false; shown.len()],
    };
    let order: Vec<usize> = ops::draw_order(&shown).into_iter().rev().collect();
    widgets::note(ui, "Front to back · drag to reorder", pal);
    let mut toggle: Option<(ElPath, bool)> = None;
    let mut select: Option<ElPath> = None;
    let mut drop: Option<(usize, usize)> = None;
    let drag_id = egui::Id::unique("ls_creator_el_drag");
    let dragging: Option<usize> = ui.data(|m| m.get_temp(drag_id));
    let mut row_rects: Vec<(usize, egui::Rect)> = vec![];
    egui::ScrollArea::vertical().id_salt("ls_creator_elements").max_height(h - 60.0).auto_shrink([false, false]).show(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            for &i in &order {
                let p = ElPath::top(i);
                let r = element_row(ui, &shown[i], &p, d.sel.as_ref(), patched[i], 0, pal, &mut toggle, &mut select);
                row_rects.push((i, r.rect));
                if r.drag_started() {
                    ui.data_mut(|m| m.insert_temp(drag_id, i));
                }
                nested_rows(ui, &shown[i], &p, d.sel.as_ref(), 1, pal, &mut toggle, &mut select);
            }
            if let Some(from) = dragging {
                let ptr = ui.input(|i| i.pointer.hover_pos());
                if let Some(p) = ptr {
                    // Insertion line above the row under the pointer.
                    let slot = row_rects.iter().position(|(_, r)| p.y < r.center().y).unwrap_or(row_rects.len());
                    let y =
                        row_rects.get(slot).map(|(_, r)| r.top()).or_else(|| row_rects.last().map(|(_, r)| r.bottom()));
                    if let Some(y) = y {
                        let x = ui.min_rect().x_range();
                        ui.painter().hline(x, y, egui::Stroke::new(2.0, pal.accent));
                    }
                    if ui.input(|i| i.pointer.any_released()) {
                        // `slot` counts rows front to back; the drawing order counts back to front.
                        let n = row_rects.len();
                        let cur = row_rects.iter().position(|(j, _)| *j == from).unwrap_or(0);
                        let target_front = if slot > cur { slot - 1 } else { slot };
                        drop = Some((from, n.saturating_sub(1).saturating_sub(target_front)));
                        ui.data_mut(|m| m.remove::<usize>(drag_id));
                    }
                }
                if !ui.input(|i| i.pointer.any_down()) {
                    ui.data_mut(|m| m.remove::<usize>(drag_id));
                }
            }
            if shown.is_empty() {
                widgets::note(ui, "Nothing here yet. Add a background, a list and a title with ＋ Add.", pal);
            }
        },
    );
    if let Some(p) = select {
        d.sel = Some(p);
        d.raw.reset();
    }
    let layer = d.layer.clone();
    let vk = d.view;
    if let Some((p, vis)) = toggle
        && let Some(doc) = d.doc.as_mut()
    {
        super::edit_element(doc, &ctx, &layer, vk, &p, &mut d.banner, |e| e.common_mut().visible = vis);
    }
    if let Some((from, to)) = drop
        && let Some(doc) = d.doc.as_mut()
    {
        if layer != Layer::Base {
            d.banner = Some(super::Banner { text: "The order is the same for every variant, shape and console: switch to “The whole theme” to change it.".into(), error: true });
        } else {
            let mut at = from;
            doc.change("Reorder elements", |f| {
                if let Some(v) = view_mut(f, vk) {
                    at = ops::move_in_draw_order(&mut v.elements, from, to);
                }
            });
            d.sel = Some(ElPath::top(at));
        }
    }
    let _ = st;
}

#[allow(clippy::too_many_arguments)]
fn element_row(
    ui: &mut Ui,
    e: &Element,
    p: &ElPath,
    sel: Option<&ElPath>,
    patched: bool,
    depth: usize,
    pal: &Pal,
    toggle: &mut Option<(ElPath, bool)>,
    select: &mut Option<ElPath>,
) -> egui::Response {
    let c = e.common();
    let selected = sel == Some(p);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 4.0, pal.sel_bg);
    } else if resp.hovered() {
        painter.rect_filled(rect, 4.0, pal.hover);
    }
    let x0 = rect.min.x + 4.0 + depth as f32 * 14.0;
    // Eye.
    let eye = egui::Rect::from_min_size(egui::pos2(x0, rect.min.y + 4.0), vec2(16.0, 16.0));
    let eye_resp = ui.interact(eye, ui.scope_id().with(("eye", p)), Sense::click());
    let eye_col = if c.visible { pal.text } else { pal.dim };
    painter.text(
        eye.center(),
        egui::Align2::CENTER_CENTER,
        if c.visible { "👁" } else { "–" },
        egui::FontId::proportional(12.0),
        eye_col,
    );
    if eye_resp.clicked() {
        *toggle = Some((p.clone(), !c.visible));
    }
    let eye_resp = eye_resp.on_hover_text(if c.visible { "Hide" } else { "Show" });
    let _ = eye_resp;
    let kind = ops::kind_word(e);
    let name = if c.name.trim().is_empty() { ops::summary(e) } else { c.name.clone() };
    let text_col = if c.visible { pal.text } else { pal.dim };
    painter.text(
        egui::pos2(x0 + 22.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        &name,
        egui::FontId::proportional(12.5),
        text_col,
    );
    let right = if patched { format!("● {kind}") } else { kind.to_string() };
    painter.text(
        egui::pos2(rect.max.x - 6.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        right,
        egui::FontId::proportional(10.5),
        if patched { pal.accent } else { pal.dim },
    );
    if resp.clicked() {
        *select = Some(p.clone());
    }
    let tip = if patched { format!("{} — changed here", ops::summary(e)) } else { ops::summary(e) };
    if !tip.is_empty() {
        let _ = resp.clone().on_hover_text(tip);
    }
    let _ = Color32::WHITE;
    resp
}

#[allow(clippy::too_many_arguments)]
fn nested_rows(
    ui: &mut Ui,
    e: &Element,
    p: &ElPath,
    sel: Option<&ElPath>,
    depth: usize,
    pal: &Pal,
    toggle: &mut Option<(ElPath, bool)>,
    select: &mut Option<ElPath>,
) {
    if depth > 4 {
        return;
    }
    if let Element::Stack { children, .. } = e {
        for (j, ch) in children.iter().enumerate() {
            let cp = p.child(Nest::Children, j);
            element_row(ui, ch, &cp, sel, false, depth, pal, toggle, select);
            nested_rows(ui, ch, &cp, sel, depth + 1, pal, toggle, select);
        }
    }
    for (j, ch) in e.common().item_elements.iter().enumerate() {
        let cp = p.child(Nest::Items, j);
        element_row(ui, ch, &cp, sel, false, depth, pal, toggle, select);
    }
}
