//! Not-a-game cleanup and bulk edits on a multi-selection: the "…and everything
//! named like this" pattern dialog and "Game options for N games…".

use super::dialogs::{hint_text, modal, plain_button, primary_button, title};
use super::{DesktopUi, Pal, games_word};
use crate::app::AppState;
use crate::config::ExcludeRule;
use crate::exclude;
use crate::game_options::GameOptions;
use crate::game_options_bulk::{self as bulk, GameOptionsPatch};
use crate::model::*;
use egui::{Key, RichText};

// ------------------------------------------------------------------------- name pattern

pub(crate) struct ExcludeLikeDlg {
    id: GameId,
    name: String,
    source: Source,
    pub pattern: String,
    pub only_source: bool,
    focus: bool,
    cache: Option<Preview>,
}

/// The pattern dialog's preview for one (rule, library generation).
struct Preview {
    key: (ExcludeRule, u64),
    matches: usize,
    /// The first few names.
    names: Vec<String>,
    /// The game the dialog was opened on matches.
    this: bool,
}

impl ExcludeLikeDlg {
    pub fn new(g: &Game) -> ExcludeLikeDlg {
        ExcludeLikeDlg {
            id: g.id,
            name: g.name.clone(),
            source: g.source,
            pattern: exclude::suggest_pattern(&g.name),
            // Every library: the same junk (Proton, redistributables) shows up in several.
            only_source: false,
            focus: true,
            cache: None,
        }
    }

    pub fn rule(&self) -> ExcludeRule {
        ExcludeRule {
            pattern: self.pattern.trim().to_string(),
            source: self.only_source.then(|| self.source.key().to_string()),
            enabled: true,
        }
    }

    fn preview(&mut self, st: &AppState) -> (usize, &[String], bool) {
        let key = (self.rule(), st.library.generation);
        if self.cache.as_ref().is_none_or(|c| c.key != key) {
            let hits = exclude::rule_matches(&key.0, st.library.games.iter());
            self.cache = Some(Preview {
                matches: hits.len(),
                names: hits.iter().take(4).map(|g| g.display_name().to_string()).collect(),
                this: hits.iter().any(|g| g.id == self.id),
                key,
            });
        }
        let c = self.cache.as_ref().expect("filled above");
        (c.matches, &c.names, c.this)
    }
}

/// Preview line for the pattern dialog: "Matches 3: Proton 9.0, Proton 8.0, Proton 7.0".
pub(crate) fn matches_line(n: usize, names: &[String]) -> String {
    let mut s = names.iter().map(|n| format!("“{n}”")).collect::<Vec<_>>().join(", ");
    if n > names.len() {
        s.push_str(&format!(" and {} more", n - names.len()));
    }
    format!("Matches {}: {s}", if n == 1 { "1 entry".into() } else { format!("{n} entries") })
}

// ------------------------------------------------------------------------- bulk game options

pub(crate) struct BulkOptsDlg {
    ids: Vec<GameId>,
    /// As edited (starts at what the games share).
    opts: GameOptions,
    /// Last frame's value, to collect what the user changed.
    prev: GameOptions,
    /// Everything changed so far: only this is written.
    pub patch: GameOptionsPatch,
    /// Selected Steam games (Steam starts them: options mostly can't reach them).
    steam: usize,
    /// The one library all selected games are from (its Settings → Games options are their
    /// Default), else None.
    source: Option<Source>,
}

impl BulkOptsDlg {
    pub fn new(st: &AppState, ids: Vec<GameId>) -> Option<BulkOptsDlg> {
        let games: Vec<&Game> = ids.iter().filter_map(|&id| st.library.get(id)).collect();
        let first = games.first()?;
        let source = games.iter().all(|g| g.source == first.source).then_some(first.source);
        let opts = bulk::common(games.iter().map(|g| &g.edits.game_options));
        Some(BulkOptsDlg {
            steam: games.iter().filter(|g| g.source == Source::Steam).count(),
            ids: games.iter().map(|g| g.id).collect(),
            prev: opts.clone(),
            opts,
            patch: GameOptionsPatch::default(),
            source,
        })
    }
}

impl DesktopUi {
    /// The pattern dialog and the bulk options dialog (when open).
    pub(super) fn cleanup_dialogs(&mut self, ctx: &egui::Context, st: &mut AppState) {
        let pal = self.pal;
        let fam = self.fonts.bold.clone();
        if let Some(mut d) = self.dlg.exclude_like.take() {
            match exclude_like_dialog(ctx, st, &mut d, &pal, &fam) {
                Some(true) => {
                    let (rule, id) = (d.rule(), d.id);
                    self.rule_added(ctx, st, rule, id);
                }
                Some(false) => {}
                None => self.dlg.exclude_like = Some(d),
            }
        }
        if let Some(mut d) = self.dlg.bulk_opts.take()
            && bulk_options_dialog(ctx, st, &mut d, &pal, &fam)
        {
            self.dlg.bulk_opts = Some(d);
        }
    }
}

/// None = keep open; Some(true) = add the rule; Some(false) = cancelled.
fn exclude_like_dialog(
    ctx: &egui::Context,
    st: &mut AppState,
    d: &mut ExcludeLikeDlg,
    pal: &Pal,
    fam: &egui::FontFamily,
) -> Option<bool> {
    let mut out = None;
    let total = st.library.games.len();
    let (_, close) = modal(ctx, "ls_exclude_like", pal, 470.0, |ui| {
        title(
            ui,
            "Not a game",
            Some("Remove everything named like this from the library, now and after every scan."),
            pal,
            fam,
        );
        ui.label(RichText::new("Name or pattern").size(13.0).strong().color(pal.text));
        let r = ui
            .add(egui::TextEdit::singleline(&mut d.pattern).id_salt("ls_exclude_pattern").desired_width(f32::INFINITY));
        if d.focus {
            r.request_focus();
            d.focus = false;
        }
        hint_text(ui, "* stands for any text, as in “Proton *”. Upper and lower case don't matter.", pal);
        ui.add_space(4.0);
        ui.checkbox(&mut d.only_source, format!("Only in {}", d.source.display()));
        ui.add_space(8.0);
        let usable = exclude::pattern_usable(&d.pattern);
        let name = d.name.clone();
        let (n, names, this) = d.preview(st);
        let (n, names) = (n, names.to_vec());
        // A pattern that catches a big part of the library is only added by a click, never by
        // an Enter typed while editing it.
        let broad = n > 50 || (n >= 5 && n * 4 > total);
        if !usable {
            hint_text(ui, "Type a name or a pattern.", pal);
        } else if n == 0 {
            hint_text(ui, "Nothing in your library is named like this now; later scans are checked too.", pal);
        } else {
            ui.add(egui::Label::new(RichText::new(matches_line(n, &names)).size(13.0).color(pal.text)).wrap());
            if broad {
                ui.label(
                    RichText::new("That's a lot of your library: check the pattern.").size(12.0).color(pal.danger),
                );
            }
        }
        if usable && !this {
            ui.label(RichText::new(format!("“{name}” itself isn't matched by this.")).size(12.0).color(pal.fav));
        }
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            let label = if n > 0 {
                format!("Remove {}", if n == 1 { "1 entry".into() } else { format!("{n} entries") })
            } else {
                "Add rule".into()
            };
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
            if (primary_button(ui, &label, pal, usable).clicked() || (enter && usable && !broad)) && out.is_none() {
                out = Some(true);
            }
            if plain_button(ui, "Cancel").clicked() {
                out = Some(false);
            }
        });
        ui.add_space(2.0);
        hint_text(ui, "Rules are listed in Settings → Libraries → Hidden & excluded.", pal);
    });
    if close && out.is_none() {
        out = Some(false);
    }
    out
}

/// True while open.
fn bulk_options_dialog(
    ctx: &egui::Context,
    st: &mut AppState,
    d: &mut BulkOptsDlg,
    pal: &Pal,
    fam: &egui::FontFamily,
) -> bool {
    let mut keep = true;
    let n = d.ids.len();
    let body_h = (ctx.content_rect().height() - 330.0).clamp(180.0, 560.0);
    let (_, close) = modal(ctx, "ls_bulk_opts", pal, 640.0, |ui| {
        title(
            ui,
            &format!("Game options for {}", games_word(n)),
            Some("Only what you change here is saved. Every other option keeps each game's own setting."),
            pal,
            fam,
        );
        if d.steam > 0 {
            let msg = if d.steam == n {
                "Steam starts these games, so most options can't reach them from Lodestar. Each game's Launch options shows what to paste into Steam.".to_string()
            } else {
                format!(
                    "{} of them are Steam games: Steam starts those, so most options can't reach them from Lodestar.",
                    d.steam
                )
            };
            ui.label(RichText::new(msg).size(12.0).color(pal.fav));
            ui.add_space(4.0);
        }
        let env = crate::game_options::launch_env();
        let defaults = match d.source {
            Some(s) => crate::game_options::defaults_for(&env, s.key()),
            None => env.defaults.clone(),
        };
        let caps = st.game_opts.caps.clone();
        egui::ScrollArea::vertical().id_salt("ls_bulk_opts_scroll").max_height(body_h).auto_shrink([false, true]).show(
            ui,
            |ui| {
                ui.set_width(ui.available_width() - 12.0);
                let cx = super::game_options_ui::EditorCtx {
                    caps: st.game_opts.probed.then_some(&*caps),
                    global: Some(&defaults),
                    report: &[],
                    salt: "bulk",
                };
                if super::game_options_ui::options_editor(ui, &mut d.opts, &cx, pal) {
                    d.patch.merge(GameOptionsPatch::between(&d.prev, &d.opts));
                    d.prev = d.opts.clone();
                }
            },
        );
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        if d.patch.is_empty() {
            hint_text(ui, "Nothing changed yet.", pal);
        } else {
            ui.add(
                egui::Label::new(RichText::new(bulk::summary_line(&d.patch, n)).size(13.0).strong().color(pal.text))
                    .wrap(),
            );
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if primary_button(ui, "Save", pal, !d.patch.is_empty()).clicked() {
                let changed = st.apply_game_options_patch(&d.ids, &d.patch);
                if changed == 0 {
                    st.notify("Nothing to change: the games already had these options");
                } else {
                    st.notify(format!("Game options saved for {}", games_word(changed)));
                }
                keep = false;
            }
            if plain_button(ui, "Cancel").clicked() {
                keep = false;
            }
        });
    });
    keep && !close
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_line() {
        let names: Vec<String> = ["A", "B", "C", "D"].iter().map(|s| s.to_string()).collect();
        assert_eq!(matches_line(1, &names[..1]), "Matches 1 entry: “A”");
        assert_eq!(matches_line(9, &names), "Matches 9 entries: “A”, “B”, “C”, “D” and 5 more");
    }
}
