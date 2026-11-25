//! Download artwork for many games: the bulk dialog and its status-bar progress.

use super::*;

// ------------------------------------------------------------------------------- bulk dialog

/// Which games a bulk download covers.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BulkScope {
    /// One game.
    Game(GameId),
    /// The games in the current (filtered) view.
    View,
    /// The multi-selection.
    Selection,
    /// An explorer entry ("Platform: Nintendo 64"), with its games.
    Section(String, Vec<GameId>),
}

pub(crate) struct BulkDlg {
    /// (label, games) choices; `scope` indexes it.
    scopes: Vec<(String, Vec<GameId>)>,
    scope: usize,
    slots: [bool; 5],
    only_missing: bool,
    /// (source name, enabled).
    sources: Vec<(String, bool)>,
}

impl DesktopUi {
    pub(in crate::ui::desktop) fn bulk_dlg(&self, st: &AppState, initial: BulkScope) -> BulkDlg {
        let mut scopes: Vec<(String, Vec<GameId>)> = vec![];
        let mut pick = 0;
        let game = match &initial {
            BulkScope::Game(id) => Some(*id),
            _ => self.selected,
        };
        if let Some(g) = game.and_then(|id| st.library.get(id)) {
            if matches!(initial, BulkScope::Game(_)) {
                pick = scopes.len();
            }
            scopes.push((format!("This game ({})", g.display_name()), vec![g.id]));
        }
        if self.multi.len() > 1 {
            if initial == BulkScope::Selection {
                pick = scopes.len();
            }
            let ids: Vec<GameId> =
                self.view.iter().map(|&i| st.library.games[i].id).filter(|id| self.multi.contains(id)).collect();
            scopes.push((format!("Selected games ({})", ids.len()), ids));
        }
        if let BulkScope::Section(label, ids) = &initial {
            pick = scopes.len();
            scopes.push((format!("{label} ({})", ids.len()), ids.clone()));
        }
        if initial == BulkScope::View || !self.view.is_empty() {
            if initial == BulkScope::View {
                pick = scopes.len();
            }
            let ids: Vec<GameId> = self.view.iter().map(|&i| st.library.games[i].id).collect();
            let label = if self.view.len() == self.counts.all {
                format!("All games ({})", ids.len())
            } else {
                format!("Games in the current view ({})", ids.len())
            };
            scopes.push((label, ids));
        }
        BulkDlg { scopes, scope: pick, slots: [true; 5], only_missing: true, sources: vec![] }
    }
}

pub(crate) enum BulkOutcome {
    Keep,
    Close,
    OpenSettings,
}

pub(crate) fn bulk_dialog(
    ctx: &egui::Context,
    st: &mut AppState,
    d: &mut BulkDlg,
    pal: &Pal,
    fam: &egui::FontFamily,
) -> BulkOutcome {
    // Sources switched off in Settings → Metadata aren't offered here. Each game uses its own
    // list (PC games / emulated games): offer what the chosen games' lists have switched on.
    let scope_ids: &[GameId] = d.scopes.get(d.scope).map_or(&[], |(_, v)| v.as_slice());
    let infos = bulk_infos(st, scope_ids);
    // Keep the user's choices; add sources that appeared.
    for s in &infos {
        if !d.sources.iter().any(|(n, _)| *n == s.name) {
            d.sources.push((s.name.clone(), s.available));
        }
    }
    let running = st.art_state.bulk.as_ref().is_some_and(|b| b.running);
    let mut out = BulkOutcome::Keep;
    let (_, close) = modal(ctx, "ls_bulk_art_dlg", pal, 520.0, |ui| {
        title(
            ui,
            "Download artwork",
            Some("Fill in or replace artwork from online sources, in the background."),
            pal,
            fam,
        );
        section(ui, "Games", pal);
        for (i, (label, _)) in d.scopes.iter().enumerate() {
            ui.radio_value(&mut d.scope, i, label.as_str());
        }
        if d.scopes.is_empty() {
            ui.label(RichText::new("No games to download artwork for.").color(pal.dim));
        }

        section(ui, "Artwork", pal);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 14.0;
            for (i, slot) in ArtSlot::ALL.iter().enumerate() {
                ui.checkbox(&mut d.slots[i], slot.label());
            }
        });

        section(ui, "Existing artwork", pal);
        ui.radio_value(&mut d.only_missing, true, "Only fill in missing artwork");
        ui.radio_value(&mut d.only_missing, false, "Replace existing artwork (your own images too)");

        section(ui, "Sources", pal);
        let mut settings = false;
        if infos.is_empty() {
            ui.label(RichText::new("No online artwork sources are available yet.").color(pal.dim));
        }
        // Ready sources as one wrapping row of checkboxes (in priority order); the rest in one
        // line with why.
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(14.0, 6.0);
            for s in infos.iter().filter(|s| s.available) {
                if let Some(entry) = d.sources.iter_mut().find(|(n, _)| *n == s.name) {
                    ui.checkbox(&mut entry.1, s.name.as_str()).on_hover_text(s.kind.blurb());
                }
            }
        });
        let rejected = |s: &SourceInfo| s.reason.as_deref().is_some_and(|r| r.contains("rejected"));
        let missing: Vec<&str> =
            infos.iter().filter(|s| !s.available && !rejected(s)).map(|s| s.name.as_str()).collect();
        let refused: Vec<&str> =
            infos.iter().filter(|s| !s.available && rejected(s)).map(|s| s.name.as_str()).collect();
        if !missing.is_empty() || !refused.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let mut text = vec![];
                if !missing.is_empty() {
                    let verb = if missing.len() == 1 { "needs" } else { "need" };
                    text.push(format!("{} {verb} your own key.", missing.join(", ")));
                }
                if !refused.is_empty() {
                    text.push(format!("{} rejected the key.", refused.join(", ")));
                }
                ui.label(RichText::new(text.join(" ")).size(12.0).color(pal.dim));
                if ui.link(RichText::new("Set up…").size(12.0)).clicked() {
                    settings = true;
                }
            });
        }
        if settings {
            out = BulkOutcome::OpenSettings;
        }

        // Summary.
        let ids: &[GameId] = d.scopes.get(d.scope).map_or(&[], |(_, v)| v.as_slice());
        let slots: Vec<ArtSlot> = ArtSlot::ALL.iter().zip(d.slots).filter(|(_, on)| *on).map(|(s, _)| *s).collect();
        let needing = if d.only_missing {
            ids.iter()
                .filter_map(|id| st.library.get(*id))
                .filter(|g| slots.iter().any(|s| s.get(&g.art).is_none()))
                .count()
        } else {
            ids.len()
        };
        let chosen: Vec<String> = infos
            .iter()
            .filter(|s| s.available)
            .filter(|s| d.sources.iter().any(|(n, on)| *n == s.name && *on))
            .map(|s| s.name.clone())
            .collect();
        ui.add_space(12.0);
        let summary = if d.only_missing {
            format!(
                "{needing} of {} game{} {} missing artwork.",
                ids.len(),
                if ids.len() == 1 { "" } else { "s" },
                if ids.len() == 1 { "is" } else { "are" }
            )
        } else {
            format!("Artwork of {} game{} will be replaced.", ids.len(), if ids.len() == 1 { "" } else { "s" })
        };
        ui.label(RichText::new(summary).color(pal.text));
        hint_text(
            ui,
            "Games without a confident match (same name, or the game's Steam / GOG id) are skipped. Servers are \
             asked politely (about two requests a second), so large libraries take a while; \
             progress shows in the status bar.",
            pal,
        );
        if running {
            ui.label(RichText::new("An artwork download is already running.").size(12.5).color(pal.danger));
        }
        let ok = !running && needing > 0 && !slots.is_empty() && !chosen.is_empty();
        let f = footer(ui, pal, Buttons::primary("Download", ok).cancel("Cancel"), |_| {});
        if f.primary {
            // All available sources ticked: let the backend use its own order (empty).
            let all = chosen.len() == infos.iter().filter(|s| s.available).count();
            let opts = BulkOptions {
                slots: slots.clone(),
                only_missing: d.only_missing,
                sources: if all { vec![] } else { chosen.clone() },
            };
            st.bulk_art(ids.to_vec(), opts);
        }
        if f.primary || f.cancel {
            out = BulkOutcome::Close;
        }
    });
    if close && matches!(out, BulkOutcome::Keep) {
        out = BulkOutcome::Close;
    }
    out
}

/// The switched-on sources of the lists the games use: PC games' list first, then what only
/// the emulated games' list adds (in its order).
fn bulk_infos(st: &AppState, ids: &[GameId]) -> Vec<SourceInfo> {
    use crate::art::ArtList;
    let mut lists: Vec<ArtList> = vec![];
    for g in ids.iter().filter_map(|id| st.library.get(*id)) {
        let l = crate::art::sources::art_list(g);
        if !lists.contains(&l) {
            lists.push(l);
            if lists.len() == 2 {
                break;
            }
        }
    }
    if lists.is_empty() {
        lists.push(ArtList::Pc);
    }
    lists.sort_by_key(|l| *l != ArtList::Pc);
    let mut out: Vec<SourceInfo> = vec![];
    for l in lists {
        for s in st.art.sources_for(l).into_iter().filter(|s| s.enabled) {
            if !out.iter().any(|o| o.kind == s.kind) {
                out.push(s);
            }
        }
    }
    out
}

pub(super) fn section(ui: &mut Ui, text: &str, pal: &Pal) {
    ui.add_space(10.0);
    ui.label(RichText::new(text).size(13.0).strong().color(pal.text));
    ui.add_space(2.0);
}

// ------------------------------------------------------------------------------- status bar

impl DesktopUi {
    /// Bulk download progress (spinner, n / total, current game, Cancel) or its result.
    pub(in crate::ui::desktop) fn bulk_status(&mut self, ui: &mut Ui, st: &mut AppState) {
        let pal = self.pal;
        let Some(b) = &st.art_state.bulk else {
            self.bulk_done_at = None;
            return;
        };
        if b.running {
            self.bulk_done_at = None;
            lazy_spinner(ui, 12.0, pal.accent);
            let frac = if b.total > 0 { b.done as f32 / b.total as f32 } else { 0.0 };
            let (r, _) = ui.allocate_exact_size(vec2(70.0, 5.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(r, 2.5, with_alpha(pal.dim, 60));
            p.rect_filled(
                Rect::from_min_size(r.min, vec2(r.width() * frac.clamp(0.0, 1.0), r.height())),
                2.5,
                pal.accent,
            );
            let wait = st.art.limit_wait();
            let mut text = format!("Downloading artwork {}/{}", b.done, b.total);
            if let Some(w) = &wait {
                // Paused for an API limit: say why and when it goes on.
                text = format!("Artwork download paused {}/{} · {w}", b.done, b.total);
                ui.ctx().request_repaint_after(Duration::from_secs(1));
            } else if !b.current.is_empty() {
                text.push_str(" · ");
                text.push_str(&b.current);
            }
            let max_w = (ui.available_width() - 260.0).clamp(120.0, 420.0);
            let g = truncated(ui.painter(), &text, egui::FontId::proportional(12.0), pal.dim, max_w);
            let (r, resp) = ui.allocate_exact_size(g.size(), Sense::hover());
            ui.painter().galley(r.min, g, pal.dim);
            resp.on_hover_text(text);
            if ui
                .add(egui::Button::new(RichText::new("Cancel").size(12.0)).small())
                .on_hover_text("Stop downloading artwork")
                .clicked()
            {
                st.cancel_bulk_art();
            }
            return;
        }
        // Finished: show the result for a while.
        let Some((changed, failed, _)) = b.result else {
            return;
        };
        let at = *self.bulk_done_at.get_or_insert_with(Instant::now);
        const SHOW: Duration = Duration::from_secs(20);
        if at.elapsed() >= SHOW {
            st.art_state.bulk = None;
            self.bulk_done_at = None;
            return;
        }
        ui.ctx().request_repaint_after(SHOW - at.elapsed());
        let text = crate::art_app::bulk_summary(b);
        let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
        let bad = changed == 0 && (failed > 0 || b.error.is_some());
        let (icon, color) = if bad {
            (Icon::Close, pal.danger)
        } else if changed > 0 {
            (Icon::Check, pal.accent)
        } else {
            (Icon::Check, pal.dim)
        };
        paint_icon(ui.painter(), r, icon, color);
        let max_w = (ui.available_width() - 260.0).clamp(120.0, 520.0);
        let g = truncated(ui.painter(), &text, egui::FontId::proportional(12.0), pal.dim, max_w);
        let (r, resp) = ui.allocate_exact_size(g.size(), Sense::hover());
        ui.painter().galley(r.min, g, pal.dim);
        if resp.hovered() {
            let mut tip = text.clone();
            if !b.not_found.is_empty() {
                tip.push_str("\n\nNo confident match:\n");
                for n in b.not_found.iter().take(25) {
                    tip.push_str("  • ");
                    tip.push_str(n);
                    tip.push('\n');
                }
                if b.not_found.len() > 25 {
                    tip.push_str(&format!("  … and {} more", b.not_found.len() - 25));
                }
            }
            resp.on_hover_text(tip);
        }
        if crate::ui::widgets::icon_button(ui, Icon::Close, false, 16.0, "Dismiss").clicked() {
            st.art_state.bulk = None;
            self.bulk_done_at = None;
        }
    }
}
