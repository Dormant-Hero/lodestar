//! A game row the theme lays out itself: an ItemsControl whose ItemsSource is
//! PART_ListGameItems' own items (`{Binding ElementName=PART_ListGameItems, Path=ItemsSource}`),
//! drawn in a horizontal StackPanel while the real list stays invisible behind it. Playnite
//! keeps the two in step (the mirror's SelectedValue follows the list), so the mirror is what
//! the player sees and moves through: Lodestar's game list goes in its place, shaped by the
//! mirror's own item template rather than by Playnite's tile panel.
//!
//! WPF lays such items out at their template size and draws them through their RenderTransform:
//! a ScaleTransform that rests small (`ScaleX="0.625"`) and grows on selection takes no extra
//! room, so the pitch comes from the layout size and the IsSelected margins, the drawn size from
//! the scale.

use super::super::extract;
use super::bind::parse_binding;
use super::emit::{el_table, set_f, set_pair};
use super::*;

/// Where the strip draws its items and how big, in px on the 1920 × 1080 screen.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Strip {
    /// An unfocused item as drawn: its layout size times its resting scale.
    pub item: [f32; 2],
    /// The focused item's scale relative to an unfocused one.
    pub focus_scale: f32,
    /// Distance between the centres of two unfocused neighbours.
    pub pitch: f32,
    /// How much farther the focused item's neighbours sit from its centre than `pitch`.
    pub focus_room: f32,
    /// The first (focused) item's centre from the strip's left edge.
    pub first: f32,
    /// The items' top below the strip's top (the item template's own margin).
    pub top: f32,
    /// The focused item grows downwards from a shared top line (RenderTransformOrigin y = 0)
    /// instead of around its centre.
    pub top_aligned: bool,
    /// Rounded corners of an unfocused item (a rounded OpacityMask).
    pub radius: f32,
    /// The item template's focus frame, glow and scrims.
    pub look: ItemLook,
}

impl Strip {
    /// Lodestar's carousel drawing the row in `r` (the mirror's box, cut to the screen): the
    /// focused item stays where the row's first item sits, as the theme scrolls its row to
    /// keep the selection in place.
    pub(super) fn carousel(&self, r: Rect) -> Table {
        let r = r.intersect(&Rect::new(0.0, 0.0, W, H));
        let mut t = el_table("carousel", Rect::new(r.x, r.y + self.top, r.w, self.item[1] * self.focus_scale));
        t.insert("orientation".into(), T::String("horizontal".into()));
        set_pair(&mut t, "item_size", [self.item[0] / W, self.item[1] / H]);
        set_f(&mut t, "visible_items", (r.w / self.pitch).max(1.0));
        set_f(&mut t, "selected_position", (self.first / r.w.max(1.0)).clamp(0.0, 1.0));
        set_f(&mut t, "item_align", if self.top_aligned { 0.0 } else { 0.5 });
        set_f(&mut t, "selected_scale", self.focus_scale);
        set_f(&mut t, "unselected_opacity", 1.0);
        let room = self.focus_room / W;
        set_pair(&mut t, "selected_margins", [room, room]);
        t.insert("item_fit".into(), T::String("cover".into()));
        t
    }
}

/// Narrower items aren't game covers (a mark, a blurred copy scaled up), px.
const MIN_ITEM: f32 = 40.0;

/// The ItemsControl shows PART_ListGameItems' items.
pub(super) fn mirrors_game_list(t: &Tree, el: X) -> bool {
    let Some(Value::Binding(raw)) = t.get(el, "ItemsSource") else { return false };
    let b = parse_binding(&raw);
    let path = b.path.trim().trim_start_matches("Path=").trim();
    b.element.as_deref() == Some("PART_ListGameItems") && matches!(path, "ItemsSource" | "Items")
}

impl<'b, 't, 'i> Builder<'b, 't, 'i> {
    /// The mirror's item geometry (None: its items don't flow in one horizontal row, or its
    /// template gives them no size).
    pub(super) fn strip(&self, el: X<'t, 'i>) -> Option<Strip> {
        if !self.items_flow_across(el) {
            return None;
        }
        let style = self.get(el, "ItemContainerStyle")?.node().filter(|s| tag(*s) == "Style")?;
        let tpl = self.t.template(style)?;
        let root = self.t.template_root(tpl)?;
        let rest_scale = self.scale_of(root).unwrap_or(1.0);
        let layout = self.layout_size(style, tpl, root)?;
        let (rest_margin, focus_margin) = self.selection_margins(style, tpl);
        let focus_scale = self.focus_scale(style, tpl, root).map(|s| s / rest_scale).unwrap_or(1.0);
        let rest_slot = layout[0] + rest_margin[0] + rest_margin[2];
        let focus_slot = layout[0] + focus_margin[0] + focus_margin[2];
        let origin_y =
            self.text(root, "RenderTransformOrigin").and_then(|s| tree::parse_sides(&s)).map(|p| p[1]).unwrap_or(0.0);
        Some(Strip {
            item: [layout[0] * rest_scale, layout[1] * rest_scale],
            focus_scale: focus_scale.clamp(1.0, 3.0),
            pitch: rest_slot.max(1.0),
            focus_room: ((focus_slot - rest_slot) / 2.0).max(0.0),
            first: focus_margin[0] + layout[0] / 2.0,
            top: self.sides(root, "Margin")[1] + rest_margin[1],
            top_aligned: origin_y < 0.25,
            radius: self.mask_radius(root).unwrap_or(0.0) * rest_scale,
            look: ItemLook { focus_scale: None, ..extract::item_look_of(self.t, style) },
        })
    }

    /// The control's items panel (in its Template or ItemsPanel) is a horizontal stack or a
    /// horizontal wrap panel.
    fn items_flow_across(&self, el: X<'t, 'i>) -> bool {
        let holders = [self.get(el, "Template"), self.get(el, "ItemsPanel")];
        let host = holders.into_iter().flatten().filter_map(|v| v.node()).find_map(|h| {
            if tag(h) == "ItemsPanelTemplate" {
                content(h).next()
            } else {
                h.descendants()
                    .find(|d| d.is_element() && attr(*d, "IsItemsHost").is_some_and(|v| v.eq_ignore_ascii_case("true")))
            }
        });
        let Some(panel) = host else { return false };
        match tag(panel) {
            "StackPanel" | "VirtualizingStackPanel" => {
                self.text(panel, "Orientation").is_some_and(|o| o.eq_ignore_ascii_case("horizontal"))
            }
            "WrapPanel" => !self.text(panel, "Orientation").is_some_and(|o| o.eq_ignore_ascii_case("vertical")),
            _ => false,
        }
    }

    /// The resting ScaleX of an element's RenderTransform ScaleTransform.
    fn scale_of(&self, el: X<'t, 'i>) -> Option<f32> {
        let st = self.get(el, "RenderTransform")?.node()?;
        let st = if tag(st) == "ScaleTransform" {
            st
        } else {
            st.descendants().find(|d| d.is_element() && tag(*d) == "ScaleTransform")?
        };
        attr(st, "ScaleX")?.trim().parse::<f32>().ok().filter(|s| *s > 0.0 && s.is_finite())
    }

    /// An item's layout size: the template root's Width / Height, else its cover's (the
    /// picture showing the game's cover, by its binding, its name or a trigger setting its
    /// source). A cover sized by width alone is as tall as Playnite's cover aspect says, up to
    /// the MaxHeight the item's style or template gives it. None: neither sets a width.
    fn layout_size(&self, style: X<'t, 'i>, tpl: X<'t, 'i>, root: X<'t, 'i>) -> Option<[f32; 2]> {
        // The widest cover part with a width of its own (a blurred copy drawn small and scaled
        // up behind it doesn't size the item).
        let cover = root
            .descendants()
            .filter(|d| d.is_element() && self.is_cover(*d, tpl))
            .filter_map(|d| Some((d, self.num(d, "Width")?)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(d, _)| d);
        let size = |prop: &str| self.num(root, prop).or_else(|| cover.and_then(|c| self.num(c, prop)));
        let w = size("Width").filter(|w| *w >= MIN_ITEM)?;
        let max_h = self.item_setter_num(style, tpl, "MaxHeight").unwrap_or(f32::INFINITY);
        let h = size("Height").unwrap_or(w * settings::COVER_ASPECT).min(max_h);
        Some([w, h])
    }

    /// The template part that shows the game's cover: an Image (or a Border painted with an
    /// ImageBrush) bound to the cover, named after it, or given it by one of the template's
    /// triggers.
    fn is_cover(&self, el: X<'t, 'i>, tpl: X<'t, 'i>) -> bool {
        let binds_cover = |v: Option<Value>| match v {
            Some(Value::Binding(b)) => b.contains("Cover"),
            Some(Value::Brush(LookBrush::Image { source, .. })) => source.contains("Cover"),
            _ => false,
        };
        let name = name_of(el).unwrap_or("");
        match tag(el) {
            "Image" => {
                name.contains("Cover")
                    || binds_cover(self.get(el, "Source"))
                    || self.t.triggers(tpl).iter().any(|tr| {
                        tr.descendants().any(|set| {
                            set.is_element()
                                && tag(set) == "Setter"
                                && attr(set, "TargetName").is_some_and(|t| t == name)
                                && attr(set, "Value").is_some_and(|v| v.contains("Cover"))
                        })
                    })
            }
            "Border" => binds_cover(self.get(el, "Background")),
            _ => false,
        }
    }

    /// A number the item's style sets on the item itself (a Setter, or a template / style
    /// trigger without a TargetName that always holds at rest).
    fn item_setter_num(&self, style: X<'t, 'i>, tpl: X<'t, 'i>, prop: &str) -> Option<f32> {
        let mut out = self.t.setter(style, prop).and_then(|v| v.num()).map(|v| v as f32);
        for trig in self.t.triggers(style).into_iter().chain(self.t.triggers(tpl)) {
            if !matches!(tag(trig), "DataTrigger" | "MultiDataTrigger")
                || self.trigger_test(style, trig) != Test::Always
            {
                continue;
            }
            for set in tree::setters(trig).filter(|s| attr(*s, "TargetName").is_none()) {
                if attr(set, "Property").map(str::trim) == Some(prop)
                    && let Some(v) = self.t.setter_value(set, prop).and_then(|v| v.num())
                {
                    out = Some(v as f32);
                }
            }
        }
        out
    }

    /// The item's Margin unselected and selected: the style's Margin, replaced by what its
    /// `IsSelected` triggers set.
    fn selection_margins(&self, style: X<'t, 'i>, tpl: X<'t, 'i>) -> ([f32; 4], [f32; 4]) {
        let base = self.t.setter(style, "Margin").and_then(|v| v.sides()).unwrap_or([0.0; 4]);
        let (mut rest, mut focus) = (base, base);
        for trig in self.t.triggers(style).into_iter().chain(self.t.triggers(tpl)) {
            let Some(selected) = selection_trigger(trig) else { continue };
            for set in tree::setters(trig).filter(|s| attr(*s, "TargetName").is_none()) {
                if attr(set, "Property").map(str::trim) == Some("Margin")
                    && let Some(m) = self.t.setter_value(set, "Margin").and_then(|v| v.sides())
                {
                    if selected {
                        focus = m;
                    } else {
                        rest = m;
                    }
                }
            }
        }
        (rest, focus)
    }

    /// The scale the item's selection storyboard grows its ScaleTransform to.
    fn focus_scale(&self, style: X<'t, 'i>, tpl: X<'t, 'i>, root: X<'t, 'i>) -> Option<f32> {
        let transform = self.get(root, "RenderTransform")?.node()?;
        let name = std::iter::once(transform).chain(transform.descendants()).find_map(name_of);
        let mut out = None;
        for trig in self.t.triggers(style).into_iter().chain(self.t.triggers(tpl)) {
            if selection_trigger(trig) != Some(true) {
                continue;
            }
            for sb in self.t.enter_storyboards(trig) {
                for a in sb.descendants().filter(|a| a.is_element() && tag(*a) == "DoubleAnimation") {
                    let prop = attr(a, "Storyboard.TargetProperty").unwrap_or("");
                    let target = attr(a, "Storyboard.TargetName").map(str::trim);
                    if prop.contains("ScaleX")
                        && (target.is_none() || target == name)
                        && let Some(to) = attr(a, "To").and_then(|v| v.trim().parse::<f32>().ok())
                    {
                        out = Some(to);
                    }
                }
            }
        }
        out.filter(|s| *s > 0.0 && s.is_finite())
    }

    /// The corner radius of the element a rounded OpacityMask (a VisualBrush of a Border) shows
    /// the item through.
    fn mask_radius(&self, root: X<'t, 'i>) -> Option<f32> {
        root.descendants().filter(|d| d.is_element()).find_map(|d| {
            let vb = self.get(d, "OpacityMask")?.node().filter(|n| tag(*n) == "VisualBrush")?;
            let Markup::Binding(raw) = tree::markup(attr(vb, "Visual")?) else { return None };
            let mask = self.t.named(root, &parse_binding(&raw).element?)?;
            self.get(mask, "CornerRadius")?.max_side().filter(|r| *r > 0.0)
        })
    }
}

/// `Trigger Property="IsSelected"` → Some(its value).
fn selection_trigger(trig: X) -> Option<bool> {
    if tag(trig) != "Trigger" || attr(trig, "Property").map(str::trim) != Some("IsSelected") {
        return None;
    }
    attr(trig, "Value").map(|v| v.trim().eq_ignore_ascii_case("true"))
}

/// Which part is the game list: PART_ListGameItems when it shows, else the first row of its
/// items the theme lays out itself (that row's geometry is returned), else an add-on's control
/// standing in for it. Every other mirror is a part Lodestar can't draw.
pub(super) fn pick_list(root: &mut LNode) -> Option<Strip> {
    let list_shows = shown_kinds(root, 1.0).any(|(n, _)| matches!(n.data.kind, Kind::List));
    let mut picked = None;
    pick(root, 1.0, list_shows, &mut picked);
    if !list_shows && picked.is_none() {
        add_on_list(root, 1.0);
    }
    picked
}

/// An add-on's control that draws the games where the hidden PART_ListGameItems sits (a
/// ContentControl beside it in the same panel, such as a carousel add-on): Lodestar's game list
/// takes that control's box. True once one is found.
fn add_on_list(n: &mut LNode, opacity: f32) -> bool {
    let o = opacity * n.data.opacity;
    if n.vis != Vis::Visible || o <= 0.01 {
        return false;
    }
    let holds_hidden_list = n.children.iter().any(|c| matches!(c.data.kind, Kind::List));
    if holds_hidden_list
        && let Some(c) = n.children.iter_mut().find(|c| {
            c.data.tag == "ContentControl"
                && matches!(c.data.kind, Kind::Drop("add-on panel"))
                && shows(c, o * c.data.opacity)
        })
    {
        c.data.kind = Kind::List;
        return true;
    }
    n.children.iter_mut().any(|c| add_on_list(c, o))
}

/// Shows on the screen: visible, not faded out, more than a sliver of the screen.
fn shows(n: &LNode, opacity: f32) -> bool {
    n.vis == Vis::Visible && opacity > 0.01 && n.rect.intersect(&Rect::new(0.0, 0.0, W, H)).area() > W * H * 0.01
}

/// The nodes under `n` (itself included) that show, with their opacity.
fn shown_kinds(n: &LNode, opacity: f32) -> Box<dyn Iterator<Item = (&LNode, f32)> + '_> {
    let o = opacity * n.data.opacity;
    if n.vis != Vis::Visible || o <= 0.01 {
        return Box::new(std::iter::empty());
    }
    let own = shows(n, o).then_some((n, o));
    Box::new(own.into_iter().chain(n.children.iter().flat_map(move |c| shown_kinds(c, o))))
}

fn pick(n: &mut LNode, opacity: f32, list_shows: bool, picked: &mut Option<Strip>) {
    let o = opacity * n.data.opacity;
    if let Kind::Mirror(s) = &n.data.kind {
        if !list_shows && picked.is_none() && shows(n, o) {
            *picked = Some((**s).clone());
            n.data.kind = Kind::List;
        } else {
            n.data.kind = Kind::Drop("list driven by the theme's helpers");
        }
    }
    for c in &mut n.children {
        pick(c, o, list_shows, picked);
    }
}
