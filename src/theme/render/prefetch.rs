//! Pictures of the items next to the selection, loaded ahead: holding a direction through
//! the system carousel or a game list shows each item's background, logo and artwork as soon
//! as it is selected instead of a few frames later.
//!
//! The items' own pictures (carousel / grid cells) are prefetched where they are drawn
//! (lists.rs). This adds the view's pictures bound to the selection (`{collection.hero}`,
//! `{art.hero}` …) for the neighbours: ±1 and ±2 in carousels and lists, ±1 and
//! ± a row in grids, and ± a page (PageDown / PageUp; on a details page the next game). They are resolved
//! once per selection (and twice a second, for artwork that arrives meanwhile), not every
//! frame, and queued behind what is on screen (`ImageCache::prefetch`).
//!
//! `prefetch_view` does the same for the screen A opens next (the selected collection's game
//! list, the selected game's page), so entering it shows its pictures right away.

use super::*;

/// What the neighbour list was resolved for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct NeighbourKey {
    kind: ViewKind,
    list_id: u64,
    selected: usize,
    n: usize,
    view: usize,
    tick: u64,
}

/// Neighbour offsets for a view's focus element: the next two each way (a grid: the next
/// one and the next row), then a page each way (PageDown / PageUp).
pub(super) fn neighbour_offsets(info: FocusInfo) -> Vec<i64> {
    let mut v = match info.kind {
        FocusKind::Grid { columns } if columns > 1 => vec![1, columns as i64, -1, -(columns as i64)],
        _ => vec![1, 2, -1, -2],
    };
    let page = info.page as i64;
    if page > 2 && !v.contains(&page) {
        v.extend([page, -page]);
    }
    v
}

/// Index `off` items away from `sel` (wrapping around when `wrap`), None past the ends or on
/// the selection itself.
pub(super) fn neighbour(sel: usize, off: i64, n: usize, wrap: bool) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let i = sel as i64 + off;
    let i = if wrap { i.rem_euclid(n as i64) } else { i };
    (0..n as i64).contains(&i).then_some(i as usize).filter(|&i| i != sel)
}

impl Renderer {
    pub(super) fn prefetch_neighbours(&mut self, f: &mut Frame, prepared: &Prepared) {
        let n = f.scene.items.len();
        if n < 2 {
            return;
        }
        let key = NeighbourKey {
            kind: f.scene.kind,
            list_id: f.scene.list_id,
            selected: f.scene.selected,
            n,
            view: prepared as *const Prepared as usize,
            tick: (f.now * 2.0).max(0.0) as u64,
        };
        if self.neighbours.as_ref().is_none_or(|(k, _)| *k != key) {
            let list = self.resolve_neighbours(f, prepared);
            self.neighbours = Some((key, list));
        }
        if let Some((_, list)) = &self.neighbours {
            for (p, size) in list {
                f.images.prefetch(p, *size);
            }
        }
    }

    fn resolve_neighbours(&mut self, f: &mut Frame, prepared: &Prepared) -> Vec<(PathBuf, ImgSize)> {
        let n = f.scene.items.len();
        let sel = f.scene.selected.min(n - 1);
        let wrap = matches!(f.scene.items, Items::Collections(_));
        let variant = self.sig.as_ref().and_then(|s| s.2.clone());
        let mut out: Vec<(PathBuf, ImgSize)> = vec![];
        for off in neighbour_offsets(prepared.focus_info(f.screen)) {
            let Some(i) = neighbour(sel, off, n, wrap) else { continue };
            let b = f.scene.items.binder(i, f.base);
            // A collection's own view (its theme overrides), as it will be drawn.
            let own = match f.scene.items {
                Items::Collections(c) => {
                    c.get(i).and_then(|c| self.prepare(f.theme, f.scene.kind, Some(&c.collection), variant.as_deref()))
                }
                Items::Games { .. } => None,
            };
            let view = own.as_deref().unwrap_or(prepared);
            for &ei in &view.order {
                let Element::Image { common, path, fallbacks, .. } = &view.view.elements[ei] else {
                    continue;
                };
                if !(path.contains('{') || fallbacks.iter().any(|p| p.contains('{'))) || !b.visible(common) {
                    continue;
                }
                let size = image_size(common, self.opts.blur_background, f.sw(), f.sh(), f.ppp);
                if let Some(p) = self.pick_image(f.theme, f.images, &b, path, fallbacks, size)
                    && !out.iter().any(|(q, s)| *q == p && *s == size)
                {
                    out.push((p, size));
                }
            }
        }
        out
    }
}

impl Renderer {
    /// Load ahead the pictures `kind` will show for `coll` / `game` (the screen A opens: a
    /// collection's game list, a game's page). Only the pictures bound to them (a game-bound
    /// picture needs `game`), plus the item pictures of `items` (the first games the list
    /// shows) once the list was drawn before (its item size is known); resolved once per
    /// target and twice a second.
    #[allow(clippy::too_many_arguments)]
    pub fn prefetch_view(
        &mut self,
        theme: &Theme,
        kind: ViewKind,
        coll: Option<&CollectionItem>,
        game: Option<&Game>,
        items: &[&Game],
        variant: Option<&str>,
        images: &mut ImageCache,
        screen: Rect,
        ppp: f32,
        now: f64,
    ) {
        let key = NeighbourKey {
            kind,
            list_id: game.map_or(u64::MAX, |g| g.id as u64),
            selected: coll.map_or(0, |c| c as *const CollectionItem as usize),
            n: 0,
            view: theme as *const Theme as usize,
            tick: (now * 2.0).max(0.0) as u64,
        };
        if self.ahead.as_ref().is_none_or(|(k, _)| *k != key) {
            let mut out: Vec<(PathBuf, ImgSize)> = vec![];
            if let Some(view) = self.prepare(theme, kind, coll.map(|c| &c.collection), variant) {
                let b = Binder {
                    game,
                    collection: coll,
                    theme_dir: theme.dir.as_deref(),
                    list: None,
                    section: None,
                    focused: false,
                    page: kind == ViewKind::Details,
                    collection_art: (!theme.file.collection_art.is_empty()).then_some(&theme.file.collection_art),
                };
                for &ei in &view.order {
                    let Element::Image { common, path, fallbacks, .. } = &view.view.elements[ei] else {
                        continue;
                    };
                    if !(path.contains('{') || fallbacks.iter().any(|p| p.contains('{'))) || !b.visible(common) {
                        continue;
                    }
                    let size = image_size(common, self.opts.blur_background, screen.width(), screen.height(), ppp);
                    if let Some(p) = self.pick_image(theme, images, &b, path, fallbacks, size)
                        && !out.iter().any(|(q, s)| *q == p && *s == size)
                    {
                        out.push((p, size));
                    }
                }
                // The list's own items (covers, logos) at the size it drew them last time.
                let focus = view.focus.and_then(|fi| Some((fi, self.item_sizes.get(&(kind, fi)).copied()?)));
                if let Some((fi, size)) = focus
                    && let Element::Carousel { item_image, item_fallbacks, .. }
                    | Element::Grid { item_image, item_fallbacks, .. } = &view.view.elements[fi]
                {
                    for g in items {
                        let b = b.with_game(Some(g));
                        if let Some(p) = self.pick_image(theme, images, &b, item_image, item_fallbacks, size) {
                            out.push((p, size));
                        }
                    }
                }
            }
            self.ahead = Some((key, out));
        }
        if let Some((_, list)) = &self.ahead {
            for (p, size) in list {
                images.prefetch(p, *size);
            }
        }
    }
}

/// The cache size an image element is loaded at (`draw_image` and the prefetch agree):
/// blurred pictures small, auto-sized ones medium, else by the drawn size in pixels.
pub(super) fn image_size(common: &Common, blur_on: bool, sw: f32, sh: f32, ppp: f32) -> ImgSize {
    let longest = (common.size[0] * sw).max(common.size[1] * sh).max(1.0) * ppp;
    if blur_on && common.blur > 0.0 {
        ImgSize::Thumb
    } else if common.size == [0.0, 0.0] {
        ImgSize::Medium
    } else {
        size_class(longest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbours_wrap_in_carousels_and_stop_at_list_ends() {
        assert_eq!(neighbour(0, -1, 5, true), Some(4));
        assert_eq!(neighbour(0, -1, 5, false), None);
        assert_eq!(neighbour(4, 2, 5, false), None);
        assert_eq!(neighbour(3, 1, 5, false), Some(4));
        assert_eq!(neighbour(0, 2, 2, true), None, "wrapped onto the selection");
        let info = |kind, page| FocusInfo { kind, page };
        assert_eq!(neighbour_offsets(info(FocusKind::Grid { columns: 4 }, 12)), [1, 4, -1, -4, 12, -12]);
        assert_eq!(neighbour_offsets(info(FocusKind::Carousel { vertical: false }, 2)), [1, 2, -1, -2]);
        assert_eq!(neighbour_offsets(info(FocusKind::List, 8)), [1, 2, -1, -2, 8, -8]);
    }
}
