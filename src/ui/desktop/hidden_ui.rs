//! Settings → Libraries → Hidden & excluded: one page with the hidden games,
//! the entries that are not games and the rules, searchable, with "Show again" / "Remove rule"
//! and switches per rule. Every change applies at once (no rescan).

use super::Pal;
use super::dialogs::hint_text;
use crate::app::AppState;
use crate::config::ExcludeRule;
use crate::exclude::{self, BUILTIN, Key, Mark, Reason};
use crate::model::{GameId, Source};
use crate::ui::widgets::{Icon, mix, paint_icon, toggle_switch, with_alpha};
use egui::{Align, Layout, RichText, Sense, Ui, vec2};

#[derive(Default)]
pub(crate) struct HiddenPage {
    pub search: String,
    new_pattern: String,
    new_source: Option<Source>,
    /// Rows for (library generation, exclusion generation, search).
    cache: Option<((u64, u64, String), Rows)>,
}

#[derive(Default)]
struct Rows {
    hidden: Vec<Row>,
    excluded: Vec<Row>,
    /// Entries caught per user rule / built-in rule (first rule that matches).
    rule_counts: Vec<usize>,
    builtin_counts: Vec<usize>,
    /// (library, hidden, excluded), libraries with any.
    per_source: Vec<(Source, usize, usize)>,
    total_hidden: usize,
    total_excluded: usize,
}

struct Row {
    key: Key,
    id: Option<GameId>,
    name: String,
    why: String,
}

/// A change asked for on the page (applied after drawing).
enum Op {
    ShowHidden(Vec<GameId>),
    ShowExcluded(Vec<Key>),
    Builtin(&'static str, bool),
    RuleOn(usize, bool),
    RemoveRule(usize),
    AddRule(ExcludeRule),
}

fn rows(st: &AppState, search: &str) -> Rows {
    let q = search.trim().to_lowercase();
    let hit = |name: &str, src: Source| {
        q.is_empty() || name.to_lowercase().contains(&q) || src.display().to_lowercase().contains(&q)
    };
    let mut r = Rows {
        rule_counts: vec![0; st.config.exclude.rules.len()],
        builtin_counts: vec![0; BUILTIN.len()],
        ..Default::default()
    };
    let mut per: std::collections::BTreeMap<&'static str, (Source, usize, usize)> = Default::default();
    for g in st.library.games.iter().filter(|g| g.hidden) {
        r.total_hidden += 1;
        per.entry(g.source.display()).or_insert((g.source, 0, 0)).1 += 1;
        if hit(g.display_name(), g.source) {
            r.hidden.push(Row {
                key: (g.source, g.source_id.clone()),
                id: Some(g.id),
                name: g.display_name().to_string(),
                why: g.source.display().to_string(),
            });
        }
    }
    for g in &st.excl.games {
        r.total_excluded += 1;
        per.entry(g.source.display()).or_insert((g.source, 0, 0)).2 += 1;
        let reason = st.excl.reason(g);
        let why = match reason {
            Some(Reason::Rule(i)) => {
                if let Some(c) = r.rule_counts.get_mut(i) {
                    *c += 1;
                }
                let p = st.config.exclude.rules.get(i).map(|r| r.pattern.as_str()).unwrap_or("");
                format!("{} · rule “{p}”", g.source.display())
            }
            Some(Reason::Builtin(i)) => {
                r.builtin_counts[i] += 1;
                format!("{} · {}", g.source.display(), BUILTIN[i].label.to_lowercase())
            }
            Some(Reason::Marked) | None => format!("{} · marked by you", g.source.display()),
        };
        if hit(g.display_name(), g.source) {
            r.excluded.push(Row {
                key: (g.source, g.source_id.clone()),
                id: Some(g.id),
                name: g.display_name().to_string(),
                why,
            });
        }
    }
    // Marks whose game the last scans didn't see (uninstalled, renamed file): still listed, so
    // they can be forgotten.
    for m in st.excl.marks.iter().filter(|m| m.mark == Mark::NotGame) {
        if st.excl.games.iter().any(|g| g.source == m.source && g.source_id == m.source_id) {
            continue;
        }
        r.total_excluded += 1;
        if hit(&m.name, m.source) {
            r.excluded.push(Row {
                key: (m.source, m.source_id.clone()),
                id: None,
                name: m.name.clone(),
                why: format!("{} · not found by the last scan", m.source.display()),
            });
        }
    }
    r.hidden.sort_by_cached_key(|x| x.name.to_lowercase());
    r.excluded.sort_by_cached_key(|x| x.name.to_lowercase());
    r.per_source = per.into_values().collect();
    r
}

fn section(ui: &mut Ui, text: &str, pal: &Pal) {
    section_with(ui, text, None, pal);
}

/// A section heading with an optional small button on the right. True = clicked.
pub(super) fn section_with(ui: &mut Ui, text: &str, button: Option<&str>, pal: &Pal) -> bool {
    ui.add_space(14.0);
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(text.to_uppercase()).size(11.0).strong().color(pal.dim).extra_letter_spacing(0.8));
        if let Some(b) = button {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                clicked = ui.small_button(b).clicked();
            });
        }
    });
    ui.add_space(4.0);
    clicked
}

pub(super) fn card<R>(ui: &mut Ui, pal: &Pal, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(mix(pal.surface, pal.surface_alt, 0.55))
        .stroke(egui::Stroke::new(1.0, with_alpha(pal.border, 160)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

const ROW_H: f32 = 30.0;

/// Name … why · [button], one line.
fn entry_row(ui: &mut Ui, name: &str, why: &str, button: &str, pal: &Pal) -> bool {
    let mut clicked = false;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), ROW_H), Layout::right_to_left(Align::Center), |ui| {
        clicked = ui.small_button(button).clicked();
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(why).size(12.0).color(pal.dim)).truncate());
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(name).size(13.5).color(pal.text)).truncate());
        });
    });
    clicked
}

/// A list of rows, virtualised (a long hidden list stays fast).
fn entry_list(ui: &mut Ui, salt: &str, list: &[Row], button: &str, pal: &Pal, mut on: impl FnMut(&Row)) {
    if list.is_empty() {
        return;
    }
    card(ui, pal, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        egui::ScrollArea::vertical().id_salt(salt).max_height(ROW_H * 8.0).auto_shrink([false, true]).show_rows(
            ui,
            ROW_H,
            list.len(),
            |ui, range| {
                for row in &list[range] {
                    if entry_row(ui, &row.name, &row.why, button, pal) {
                        on(row);
                    }
                }
            },
        );
    });
}

/// The page. True = "‹ Libraries" was clicked (close the page).
pub(crate) fn hidden_page(ui: &mut Ui, st: &mut AppState, page: &mut HiddenPage, pal: &Pal) -> bool {
    let mut back = false;
    let mut ops: Vec<Op> = vec![];
    if ui.add(egui::Button::new(RichText::new("‹ Libraries").color(pal.selection)).frame(false)).clicked() {
        back = true;
    }
    ui.add_space(2.0);
    ui.label(RichText::new("Hidden & excluded").size(15.0).strong().color(pal.text));
    hint_text(
        ui,
        "Hidden games stay in the library (Library → Hidden). Entries that are not games are out of every view, and rescans don't bring them back.",
        pal,
    );
    ui.add_space(6.0);
    ui.add(
        egui::TextEdit::singleline(&mut page.search)
            .id_salt("ls_hidden_search")
            .hint_text("Search hidden and excluded")
            .desired_width(280.0),
    );

    let key = (st.library.generation, st.excl.generation, page.search.clone());
    if page.cache.as_ref().is_none_or(|c| c.0 != key) {
        page.cache = Some((key, rows(st, &page.search)));
    }
    let r = &page.cache.as_ref().expect("filled above").1;

    if !r.per_source.is_empty() {
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(14.0, 4.0);
            for (s, h, e) in &r.per_source {
                let mut parts = vec![];
                if *h > 0 {
                    parts.push(format!("{h} hidden"));
                }
                if *e > 0 {
                    parts.push(format!("{e} excluded"));
                }
                ui.label(RichText::new(format!("{}: {}", s.display(), parts.join(", "))).size(12.0).color(pal.dim));
            }
        });
    }

    // ---- hidden
    let all = (r.hidden.len() > 1).then(|| format!("Show all {} again", r.hidden.len()));
    if section_with(ui, &format!("Hidden · {}", r.total_hidden), all.as_deref(), pal) {
        ops.push(Op::ShowHidden(r.hidden.iter().filter_map(|x| x.id).collect()));
    }
    if r.total_hidden == 0 {
        hint_text(ui, "No hidden games. Right-click a game → Hide.", pal);
    } else if r.hidden.is_empty() {
        hint_text(ui, "No hidden game matches the search.", pal);
    } else {
        entry_list(ui, "ls_hidden_list", &r.hidden, "Show again", pal, |row| {
            ops.push(Op::ShowHidden(row.id.into_iter().collect()));
        });
    }

    // ---- not games
    section(ui, &format!("Not games · {}", r.total_excluded), pal);
    if r.total_excluded == 0 {
        hint_text(ui, "Nothing excluded yet. Right-click an entry → Not a game.", pal);
    } else if r.excluded.is_empty() {
        hint_text(ui, "No excluded entry matches the search.", pal);
    } else {
        hint_text(
            ui,
            "Show again brings an entry back; one a rule caught stays back while the rule keeps working for the rest.",
            pal,
        );
        ui.add_space(4.0);
        entry_list(ui, "ls_excluded_list", &r.excluded, "Show again", pal, |row| {
            ops.push(Op::ShowExcluded(vec![row.key.clone()]));
        });
    }

    // ---- rules
    section(ui, "Rules", pal);
    hint_text(ui, "Entries named like a rule are left out, now and after every scan. * stands for any text.", pal);
    ui.add_space(4.0);
    card(ui, pal, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let q = page.search.trim().to_lowercase();
        for (i, b) in BUILTIN.iter().enumerate() {
            if !q.is_empty() && !b.label.to_lowercase().contains(&q) && !b.shown.to_lowercase().contains(&q) {
                continue;
            }
            let on = !st.config.exclude.builtin_off.iter().any(|o| o == b.id);
            if let Some(v) =
                rule_row(ui, on, b.label, &format!("{} · built in", b.shown), r.builtin_counts[i], None, pal).0
            {
                ops.push(Op::Builtin(b.id, v));
            }
        }
        for (i, rule) in st.config.exclude.rules.iter().enumerate() {
            if !q.is_empty() && !rule.pattern.to_lowercase().contains(&q) {
                continue;
            }
            let scope = match exclude::rule_scope(rule) {
                Some(Some(s)) => format!("only {}", s.display()),
                Some(None) => "every library".to_string(),
                None => format!("only “{}” (unknown library)", rule.source.as_deref().unwrap_or("")),
            };
            let usable = exclude::pattern_usable(&rule.pattern) && exclude::rule_scope(rule).is_some();
            let sub = if usable { scope } else { format!("{scope} · matches nothing") };
            let count = r.rule_counts.get(i).copied().unwrap_or(0);
            let (toggled, removed) = rule_row(ui, rule.enabled, &rule.pattern, &sub, count, Some("Remove rule"), pal);
            if let Some(v) = toggled {
                ops.push(Op::RuleOn(i, v));
            }
            if removed {
                ops.push(Op::RemoveRule(i));
            }
        }
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut page.new_pattern)
                    .id_salt("ls_new_rule")
                    .hint_text("New rule, e.g. *Benchmark*")
                    .desired_width((ui.available_width() - 250.0).clamp(140.0, 300.0)),
            );
            let label =
                page.new_source.map(|s| format!("Only {}", s.display())).unwrap_or_else(|| "Every library".into());
            egui::ComboBox::from_id_salt("ls_new_rule_scope").selected_text(label).width(130.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut page.new_source, None, "Every library");
                for (s, _) in super::libraries::builtin_libraries().map(|(s, n, _)| (s, n)) {
                    ui.selectable_value(&mut page.new_source, Some(s), format!("Only {}", s.display()));
                }
            });
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let usable = exclude::pattern_usable(&page.new_pattern);
            if (ui.add_enabled(usable, egui::Button::new("Add rule")).clicked() || (enter && usable)) && usable {
                ops.push(Op::AddRule(ExcludeRule {
                    pattern: page.new_pattern.trim().to_string(),
                    source: page.new_source.map(|s| s.key().to_string()),
                    enabled: true,
                }));
                page.new_pattern.clear();
            }
        });
    });
    ui.add_space(8.0);

    for op in ops {
        match op {
            Op::ShowHidden(ids) => st.set_hidden_many(&ids, false),
            Op::ShowExcluded(keys) => st.show_again(&keys),
            Op::Builtin(id, on) => {
                st.config.exclude.builtin_off.retain(|o| o != id);
                if !on {
                    st.config.exclude.builtin_off.push(id.to_string());
                }
                st.exclude_config_changed();
            }
            Op::RuleOn(i, on) => {
                if let Some(r) = st.config.exclude.rules.get_mut(i) {
                    r.enabled = on;
                }
                st.exclude_config_changed();
            }
            Op::RemoveRule(i) => {
                if i < st.config.exclude.rules.len() {
                    st.config.exclude.rules.remove(i);
                }
                st.exclude_config_changed();
            }
            Op::AddRule(rule) => {
                st.add_exclude_rule(rule);
            }
        }
    }
    back
}

/// switch · label / sub · count · [button]. Returns (switched to, button clicked).
fn rule_row(
    ui: &mut Ui,
    on: bool,
    label: &str,
    sub: &str,
    count: usize,
    button: Option<&str>,
    pal: &Pal,
) -> (Option<bool>, bool) {
    let mut toggled = None;
    let mut clicked = false;
    ui.allocate_ui_with_layout(vec2(ui.available_width(), 40.0), Layout::left_to_right(Align::Center), |ui| {
        let mut v = on;
        if toggle_switch(ui, &mut v, 18.0, pal.accent)
            .on_hover_text(if on { "On: click to switch it off" } else { "Off: click to switch it on" })
            .changed()
        {
            toggled = Some(v);
        }
        ui.add_space(8.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(b) = button {
                clicked = ui.small_button(b).clicked();
                ui.add_space(6.0);
            }
            let n = match (on, count) {
                (false, _) => "off".to_string(),
                (true, 0) => "nothing caught".to_string(),
                (true, 1) => "1 entry".to_string(),
                (true, n) => format!("{n} entries"),
            };
            ui.label(RichText::new(n).size(12.0).color(if on && count > 0 { pal.text } else { pal.dim }));
            ui.add_space(8.0);
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(3.0);
                ui.add(
                    egui::Label::new(RichText::new(label).size(13.5).color(if on { pal.text } else { pal.dim }))
                        .truncate(),
                );
                ui.add(egui::Label::new(RichText::new(sub).size(11.5).color(pal.dim)).truncate());
            });
        });
    });
    (toggled, clicked)
}

/// The Libraries tab's entry to the page: counts and "Manage…". True = open the page.
pub(crate) fn summary_card(ui: &mut Ui, st: &AppState, pal: &Pal) -> bool {
    let hidden = st.library.games.iter().filter(|g| g.hidden).count();
    let excluded = st.excl.games.len()
        + st.excl
            .marks
            .iter()
            .filter(|m| m.mark == Mark::NotGame)
            .filter(|m| !st.excl.games.iter().any(|g| g.source == m.source && g.source_id == m.source_id))
            .count();
    let builtin_on = BUILTIN.iter().filter(|b| !st.config.exclude.builtin_off.iter().any(|o| o == b.id)).count();
    let own = st.config.exclude.rules.len();
    let mut open = false;
    card(ui, pal, |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
        let resp = ui.interact(rect, ui.scope_id().with("ls_hidden_card"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect.expand2(vec2(8.0, 2.0)), 6.0, with_alpha(pal.text, 8));
        }
        let p = ui.painter();
        let mut sub =
            format!("{hidden} hidden · {excluded} not games · {builtin_on} of {} built-in rules on", BUILTIN.len());
        if own > 0 {
            sub.push_str(&format!(" · {own} own rule{}", if own == 1 { "" } else { "s" }));
        }
        p.text(
            rect.left_top() + vec2(0.0, 4.0),
            egui::Align2::LEFT_TOP,
            "Hidden games, not games and rules",
            egui::FontId::proportional(14.0),
            pal.text,
        );
        p.text(
            rect.left_top() + vec2(0.0, 23.0),
            egui::Align2::LEFT_TOP,
            sub,
            egui::FontId::proportional(11.5),
            pal.dim,
        );
        paint_icon(
            p,
            egui::Rect::from_center_size(rect.right_center() - vec2(8.0, 0.0), vec2(13.0, 13.0)),
            Icon::ChevronRight,
            pal.dim,
        );
        open = resp.clicked();
        resp.on_hover_text("See hidden games, entries that are not games, and the rules");
    });
    open
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::model::ScannedGame;

    #[test]
    fn rows_count_reasons_and_search() {
        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let games = [
            ScannedGame::new(Source::Desktop, "u", "Uninstall Foo"),
            ScannedGame::new(Source::Lutris, "p", "Proton 9.0"),
            ScannedGame::new(Source::Lutris, "a", "Alpha"),
            ScannedGame::new(Source::Lutris, "b", "Beta"),
        ];
        Db::open(&st.paths.db_file()).unwrap().merge_scan(&games, &[Source::Desktop, Source::Lutris]).unwrap();
        st.reload_library();
        let beta = st.library.games.iter().find(|g| g.name == "Beta").unwrap().id;
        let alpha = st.library.games.iter().find(|g| g.name == "Alpha").unwrap().id;
        st.set_hidden(beta, true);
        st.exclude_games(&[alpha]);
        let r = rows(&st, "");
        assert_eq!((r.total_hidden, r.total_excluded), (1, 3));
        assert_eq!(r.builtin_counts[0], 1, "proton");
        assert_eq!(r.builtin_counts[3], 1, "uninstall");
        assert!(r.excluded.iter().any(|x| x.name == "Alpha" && x.why.contains("marked by you")));
        assert_eq!(r.per_source.iter().find(|p| p.0 == Source::Lutris).map(|p| (p.1, p.2)), Some((1, 2)));
        let r = rows(&st, "proton");
        assert_eq!((r.excluded.len(), r.hidden.len()), (1, 0));
        // A marked entry whose game is gone is still listed.
        Db::open(&st.paths.db_file()).unwrap().delete(alpha).unwrap();
        st.reload_library();
        let r = rows(&st, "alpha");
        assert_eq!(r.excluded.len(), 1);
        assert!(r.excluded[0].why.contains("not found"));
    }
}
