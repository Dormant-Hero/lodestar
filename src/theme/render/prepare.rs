//! Preparing a view for drawing: focus information, and merging a theme variant's / the
//! user's override tables into the view's elements.

use super::*;

impl Prepared {
    pub fn from_view(view: View, display_name: Option<String>) -> Prepared {
        let mut order: Vec<usize> = (0..view.elements.len()).collect();
        order.sort_by(|&a, &b| {
            view.elements[a].common().z.partial_cmp(&view.elements[b].common().z).unwrap_or(std::cmp::Ordering::Equal)
        });
        let focus = view.elements.iter().position(|e| e.is_focusable() && e.common().visible);
        Prepared { view, order, focus, display_name }
    }

    pub fn focus_info(&self, screen: Rect) -> FocusInfo {
        let screen = view_frame(&self.view, screen);
        let Some(el) = self.focus.and_then(|i| self.view.elements.get(i)) else {
            return FocusInfo { kind: FocusKind::None, page: 10 };
        };
        let c = el.common();
        match el {
            Element::Carousel { orientation, visible_items, .. } => {
                let vertical = match orientation {
                    Orientation::Vertical | Orientation::VerticalWheel => true,
                    Orientation::Horizontal | Orientation::HorizontalWheel => false,
                    Orientation::Wheel => c.size[1] * screen.height() > c.size[0] * screen.width(),
                };
                FocusInfo { kind: FocusKind::Carousel { vertical }, page: (visible_items.floor() as usize).max(1) }
            }
            Element::Grid { columns, item_aspect, spacing, show_labels, item_text_style, .. } => {
                let cols = (*columns).max(1) as usize;
                let (sw, sh) = (screen.width(), screen.height());
                let g = spacing * sh;
                let iw = ((c.size[0] * sw - (cols as f32 - 1.0) * g) / cols as f32).max(1.0);
                let label = if *show_labels { item_text_style.font_size * sh * 1.7 } else { 0.0 };
                let row_h = iw * item_aspect.max(0.05) + label + g;
                let rows = ((c.size[1] * sh + g) / row_h).floor().max(1.0) as usize;
                FocusInfo { kind: FocusKind::Grid { columns: cols }, page: cols * rows }
            }
            Element::TextList { style, row_height, .. } => {
                let rh = (style.font_size * row_height).max(0.001);
                FocusInfo { kind: FocusKind::List, page: ((c.size[1] / rh).floor() as usize).max(1) }
            }
            _ => FocusInfo { kind: FocusKind::None, page: 10 },
        }
    }
}

/// Merge `patch` over `base` recursively (tables merge, everything else replaces).
pub fn merge_table(base: &mut toml::Table, patch: &toml::Table) {
    for (k, v) in patch {
        match (base.get_mut(k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(p)) => merge_table(b, p),
            _ => {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Apply a partial table to an element (serialize → merge → deserialize).
pub fn merge_element(el: &Element, patch: &toml::Table) -> anyhow::Result<Element> {
    let toml::Value::Table(mut base) = toml::Value::try_from(el)? else {
        anyhow::bail!("element did not serialise to a table");
    };
    merge_table(&mut base, patch);
    Ok(toml::Value::Table(base).try_into()?)
}

pub(super) fn apply_tables(view: &mut View, tables: &std::collections::BTreeMap<String, toml::Table>) {
    if tables.is_empty() {
        return;
    }
    for el in view.elements.iter_mut() {
        let name = &el.common().name;
        if name.is_empty() {
            continue;
        }
        if let Some(t) = tables.get(name) {
            match merge_element(el, t) {
                Ok(e) => *el = e,
                Err(err) => log::warn!("theme override for element {name:?} ignored: {err:#}"),
            }
        }
    }
}

pub(super) fn override_tables(
    o: &CollectionOverride,
    kind: ViewKind,
) -> &std::collections::BTreeMap<String, toml::Table> {
    match kind {
        ViewKind::Collections => &o.collections,
        ViewKind::Games => &o.games,
        ViewKind::Details => &o.details,
        ViewKind::Home => &o.home,
    }
}

/// Build the view for (kind, collection): base → variant → first matching collection override.
#[cfg(test)]
pub fn prepare_view(
    file: &ThemeFile,
    kind: ViewKind,
    coll: Option<&Collection>,
    variant: Option<&str>,
) -> Option<Prepared> {
    prepare_view_for(file, kind, coll, variant, None)
}

/// `prepare_view` with the layout for a screen shape (`ThemeFile::aspects` key) applied after
/// the variant.
pub fn prepare_view_for(
    file: &ThemeFile,
    kind: ViewKind,
    coll: Option<&Collection>,
    variant: Option<&str>,
    aspect: Option<&str>,
) -> Option<Prepared> {
    let mut view = file.view(kind)?.clone();
    if let Some(v) = variant.and_then(|v| file.variants.get(v)) {
        apply_tables(&mut view, override_tables(v, kind));
    }
    if let Some(a) = aspect.and_then(|a| file.aspects.get(a)) {
        apply_tables(&mut view, override_tables(a, kind));
    }
    let mut display_name = None;
    if let Some(c) = coll {
        let key = c.key();
        if let Some(o) = file.overrides.get(&key) {
            apply_tables(&mut view, override_tables(o, kind));
            display_name.clone_from(&o.display_name);
        }
    }
    Some(Prepared::from_view(view, display_name))
}
