//! Missing games (a game no longer in the library it came from, e.g. a non-Steam shortcut
//! deleted in Steam): the "Remove from library…" confirmation (right-click, one game or a
//! multi-selection, with an Undo toast) and Settings → Libraries → Missing games (select,
//! Remove selected / Remove all / Keep, with Undo on the page).

use super::dialogs::{Buttons, footer, hint_text, modal, title};
use super::hidden_ui::{card, section_with};
use super::undo::UndoWhat;
use super::{DesktopUi, Pal};
use crate::app::AppState;
use crate::missing_app::{RemovedGames, one_source};
use crate::model::{Game, GameId, Source, now_unix};
use crate::ui::widgets::{Icon, mix, paint_icon, with_alpha};
use egui::{Align, Layout, RichText, Sense, Ui, vec2};
use std::collections::BTreeMap;

/// "since today" / "since yesterday" / "for 5 days" / "since 2026-10-01".
pub(crate) fn missing_for(since: i64, now: i64) -> String {
    match (now - since).max(0) / 86_400 {
        0 => "since today".into(),
        1 => "since yesterday".into(),
        d if d < 30 => format!("for {d} days"),
        _ => format!("since {}", ymd(since)),
    }
}

/// "2024-03-05" from unix seconds (UTC).
fn ymd(ts: i64) -> String {
    // Days to civil date (H. Hinnant's algorithm).
    let z = ts.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

/// "Steam: 2 · Faugus: 1" for these games.
fn per_source(games: &[&Game]) -> String {
    let mut per: BTreeMap<&'static str, usize> = BTreeMap::new();
    for g in games {
        *per.entry(g.source.display()).or_default() += 1;
    }
    per.into_iter().map(|(s, n)| format!("{s}: {n}")).collect::<Vec<_>>().join(" · ")
}

/// The confirmation's text, naming the library.
fn confirm_text(st: &AppState, ids: &[GameId]) -> String {
    let games: Vec<&Game> = ids.iter().filter_map(|id| st.library.get(*id)).collect();
    match games.as_slice() {
        [g] => {
            let t = g.source.missing_text();
            let lower = t[..1].to_lowercase() + &t[1..];
            format!(
                "“{}” is {lower}. Remove it from your Lodestar library? Its play time, tags and other \
                 details go with it. No files are touched.",
                g.display_name()
            )
        }
        _ => {
            let n = games.len();
            let lead = match one_source(st, ids) {
                Some(Source::Desktop) => format!("{n} games are no longer in the app menu."),
                Some(Source::Emulator) => format!("{n} games are no longer in their ROM folders."),
                Some(s) => format!("{n} games are no longer in {}.", s.display()),
                None => format!("{n} games are no longer in the libraries they came from ({}).", per_source(&games)),
            };
            format!(
                "{lead} Remove them from your Lodestar library? Their play time, tags and other details \
                 go with them. No files are touched."
            )
        }
    }
}

fn removed_text(st: &AppState, ids: &[GameId]) -> String {
    match ids {
        [id] => format!("Removed “{}” from the library", st.library.get(*id).map(|g| g.display_name()).unwrap_or("")),
        _ => format!("Removed {} games from the library", ids.len()),
    }
}

impl DesktopUi {
    /// Right-click → "Remove from library…" on missing games.
    pub(super) fn missing_dialog(&mut self, ctx: &egui::Context, st: &mut AppState) {
        let Some(ids) = &self.dlg.remove_missing else { return };
        let ids: Vec<GameId> =
            ids.iter().copied().filter(|id| st.library.get(*id).is_some_and(|g| g.missing.is_some())).collect();
        if ids.is_empty() {
            self.dlg.remove_missing = None;
            return;
        }
        let pal = self.pal;
        let fam = self.fonts.bold.clone();
        let text = confirm_text(st, &ids);
        let names: Vec<String> =
            ids.iter().filter_map(|id| st.library.get(*id)).map(|g| g.display_name().to_string()).collect();
        let mut answer = None;
        let (_, close) = modal(ctx, "ls_remove_missing_dlg", &pal, 420.0, |ui| {
            title(ui, if ids.len() == 1 { "Remove game?" } else { "Remove games?" }, None, &pal, &fam);
            ui.label(RichText::new(text).color(pal.text));
            if names.len() > 1 {
                ui.add_space(6.0);
                const SHOWN: usize = 6;
                for n in names.iter().take(SHOWN) {
                    ui.label(RichText::new(format!("•  {n}")).size(12.5).color(pal.dim));
                }
                if names.len() > SHOWN {
                    ui.label(RichText::new(format!("…and {} more", names.len() - SHOWN)).size(12.5).color(pal.dim));
                }
            }
            let f = footer(ui, &pal, Buttons::primary("Remove", true).danger().cancel("Cancel"), |_| {});
            if f.primary {
                answer = Some(true);
            }
            if f.cancel {
                answer = Some(false);
            }
        });
        if answer == Some(true) {
            let text = removed_text(st, &ids);
            self.step_off(st, &ids);
            if let Some(r) = st.remove_missing(&ids) {
                self.show_undo(ctx, UndoWhat::Removed(r), text, ids.first().copied());
            }
        }
        if answer.is_some() || close {
            self.dlg.remove_missing = None;
        }
    }
}

/// Under the Play row of a missing game: "No longer in Steam".
pub(crate) fn page_hint(ui: &mut Ui, g: &Game, pal: &Pal) {
    let Some(m) = g.missing else { return };
    ui.add_space(6.0);
    let r = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (rect, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
            paint_icon(ui.painter(), rect, Icon::InfoCircle, pal.dim);
            ui.label(RichText::new(g.source.missing_text()).size(12.5).color(pal.dim));
        })
        .response;
    r.on_hover_text(format!(
        "Lodestar didn't find it in {} {}. Right-click it → Remove from library… to take it out, or keep it: \
         Settings → Libraries → Missing games.",
        g.source.display(),
        missing_for(m.since, now_unix())
    ));
}

/// The game page's note for a Steam game that is no longer in the account's Steam library
/// (steam_owned.rs); its Install button says Open in Steam.
pub(crate) fn steam_library_hint(ui: &mut Ui, caps: Option<&super::StoreCaps>, pal: &Pal) {
    if !caps.is_some_and(|c| c.not_in_steam_library) {
        return;
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
        paint_icon(ui.painter(), rect, Icon::InfoCircle, pal.dim);
        ui.label(RichText::new("Not in your Steam library").size(12.5).strong().color(pal.dim));
    });
    ui.add(
        egui::Label::new(
            RichText::new(
                "It may have come with a subscription such as EA Play that has ended, or from someone's \
                 shared library.",
            )
            .size(12.5)
            .color(pal.dim),
        )
        .wrap(),
    );
}

// ------------------------------------------------------------------ Settings → Libraries

/// Settings → Libraries → Missing games.
#[derive(Default)]
pub(crate) struct MissingPage {
    selected: Vec<GameId>,
    show_kept: bool,
    /// The last change, with its Undo (stays until the next change or the page closes).
    undo: Option<(String, PageUndo)>,
}

enum PageUndo {
    Removed(RemovedGames),
    Kept(Vec<GameId>),
}

/// The Libraries tab's entry: counts and a way in. True = open the page.
pub(crate) fn summary(ui: &mut Ui, st: &AppState, pal: &Pal) -> bool {
    let listed: Vec<&Game> = st.library.missing().collect();
    let kept = st.library.games.iter().filter(|g| g.missing.is_some_and(|m| m.kept)).count();
    if listed.is_empty() && kept == 0 {
        hint_text(
            ui,
            "Nothing missing. A game that disappears from its library (for example a shortcut deleted in Steam) \
             is listed here, so you can remove it or keep it.",
            pal,
        );
        return false;
    }
    let mut open = false;
    card(ui, pal, |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
        let resp = ui.interact(rect, ui.scope_id().with("ls_missing_card"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(rect.expand2(vec2(8.0, 2.0)), 6.0, with_alpha(pal.text, 8));
        }
        let head = match listed.len() {
            0 => "Nothing missing".to_string(),
            1 => "1 game is no longer in its library".to_string(),
            n => format!("{n} games are no longer in their libraries"),
        };
        let mut sub = per_source(&listed);
        if kept > 0 {
            if !sub.is_empty() {
                sub.push_str(" · ");
            }
            sub.push_str(&format!("{kept} kept"));
        }
        let p = ui.painter();
        p.text(
            rect.left_top() + vec2(0.0, 4.0),
            egui::Align2::LEFT_TOP,
            head,
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
        resp.on_hover_text("See them, remove them or keep them");
    });
    open
}

const ROW_H: f32 = 32.0;

/// One row: [x] name … library · since. True = the row was clicked (toggle).
fn row(ui: &mut Ui, g: &Game, on: bool, now: i64, button: Option<&str>, pal: &Pal) -> (bool, bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::hover());
    let resp = ui.interact(rect, ui.scope_id().with(("ls_missing_row", g.id)), Sense::click());
    if on {
        ui.painter().rect_filled(rect, 4.0, with_alpha(pal.selection, 34));
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, 4.0, with_alpha(pal.text, 8));
    }
    let mut clicked_button = false;
    let mut child = ui.new_child(
        egui::UiBuilder::new().max_rect(rect.shrink2(vec2(6.0, 0.0))).layout(Layout::left_to_right(Align::Center)),
    );
    if button.is_none() {
        let mut v = on;
        if child.checkbox(&mut v, "").changed() {
            return (true, false);
        }
    }
    child.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if let Some(b) = button {
            clicked_button = ui.small_button(b).clicked();
            ui.add_space(6.0);
        }
        let since = g.missing.map(|m| missing_for(m.since, now)).unwrap_or_default();
        ui.label(RichText::new(format!("{} · {since}", g.source.display())).size(12.0).color(pal.dim));
        ui.add_space(8.0);
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.add(
                egui::Label::new(RichText::new(g.display_name()).size(13.5).color(pal.text))
                    .truncate()
                    .selectable(false),
            );
        });
    });
    (button.is_none() && resp.clicked() && !clicked_button, clicked_button)
}

/// The page. True = "‹ Libraries" was clicked (close the page).
pub(crate) fn missing_page(ui: &mut Ui, st: &mut AppState, page: &mut MissingPage, pal: &Pal) -> bool {
    let mut back = false;
    if ui.add(egui::Button::new(RichText::new("‹ Libraries").color(pal.selection)).frame(false)).clicked() {
        back = true;
    }
    ui.add_space(2.0);
    ui.label(RichText::new("Missing games").size(15.0).strong().color(pal.text));
    hint_text(
        ui,
        "These games are no longer in the library they came from, for example a shortcut deleted in Steam. \
         Lodestar keeps them until you decide: they may have your play time, favourites, tags or notes. \
         Remove the ones you don't need; Keep leaves a game in your library and stops listing it here.",
        pal,
    );
    let now = now_unix();
    let listed: Vec<GameId> = st.library.missing().map(|g| g.id).collect();
    page.selected.retain(|id| listed.contains(id));

    // ---- the last change, with Undo
    let mut undo = false;
    if let Some((text, _)) = &page.undo {
        ui.add_space(8.0);
        egui::Frame::new()
            .fill(mix(pal.surface, pal.surface_alt, 0.8))
            .stroke(egui::Stroke::new(1.0, pal.border))
            .corner_radius(6)
            .inner_margin(egui::Margin::symmetric(12, 6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(text).color(pal.text));
                    ui.add_space(8.0);
                    undo = ui
                        .add(egui::Button::new(RichText::new("Undo").color(pal.selection).strong()).frame(false))
                        .clicked();
                });
            });
    }
    if undo && let Some((_, what)) = page.undo.take() {
        match what {
            PageUndo::Removed(r) => st.restore_removed(&r),
            PageUndo::Kept(ids) => st.keep_missing(&ids, false),
        }
    }

    // ---- actions
    let mut remove: Option<Vec<GameId>> = None;
    let mut keep: Option<Vec<GameId>> = None;
    ui.add_space(10.0);
    if !listed.is_empty() {
        ui.horizontal(|ui| {
            let n = page.selected.len();
            let all_on = n == listed.len();
            if ui.button(if all_on { "Select none" } else { "Select all" }).clicked() {
                page.selected = if all_on { vec![] } else { listed.clone() };
            }
            ui.add_space(6.0);
            let label = if n > 0 { format!("Remove selected ({n})") } else { "Remove selected".into() };
            if ui
                .add_enabled(n > 0, egui::Button::new(label))
                .on_disabled_hover_text("Tick the games to remove first")
                .clicked()
            {
                remove = Some(page.selected.clone());
            }
            if ui
                .add_enabled(n > 0, egui::Button::new("Keep"))
                .on_hover_text("Leave them in your library and stop listing them here")
                .on_disabled_hover_text("Tick the games to keep first")
                .clicked()
            {
                keep = Some(page.selected.clone());
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button(format!("Remove all {}", listed.len())).clicked() {
                    remove = Some(listed.clone());
                }
            });
        });
        ui.add_space(6.0);
        let games: Vec<&Game> = listed.iter().filter_map(|id| st.library.get(*id)).collect();
        let mut toggle = None;
        card(ui, pal, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            egui::ScrollArea::vertical()
                .id_salt("ls_missing_list")
                .max_height(ROW_H * 10.0)
                .auto_shrink([false, true])
                .show_rows(ui, ROW_H, games.len(), |ui, range| {
                    for g in &games[range] {
                        let on = page.selected.contains(&g.id);
                        if row(ui, g, on, now, None, pal).0 {
                            toggle = Some(g.id);
                        }
                    }
                });
        });
        if let Some(id) = toggle {
            if let Some(i) = page.selected.iter().position(|s| *s == id) {
                page.selected.remove(i);
            } else {
                page.selected.push(id);
            }
        }
    } else {
        hint_text(ui, "Nothing missing right now.", pal);
    }

    // ---- kept
    let kept: Vec<&Game> = st.library.games.iter().filter(|g| g.missing.is_some_and(|m| m.kept)).collect();
    if !kept.is_empty() {
        let label = if page.show_kept { "Hide" } else { "Show" };
        if section_with(ui, &format!("Kept in your library · {}", kept.len()), Some(label), pal) {
            page.show_kept = !page.show_kept;
        }
        if page.show_kept {
            let mut out = None;
            card(ui, pal, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                egui::ScrollArea::vertical()
                    .id_salt("ls_missing_kept")
                    .max_height(ROW_H * 6.0)
                    .auto_shrink([false, true])
                    .show_rows(ui, ROW_H, kept.len(), |ui, range| {
                        for g in &kept[range] {
                            if row(ui, g, false, now, Some("Remove"), pal).1 {
                                out = Some(g.id);
                            }
                        }
                    });
            });
            if let Some(id) = out {
                remove = Some(vec![id]);
            }
        }
    }
    ui.add_space(8.0);

    if let Some(ids) = remove {
        let text = removed_text(st, &ids);
        if let Some(r) = st.remove_missing(&ids) {
            page.undo = Some((text, PageUndo::Removed(r)));
        }
        page.selected.clear();
    }
    if let Some(ids) = keep {
        let text = match ids.as_slice() {
            [id] => format!("Kept “{}” in your library", st.library.get(*id).map(|g| g.display_name()).unwrap_or("")),
            _ => format!("Kept {} games in your library", ids.len()),
        };
        st.keep_missing(&ids, true);
        page.undo = Some((text, PageUndo::Kept(ids)));
        page.selected.clear();
    }
    back
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use crate::model::ScannedGame;

    #[test]
    fn wording_names_the_library() {
        assert_eq!(missing_for(1000, 1000 + 3600), "since today");
        assert_eq!(missing_for(0, 86_400 + 5), "since yesterday");
        assert_eq!(missing_for(0, 86_400 * 5), "for 5 days");
        assert_eq!(missing_for(0, 86_400 * 90), "since 1970-01-01");
        assert_eq!(ymd(1_709_164_800), "2024-02-29");

        let dir = tempfile::tempdir().unwrap();
        let mut st = AppState::for_test(dir.path());
        let scan = [
            ScannedGame::new(Source::Steam, "shortcut:1", "Ghost"),
            ScannedGame::new(Source::Steam, "shortcut:2", "Ghost 2"),
            ScannedGame::new(Source::Faugus, "f", "Wine thing"),
            ScannedGame::new(Source::Steam, "shortcut:3", "Stays"),
            ScannedGame::new(Source::Faugus, "g", "Stays too"),
        ];
        let mut db = Db::open(&st.paths.db_file()).unwrap();
        db.merge_scan(&scan, &[Source::Steam, Source::Faugus]).unwrap();
        for g in db.load_all().unwrap() {
            db.add_playtime(g.id, 60).unwrap();
        }
        db.merge_scan(&scan[3..], &[Source::Steam, Source::Faugus]).unwrap();
        st.reload_library();
        let id = |n: &str| st.library.games.iter().find(|g| g.name == n).unwrap().id;
        let one = confirm_text(&st, &[id("Ghost")]);
        assert!(one.starts_with("“Ghost” is no longer in Steam."), "{one}");
        let two = confirm_text(&st, &[id("Ghost"), id("Ghost 2")]);
        assert!(two.starts_with("2 games are no longer in Steam."), "{two}");
        let mixed = confirm_text(&st, &[id("Ghost"), id("Ghost 2"), id("Wine thing")]);
        assert!(mixed.contains("(Faugus: 1 · Steam: 2)"), "{mixed}");
    }
}
