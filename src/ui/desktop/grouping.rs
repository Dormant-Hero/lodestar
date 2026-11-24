//! Group by …: which groups a game falls in, the flat list of shown games with
//! their group headers (collapsed groups show only the header), the vertical layout every view
//! shares (Grid, List, Details list) and grid keyboard moves across groups.
//!
//! The grouped list is recomputed only when the view, the grouping or the collapsed groups
//! change (`GroupState::key`); drawing only looks things up.

use super::*;
use crate::config::GroupBy;
use crate::library::Library;
use std::collections::HashMap;

/// One group of the view, before collapsing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Group {
    /// Stable key within its grouping ("steam", "rpg", "2024", "never" …).
    pub key: String,
    pub label: String,
    /// Its games in view order (indices into `Library::games`).
    pub games: Vec<usize>,
}

/// What grouping needs besides the games: the time (Last played buckets) and the mounted
/// drives (Install drive).
#[derive(Clone, Debug, Default)]
pub(crate) struct GroupCtx {
    /// Local midnight today, Monday of this week and the 1st of this month (unix seconds).
    pub today: i64,
    pub week: i64,
    pub month: i64,
    /// Mount points, longest first.
    pub mounts: Vec<String>,
}

impl GroupCtx {
    pub fn now(by: GroupBy) -> GroupCtx {
        let now = now_unix();
        let (_, _, day, h, m, wday) = crate::theme::render::bind::local_time(now);
        let today = now - (h as i64 * 3600 + m as i64 * 60 + now.rem_euclid(60));
        GroupCtx {
            today,
            week: today - ((wday as i64 + 6) % 7) * 86_400,
            month: today - (day as i64 - 1) * 86_400,
            mounts: if by == GroupBy::InstallDrive { read_mounts() } else { vec![] },
        }
    }

    /// Day stamp: Last played groups are recomputed when the day changes.
    pub fn day(&self) -> i64 {
        self.today
    }
}

/// Mount points from /proc/self/mountinfo, longest first ("/" always there).
fn read_mounts() -> Vec<String> {
    let text = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
    parse_mounts(&text)
}

pub(crate) fn parse_mounts(mountinfo: &str) -> Vec<String> {
    let mut v: Vec<String> = mountinfo
        .lines()
        .filter_map(|l| l.split(' ').nth(4))
        .map(|m| m.replace("\\040", " ").replace("\\011", "\t").replace("\\134", "\\"))
        .collect();
    v.push("/".into());
    v.sort_by(|a, b| b.len().cmp(&a.len()).then(a.cmp(b)));
    v.dedup();
    v
}

/// The mount point holding `path` (longest matching prefix).
pub(crate) fn drive_of<'a>(mounts: &'a [String], path: &str) -> &'a str {
    mounts
        .iter()
        .find(|m| {
            m.as_str() == "/" || path == m.as_str() || path.strip_prefix(m.as_str()).is_some_and(|r| r.starts_with('/'))
        })
        .map_or("/", |m| m.as_str())
}

/// Play time buckets (most first) and the never-played one.
const PLAYTIME: [(u64, &str, &str); 5] = [
    (100 * 3600, "100h", "Over 100 hours"),
    (20 * 3600, "20h", "20 to 100 hours"),
    (5 * 3600, "5h", "5 to 20 hours"),
    (3600, "1h", "1 to 5 hours"),
    (1, "0h", "Under 1 hour"),
];

/// Group order: (no value: last, rank, lowercase label).
type Order = (bool, i64, String);

/// The groups of `view` (in view order inside each group) for `by`; empty for `GroupBy::None`.
/// A game with several values (genres, tags, collections, companies) is in each of them.
pub(crate) fn group_games(lib: &Library, view: &[usize], by: GroupBy, cx: &GroupCtx) -> Vec<Group> {
    if by == GroupBy::None {
        return vec![];
    }
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut groups: Vec<(Order, Group)> = vec![];
    // Tag spelling shown: the library's (most used) one.
    let tag_names: HashMap<String, String> = if by == GroupBy::Tag {
        lib.all_tags().into_iter().map(|t| (t.to_lowercase(), t)).collect()
    } else {
        HashMap::new()
    };
    for &i in view {
        let Some(g) = lib.games.get(i) else { continue };
        let mut add = |key: &str, label: &str, missing: bool, rank: i64| {
            let gi = match index.get(key) {
                Some(&gi) => gi,
                None => {
                    index.insert(key.to_string(), groups.len());
                    groups.push((
                        (missing, rank, label.to_lowercase()),
                        Group { key: key.to_string(), label: label.to_string(), games: vec![] },
                    ));
                    groups.len() - 1
                }
            };
            let games = &mut groups[gi].1.games;
            if games.last() != Some(&i) {
                games.push(i);
            }
        };
        match by {
            GroupBy::None => {}
            GroupBy::Source => add(g.source_group_key(), g.source_name(), false, 0),
            GroupBy::Platform | GroupBy::Genre | GroupBy::Developer | GroupBy::Publisher => {
                let (facet, none) = match by {
                    GroupBy::Platform => (Facet::Platform, "Unknown platform"),
                    GroupBy::Genre => (Facet::Genre, "No genre"),
                    GroupBy::Developer => (Facet::Developer, "Unknown developer"),
                    _ => (Facet::Publisher, "Unknown publisher"),
                };
                let mut any = false;
                facet.for_each(g, |v| {
                    if !v.is_empty() {
                        any = true;
                        add(&v.to_lowercase(), v, false, 0);
                    }
                });
                if !any {
                    add("", none, true, 0);
                }
            }
            // The year of the release date when known (like Sort → Release date), else the year.
            GroupBy::ReleaseYear => match crate::library::release_key(g).map(|k| k.0) {
                Some(y) => add(&y.to_string(), &y.to_string(), false, -(y as i64)),
                None => add("", "Unknown year", true, 0),
            },
            GroupBy::Installed => {
                if g.installed {
                    add("installed", "Installed", false, 0)
                } else {
                    add("not-installed", "Not installed", false, 1)
                }
            }
            GroupBy::Completion => match g.completion() {
                Some(c) => add(
                    &format!("{c:?}").to_lowercase(),
                    c.label(),
                    false,
                    crate::library::completion_rank(Some(c)) as i64,
                ),
                None => add("", "Not set", true, 0),
            },
            GroupBy::LastPlayed => {
                let (key, label, rank) = match g.last_played {
                    None => ("never", "Never", 4),
                    Some(t) if t >= cx.today => ("today", "Today", 0),
                    Some(t) if t >= cx.week => ("week", "This week", 1),
                    Some(t) if t >= cx.month => ("month", "This month", 2),
                    Some(_) => ("earlier", "Earlier", 3),
                };
                add(key, label, false, rank)
            }
            GroupBy::Playtime => match PLAYTIME.iter().position(|b| g.playtime_secs >= b.0) {
                Some(r) => add(PLAYTIME[r].1, PLAYTIME[r].2, false, r as i64),
                None => add("never", "Not played", true, 0),
            },
            GroupBy::InstallDrive => {
                let path = g
                    .install_dir
                    .as_deref()
                    .or_else(|| g.emu.as_ref().map(|e| e.rom.as_path()))
                    .filter(|p| p.is_absolute());
                match path {
                    Some(p) => {
                        let d = drive_of(&cx.mounts, &p.to_string_lossy()).to_string();
                        let label = if d == "/" { "System drive (/)".to_string() } else { d.clone() };
                        add(&d, &label, false, 0)
                    }
                    None => add("", "No install folder", true, 0),
                }
            }
            GroupBy::Tag => {
                let mut any = false;
                for t in &g.tags {
                    let t = t.trim();
                    if t.is_empty() {
                        continue;
                    }
                    any = true;
                    let key = t.to_lowercase();
                    let label = tag_names.get(&key).map_or(t, String::as_str);
                    add(&key, label, false, 0);
                }
                if !any {
                    add("", "No tags", true, 0);
                }
            }
            GroupBy::Collection => {
                let mut any = false;
                for (n, c) in lib.user.iter().enumerate() {
                    if c.contains(g) {
                        any = true;
                        add(&c.id.to_string(), &c.name, false, n as i64);
                    }
                }
                if !any {
                    add("", "Not in a collection", true, 0);
                }
            }
        }
    }
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    groups.into_iter().map(|(_, g)| g).collect()
}

/// A group as shown: its header and the run of `items` it covers (none while collapsed).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Span {
    /// "<group by key>:<group key>" (remembered collapsed state).
    pub key: String,
    pub label: String,
    /// Games in the group (also while collapsed).
    pub count: usize,
    /// First item of the group in the shown list (where it would be while collapsed).
    pub start: usize,
    /// Items shown (0 while collapsed).
    pub len: usize,
    pub collapsed: bool,
}

/// The shown list (a game can be in it more than once) and its spans.
pub(crate) fn flatten(groups: Vec<Group>, by: GroupBy, collapsed: &[String]) -> (Vec<usize>, Vec<Span>) {
    let mut items = vec![];
    let mut spans = Vec::with_capacity(groups.len());
    for g in groups {
        let key = format!("{}:{}", by.key(), g.key);
        let is_collapsed = collapsed.contains(&key);
        let start = items.len();
        if !is_collapsed {
            items.extend_from_slice(&g.games);
        }
        spans.push(Span {
            key,
            label: g.label,
            count: g.games.len(),
            start,
            len: items.len() - start,
            collapsed: is_collapsed,
        });
    }
    (items, spans)
}

/// The run of items a position is in (the whole list without groups): (start, len).
pub(crate) fn run_of(spans: &[Span], n: usize, pos: usize) -> (usize, usize) {
    if spans.is_empty() {
        return (0, n);
    }
    // The span holding `pos` is the last non-empty one starting at or before it.
    let i = spans.partition_point(|s| s.start <= pos);
    spans[..i].iter().rev().find(|s| s.len > 0 && pos < s.start + s.len).map_or((0, n), |s| (s.start, s.len))
}

/// Grid keyboard move across groups: Up / Down keep the column (clamped to a short row),
/// moving into the previous / next group's last / first row. Left / Right are linear.
pub(crate) fn grid_move(spans: &[Span], n: usize, cols: usize, cur: usize, down: bool) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let cols = cols.max(1);
    let cur = cur.min(n - 1);
    let (start, len) = run_of(spans, n, cur);
    let local = cur - start;
    let (row, col) = (local / cols, local % cols);
    let last_row = (len - 1) / cols;
    let runs: Vec<(usize, usize)> = if spans.is_empty() {
        vec![(0, n)]
    } else {
        spans.iter().filter(|s| s.len > 0).map(|s| (s.start, s.len)).collect()
    };
    let at = runs.iter().position(|r| r.0 == start).unwrap_or(0);
    if down {
        if local + cols < len {
            return Some(cur + cols);
        }
        if row < last_row {
            // Moving down from the last full row lands on the last item (like Playnite).
            return Some(start + len - 1);
        }
        let &(s, l) = runs.get(at + 1)?;
        Some(s + col.min(l - 1))
    } else {
        if row > 0 {
            return Some(cur - cols);
        }
        let &(s, l) = runs.get(at.checked_sub(1)?)?;
        let last = (l - 1) / cols;
        Some(s + (last * cols + col).min(l - 1))
    }
}

/// Vertical layout of a grouped view: per group a header (`header_h`, none without groups)
/// and its rows of `cols` items, `row_h` each; `pad` above the first group.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Blocks {
    pub cols: usize,
    pub row_h: f32,
    pub header_h: f32,
    pub pad: f32,
    /// (span index or None for an ungrouped list, first item, items, top y of the header).
    pub blocks: Vec<(Option<usize>, usize, usize, f32)>,
    pub total_h: f32,
}

impl Blocks {
    pub fn new(spans: &[Span], n: usize, cols: usize, row_h: f32, header_h: f32, pad: (f32, f32)) -> Blocks {
        let cols = cols.max(1);
        let mut blocks = vec![];
        let mut y = pad.0;
        let header_h = if spans.is_empty() { 0.0 } else { header_h };
        if spans.is_empty() {
            blocks.push((None, 0, n, y));
            y += n.div_ceil(cols) as f32 * row_h;
        } else {
            for (i, s) in spans.iter().enumerate() {
                blocks.push((Some(i), s.start, s.len, y));
                y += header_h + s.len.div_ceil(cols) as f32 * row_h;
            }
        }
        Blocks { cols, row_h, header_h, pad: pad.0, blocks, total_h: y + pad.1 }
    }

    /// Top-left of item `pos` relative to the content origin: (column, y of its row).
    pub fn item_pos(&self, pos: usize) -> (usize, f32) {
        let b = self.blocks.partition_point(|b| b.1 <= pos).saturating_sub(1);
        // Collapsed groups (no items) share their start with the next group: take the last
        // block starting at or before `pos` that holds it.
        let b = (0..=b).rev().find(|&i| pos < self.blocks[i].1 + self.blocks[i].2).unwrap_or(b);
        let (_, start, _, top) = self.blocks.get(b).copied().unwrap_or((None, 0, 0, self.pad));
        let local = pos.saturating_sub(start);
        (local % self.cols, top + self.header_h + (local / self.cols) as f32 * self.row_h)
    }

    /// Blocks intersecting `y0..y1` with their visible row ranges: (block index, rows).
    pub fn visible(&self, y0: f32, y1: f32) -> Vec<(usize, std::ops::Range<usize>)> {
        let mut out = vec![];
        let first = self.blocks.partition_point(|b| b.3 <= y0).saturating_sub(1);
        for (bi, &(_, _, len, top)) in self.blocks.iter().enumerate().skip(first) {
            if top > y1 {
                break;
            }
            let rows = len.div_ceil(self.cols);
            let body = top + self.header_h;
            let r0 = (((y0 - body) / self.row_h).floor().max(0.0) as usize).min(rows);
            let r1 = ((((y1 - body) / self.row_h).ceil().max(0.0) as usize) + 1).min(rows);
            out.push((bi, r0..r1.max(r0)));
        }
        out
    }
}

/// What the views draw: the view (each game once, for multi-selection order and drag and
/// drop), the shown list (grouped; positions are `sel_pos`) and its group headers.
#[derive(Clone, Copy)]
pub(crate) struct Shown<'a> {
    pub view: &'a [usize],
    pub items: &'a [usize],
    pub spans: &'a [Span],
}

/// The grouping cache of the desktop UI.
#[derive(Default)]
pub(crate) struct GroupState {
    /// Shown games (a game can be there more than once); positions are `sel_pos`.
    pub items: Vec<usize>,
    pub spans: Vec<Span>,
    /// (view stamp, grouping, collapsed groups, day) the lists were made for.
    key: Option<(u64, GroupBy, Vec<String>, i64)>,
    /// Bumped whenever the view is recomputed.
    pub stamp: u64,
    /// Grouping and collapsed groups (copied from the config each frame).
    pub by: GroupBy,
    pub collapsed: Vec<String>,
    /// The position clicked this frame (a game shown twice is selected where it was clicked).
    pub clicked_pos: Option<usize>,
    day_checked: Option<Instant>,
    day: i64,
    /// The user clicked or scrolled the games (no more scrolling to the selection by itself
    /// while the library settles).
    pub touched: bool,
}

impl DesktopUi {
    /// Recompute the shown list (and place the selection) next frame.
    pub(crate) fn regroup_soon(&mut self) {
        self.groups.key = None;
    }

    /// Recompute the shown list when the view, the grouping or the collapsed groups changed,
    /// and place the selection in it.
    pub(crate) fn regroup(&mut self, lib: &Library) {
        let gs = &mut self.groups;
        if gs.by == GroupBy::LastPlayed && gs.day_checked.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
            gs.day_checked = Some(Instant::now());
            gs.day = GroupCtx::now(GroupBy::None).day();
        }
        let day = if gs.by == GroupBy::LastPlayed { gs.day } else { 0 };
        let fresh =
            matches!(&gs.key, Some((s, b, c, d)) if *s == gs.stamp && *b == gs.by && *c == gs.collapsed && *d == day);
        if fresh {
            return;
        }
        // While the library settles after startup (scan results, metadata) the selected game
        // can move to another group: keep it in view. Later updates leave the scroll alone.
        if gs.key.is_some() && gs.by != GroupBy::None && self.frame < 240 && !gs.touched {
            self.scroll_to_sel = true;
        }
        let gs = &mut self.groups;
        gs.key = Some((gs.stamp, gs.by, gs.collapsed.clone(), day));
        if gs.by == GroupBy::None {
            gs.items = self.view.clone();
            gs.spans.clear();
        } else {
            let groups = group_games(lib, &self.view, gs.by, &GroupCtx::now(gs.by));
            (gs.items, gs.spans) = flatten(groups, gs.by, &gs.collapsed);
        }
        self.place_selection(lib);
    }

    /// `sel_pos` for the selected game in the shown list. A selected game inside a collapsed
    /// group moves to the next shown game; a selection outside the view to the first one.
    fn place_selection(&mut self, lib: &Library) {
        let items = &self.groups.items;
        let id_at = |p: usize| items.get(p).and_then(|&i| lib.games.get(i)).map(|g| g.id);
        let keep = self.sel_pos.filter(|&p| self.selected.is_some() && id_at(p) == self.selected);
        self.sel_pos = keep.or_else(|| {
            let id = self.selected?;
            items.iter().position(|&i| lib.games.get(i).is_some_and(|g| g.id == id))
        });
        if self.sel_pos.is_some() || items.is_empty() && self.view.is_empty() {
            return;
        }
        let in_view =
            self.selected.and_then(|id| self.view.iter().position(|&i| lib.games.get(i).is_some_and(|g| g.id == id)));
        let pos = match in_view {
            // In a collapsed group: the first game shown after it (else the last one).
            Some(_) => {
                let id = self.selected;
                // Once (Install drive reads the mount table).
                let cx = GroupCtx::now(self.groups.by);
                let at = self
                    .groups
                    .spans
                    .iter()
                    .find(|s| s.collapsed && self.group_has(lib, s, id, &cx))
                    .map_or(0, |s| s.start);
                at.min(items.len().saturating_sub(1))
            }
            None => 0,
        };
        if let Some(id) = id_at(pos) {
            self.sel_pos = Some(pos);
            self.selected = Some(id);
        }
    }

    /// Is `id` one of the games of (collapsed) span `s`? Recomputes that group's members.
    fn group_has(&self, lib: &Library, s: &Span, id: Option<GameId>, cx: &GroupCtx) -> bool {
        let Some(id) = id else { return false };
        let Some(i) = lib.index_of(id) else { return false };
        let by = self.groups.by;
        group_games(lib, &[i], by, cx).iter().any(|g| format!("{}:{}", by.key(), g.key) == s.key)
    }

    /// Collapse / expand a group (remembered in the config).
    pub(crate) fn toggle_group(&mut self, st: &mut AppState, key: &str) {
        let c = &mut st.config.desktop.collapsed_groups;
        match c.iter().position(|k| k == key) {
            Some(i) => {
                c.remove(i);
            }
            None => c.push(key.to_string()),
        }
        self.groups.collapsed = c.clone();
        self.mark_config_dirty();
    }

    /// Expand every collapsed group holding `id` (a random pick or a jump lands there).
    pub(crate) fn reveal(&mut self, st: &mut AppState, id: GameId) {
        let by = self.groups.by;
        let Some(i) = st.library.index_of(id) else { return };
        if by == GroupBy::None {
            return;
        }
        let keys: Vec<String> = group_games(&st.library, &[i], by, &GroupCtx::now(by))
            .into_iter()
            .map(|g| format!("{}:{}", by.key(), g.key))
            .collect();
        let c = &mut st.config.desktop.collapsed_groups;
        let before = c.len();
        c.retain(|k| !keys.contains(k));
        if c.len() != before {
            self.groups.collapsed = c.clone();
            self.mark_config_dirty();
        }
    }

    /// Called by the views: a click or the wheel over them counts as the user's own move.
    pub(crate) fn note_input(&mut self, ui: &egui::Ui) {
        if !self.groups.touched
            && ui.rect_contains_pointer(ui.max_rect())
            && ui.input(|i| i.pointer.any_pressed() || i.smooth_scroll_delta != egui::Vec2::ZERO)
        {
            self.groups.touched = true;
        }
    }

    /// The shown position a click on a game at `pos` selects (see `GroupState::clicked_pos`).
    pub(crate) fn note_click(&mut self, pos: usize) {
        self.groups.clicked_pos = Some(pos);
    }

    /// A group header row: chevron, name, count, a hairline. Click toggles it.
    pub(crate) fn group_header(&mut self, ui: &mut egui::Ui, st: &mut AppState, rect: egui::Rect, span: &Span) {
        let pal = self.pal;
        let id = ui.scope_id().with(("ls_group", &span.key));
        let resp = ui.interact(rect, id, egui::Sense::click()).on_hover_text(if span.collapsed {
            "Show these games"
        } else {
            "Hide these games"
        });
        let p = ui.painter();
        let mid = rect.center().y;
        let color = if resp.hovered() { pal.text } else { mix(pal.text, pal.dim, 0.25) };
        let ir = egui::Rect::from_center_size(egui::pos2(rect.left() + 16.0, mid), egui::vec2(12.0, 12.0));
        let icon =
            if span.collapsed { crate::ui::widgets::Icon::ChevronRight } else { crate::ui::widgets::Icon::ChevronDown };
        crate::ui::widgets::paint_icon(p, ir, icon, pal.dim);
        let font = egui::FontId::new(14.0, self.fonts.bold.clone());
        let name = crate::ui::widgets::truncated(p, &span.label, font, color, (rect.width() - 120.0).max(60.0));
        let x = ir.right() + 8.0;
        let nw = name.size().x;
        p.galley(egui::pos2(x, mid - name.size().y / 2.0), name, color);
        let count = p.layout_no_wrap(span.count.to_string(), egui::FontId::proportional(12.5), pal.dim);
        let cx = x + nw + 8.0;
        let cw = count.size().x;
        p.galley(egui::pos2(cx, mid - count.size().y / 2.0), count, pal.dim);
        let line_x = cx + cw + 12.0;
        if line_x < rect.right() - 12.0 {
            p.hline(line_x..=rect.right() - 12.0, mid, egui::Stroke::new(1.0, with_alpha(pal.border, 160)));
        }
        if resp.clicked() {
            self.toggle_group(st, &span.key);
        }
    }
}

use crate::ui::widgets::{mix, with_alpha};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::desktop::tests::fake_game;

    fn lib() -> Library {
        let mut games: Vec<Game> = (1..=12).map(|i| fake_game(i, &format!("Game {i:02}"), Source::Steam)).collect();
        games[0].meta.genres = vec!["RPG".into(), "Action".into()];
        games[1].meta.genres = vec!["rpg".into()];
        games[2].source = Source::Gog;
        games[3].meta.release_year = Some(2020);
        games[4].meta.release_year = Some(2024);
        // An edited release date wins over the fetched year (as in Sort → Release date).
        games[5].meta.release_year = Some(2019);
        games[5].meta.release_date = Some("2025-03-01".into());
        Library::new(games)
    }

    fn names(lib: &Library, gs: &[Group]) -> Vec<(String, usize)> {
        let _ = lib;
        gs.iter().map(|g| (g.label.clone(), g.games.len())).collect()
    }

    #[test]
    fn games_with_several_values_are_in_each_group() {
        let l = lib();
        let view: Vec<usize> = (0..l.games.len()).collect();
        let g = group_games(&l, &view, GroupBy::Genre, &GroupCtx::default());
        assert_eq!(
            names(&l, &g),
            vec![("Action".into(), 1), ("RPG".into(), 2), ("No genre".into(), 10)],
            "case-insensitive values, the no-value group last"
        );
        let y = group_games(&l, &view, GroupBy::ReleaseYear, &GroupCtx::default());
        assert_eq!(y[0].label, "2025", "newest year first, from the full date");
        assert_eq!(y[1].label, "2024");
        assert!(!y.iter().any(|g| g.label == "2019"));
        assert_eq!(y.last().unwrap().label, "Unknown year");
        let s = group_games(&l, &view, GroupBy::Source, &GroupCtx::default());
        assert_eq!(s.iter().map(|g| g.games.len()).sum::<usize>(), 12);
        assert!(group_games(&l, &view, GroupBy::None, &GroupCtx::default()).is_empty());
    }

    #[test]
    fn last_played_and_playtime_buckets() {
        let mut games = vec![];
        let today = 1_800_000_000;
        let cx = GroupCtx { today, week: today - 3 * 86_400, month: today - 10 * 86_400, mounts: vec![] };
        for (i, lp, pt) in [
            (1, Some(today + 60), 3600 * 150),
            (2, Some(today - 86_400), 3600 * 2),
            (3, Some(today - 5 * 86_400), 60),
            (4, Some(today - 50 * 86_400), 0),
            (5, None, 0),
        ] {
            let mut g = fake_game(i, &format!("G{i}"), Source::Steam);
            g.last_played = lp;
            g.playtime_secs = pt;
            games.push(g);
        }
        let l = Library::new(games);
        let view: Vec<usize> = (0..5).collect();
        let lp: Vec<String> = group_games(&l, &view, GroupBy::LastPlayed, &cx).into_iter().map(|g| g.label).collect();
        assert_eq!(lp, ["Today", "This week", "This month", "Earlier", "Never"]);
        let pt: Vec<String> = group_games(&l, &view, GroupBy::Playtime, &cx).into_iter().map(|g| g.label).collect();
        assert_eq!(pt, ["Over 100 hours", "1 to 5 hours", "Under 1 hour", "Not played"]);
    }

    #[test]
    fn install_drive_is_the_longest_mount() {
        let mi = "22 1 0:21 / / rw - btrfs /dev/x rw\n\
                  40 22 0:40 / /home rw - btrfs /dev/x rw\n\
                  41 22 8:1 / /run/media/me/My\\040Games rw - ext4 /dev/sdb1 rw\n";
        let m = parse_mounts(mi);
        assert_eq!(m[0], "/run/media/me/My Games");
        assert_eq!(drive_of(&m, "/run/media/me/My Games/steam/x"), "/run/media/me/My Games");
        assert_eq!(drive_of(&m, "/home/me/Games/x"), "/home");
        assert_eq!(drive_of(&m, "/homework/x"), "/");
        assert_eq!(drive_of(&m, "/opt/x"), "/");
    }

    #[test]
    fn collapsed_groups_keep_their_header_and_count() {
        let l = lib();
        let view: Vec<usize> = (0..l.games.len()).collect();
        let g = group_games(&l, &view, GroupBy::Genre, &GroupCtx::default());
        let (items, spans) = flatten(g, GroupBy::Genre, &["genre:rpg".to_string()]);
        assert_eq!(items.len(), 11, "Action (1) + No genre (10)");
        assert_eq!(spans[1].key, "genre:rpg");
        assert!(spans[1].collapsed && spans[1].len == 0 && spans[1].count == 2);
        assert_eq!(spans[1].start, 1);
        assert_eq!(spans[2].start, 1);
        assert_eq!(run_of(&spans, items.len(), 1), (1, 10));
        assert_eq!(run_of(&spans, items.len(), 0), (0, 1));
    }

    fn span(start: usize, len: usize) -> Span {
        Span { key: String::new(), label: String::new(), count: len, start, len, collapsed: len == 0 }
    }

    #[test]
    fn grid_moves_across_groups_keep_the_column() {
        // Groups of 5, 0 (collapsed), 2 and 7 games in 4 columns.
        let spans = [span(0, 5), span(5, 0), span(5, 2), span(7, 7)];
        let n = 14;
        let down = |p| grid_move(&spans, n, 4, p, true);
        let up = |p| grid_move(&spans, n, 4, p, false);
        assert_eq!(down(0), Some(4));
        assert_eq!(down(2), Some(4), "short last row: the last item");
        assert_eq!(down(4), Some(5), "into the next group (collapsed one skipped), column 0");
        assert_eq!(down(6), Some(8), "column 1 into the last group");
        assert_eq!(down(7), Some(11));
        assert_eq!(down(9), Some(13));
        assert_eq!(down(11), None, "the last row");
        assert_eq!(up(8), Some(6), "column 1 back into the 2-game group");
        assert_eq!(up(10), Some(6), "column 3 clamps to the group's last game");
        assert_eq!(up(5), Some(4), "the last row of the group above, same column (clamped)");
        assert_eq!(up(0), None);
        // Without groups it is the plain grid.
        assert_eq!(grid_move(&[], 10, 4, 5, true), Some(9));
        assert_eq!(grid_move(&[], 10, 4, 8, true), None);
        assert_eq!(grid_move(&[], 10, 4, 6, true), Some(9));
    }

    #[test]
    fn blocks_place_items_under_their_headers() {
        let spans = [span(0, 5), span(5, 0), span(5, 3)];
        let b = Blocks::new(&spans, 8, 4, 100.0, 30.0, (10.0, 10.0));
        assert_eq!(b.item_pos(0), (0, 40.0));
        assert_eq!(b.item_pos(4), (0, 140.0));
        // Collapsed group: header only (30), then the third group's header.
        assert_eq!(b.item_pos(5), (0, 10.0 + 30.0 + 200.0 + 30.0 + 30.0));
        assert_eq!(b.total_h, 10.0 + 230.0 + 30.0 + 130.0 + 10.0);
        let vis = b.visible(0.0, 50.0);
        assert_eq!(vis[0].0, 0);
        assert!(vis[0].1.start == 0 && vis[0].1.len() <= 2);
        let vis = b.visible(250.0, 300.0);
        assert!(vis.iter().any(|v| v.0 == 1));
        assert!(vis.iter().any(|v| v.0 == 2 && v.1 == (0..1)));
        // Ungrouped: the plain list.
        let p = Blocks::new(&[], 3, 1, 32.0, 30.0, (0.0, 0.0));
        assert_eq!(p.item_pos(2), (0, 64.0));
        assert_eq!(p.total_h, 96.0);
    }

    #[test]
    fn grouping_five_thousand_games_is_instant() {
        let games: Vec<Game> = (1..=5000)
            .map(|i| {
                let mut g = fake_game(i, &format!("Game {i}"), if i % 3 == 0 { Source::Gog } else { Source::Steam });
                g.meta.genres = vec![format!("Genre {}", i % 40), "Action".into()];
                g.tags = vec![format!("tag{}", i % 25)];
                g.meta.release_year = Some(1990 + (i % 35) as i32);
                g
            })
            .collect();
        let l = Library::new(games);
        let mut view: Vec<usize> = (0..l.games.len()).collect();
        let t = Instant::now();
        l.sort(&mut view, crate::config::SortBy::Playtime, false);
        for by in GroupBy::ALL {
            let g = group_games(&l, &view, by, &GroupCtx::default());
            let _ = flatten(g, by, &[]);
        }
        let el = t.elapsed();
        // Debug builds are much slower; the release test run is what counts.
        assert!(el < Duration::from_millis(if cfg!(debug_assertions) { 2000 } else { 150 }), "{el:?}");
    }
}
