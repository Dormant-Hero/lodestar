//! Download metadata… in Edit Game Details: looks up the name (and platform) typed in the
//! dialog, shows what it found in the fields (highlighted, nothing saved until Save; Cancel
//! discards), and lets the user pick when the name matches several games.

use super::*;
use crate::metadata::{Candidate, FetchResult, Pick};
use crate::ui::desktop::dialogs::plain_button;

/// Where the dialog's lookup is.
#[derive(Clone, Debug, Default)]
pub(crate) enum LookupState {
    #[default]
    Idle,
    /// Waiting for the answer to lookup `token` of `name`.
    Waiting { token: u64, name: String },
    /// Several games match `name`: the user picks one. `partial` is what the other sources found.
    Picking { name: String, partial: Box<FetchResult> },
    /// The answer is in the fields: `changed` fields got a new value (stored on Save).
    Shown { name: String, changed: usize },
    /// Nothing was found for `name` (the fields are left as they were).
    NotFound { name: String },
    /// The lookup couldn't run (no source set up, offline …), in plain words.
    Failed(String),
}

/// The lookup found facts about a game (not only Steam app facts such as ProtonDB).
fn found_anything(m: &Metadata) -> bool {
    m.description.is_some()
        || m.developer.is_some()
        || m.publisher.is_some()
        || !m.genres.is_empty()
        || m.release_year.is_some()
        || m.release_date.is_some()
        || !m.features.is_empty()
        || !m.themes.is_empty()
        || m.critic_score.is_some()
        || m.community_score.is_some()
        || m.series.is_some()
}

impl EditDlg {
    /// The game as typed in the dialog (what a lookup asks about): the typed name, platform and
    /// hand edits over the library's game (`game`; None for a game not added yet).
    pub(super) fn typed_game(&self, game: Option<&Game>) -> Game {
        let name = self.name.text.trim().to_string();
        let mut g = game.cloned().unwrap_or_else(|| Game::blank(&name));
        g.user_name = Some(name);
        g.edits = self.build_edits(&g.edits);
        if let Some(p) = g.edits.platform.clone().filter(|p| !p.trim().is_empty()) {
            g.platform = Some(p);
        }
        g
    }

    pub(crate) fn lookup_busy(&self) -> bool {
        matches!(self.lookup, LookupState::Waiting { .. })
    }

    /// Download metadata… (or a candidate picked): ask with what is typed now.
    pub(crate) fn start_lookup(&mut self, st: &mut AppState, game: Option<&Game>, pick: Option<Pick>) {
        let g = self.typed_game(game);
        let name = g.display_name().to_string();
        self.lookup = match st.lookup_metadata(&g, pick) {
            Ok(token) => LookupState::Waiting { token, name },
            Err(e) => LookupState::Failed(e),
        };
    }

    /// Pick up the answer once it is there.
    pub(super) fn poll_lookup(&mut self, st: &mut AppState, game: Option<&Game>) {
        let LookupState::Waiting { token, name } = &self.lookup else { return };
        let Some(r) = st.take_lookup(*token) else { return };
        let name = name.clone();
        if let Some(e) = &r.error {
            self.lookup = LookupState::Failed(e.clone());
        } else if !r.candidates.is_empty() {
            self.lookup = LookupState::Picking { name, partial: Box::new(r) };
        } else {
            self.apply_lookup(st, game, r, name);
        }
    }

    /// Show an answer in the fields (see `Field::take_lookup`); stored on Save.
    pub(super) fn apply_lookup(&mut self, st: &AppState, game: Option<&Game>, r: FetchResult, name: String) {
        if !found_anything(&r.meta) {
            // A miss never wipes what the game has.
            self.lookup = LookupState::NotFound { name };
            return;
        }
        let Some(preview) = self.preview(st, game, &r) else {
            self.lookup = LookupState::Failed("This game is no longer in the library".into());
            return;
        };
        let changed = self.take_preview(&preview, st.library.generation);
        self.found = Some(r);
        self.tab = Tab::General;
        self.lookup = LookupState::Shown { name, changed };
    }

    /// Fill the fields from `preview` (the game with the answer stored); how many changed.
    pub(super) fn take_preview(&mut self, preview: &Game, lib_gen: u64) -> usize {
        let fresh = EditDlg::from_game(preview, lib_gen);
        let mut changed = 0;
        for (f, n) in self.meta_fields().into_iter().zip(fresh.meta_fields_ref()) {
            changed += usize::from(f.take_lookup(n));
        }
        self.controller_auto = fresh.controller_auto;
        changed
    }

    /// The library's game with answer `r` stored (a game not added yet: just the answer).
    fn preview(&self, st: &AppState, game: Option<&Game>, r: &FetchResult) -> Option<Game> {
        match game {
            Some(g) => st.preview_lookup(g.id, r),
            None => {
                let mut g = Game::blank(self.name.text.trim());
                g.meta = r.meta.clone();
                g.meta.looked_up = false;
                Some(g.with_edits_applied())
            }
        }
    }

    /// The game the dialog's fields follow: the library's, with a shown answer stored.
    pub(super) fn followed_game(&self, st: &AppState, g: &Game) -> Option<Game> {
        match &self.found {
            Some(r) => st.preview_lookup(g.id, r),
            None => Some(g.clone()),
        }
    }

    /// The fields a lookup fills, in one order (`meta_fields_ref` is the same list).
    fn meta_fields(&mut self) -> [&mut Field; 10] {
        [
            &mut self.genres,
            &mut self.developers,
            &mut self.publishers,
            &mut self.features,
            &mut self.release_date,
            &mut self.release_year,
            &mut self.series,
            &mut self.critic_score,
            &mut self.community_score,
            &mut self.description,
        ]
    }

    fn meta_fields_ref(&self) -> [&Field; 10] {
        [
            &self.genres,
            &self.developers,
            &self.publishers,
            &self.features,
            &self.release_date,
            &self.release_year,
            &self.series,
            &self.critic_score,
            &self.community_score,
            &self.description,
        ]
    }

    /// The footer's status next to Download metadata… (text, is it a problem, offer Settings).
    pub(super) fn lookup_status(&self) -> Option<(String, bool, bool)> {
        match &self.lookup {
            LookupState::Idle | LookupState::Picking { .. } => None,
            LookupState::Waiting { name, .. } => Some((format!("Looking up “{name}”…"), false, false)),
            LookupState::Shown { name, changed: 0 } => Some((format!("“{name}”: nothing new found"), false, false)),
            LookupState::Shown { name, changed } => Some((
                format!(
                    "“{name}”: {changed} field{} updated — Save keeps {}",
                    if *changed == 1 { "" } else { "s" },
                    if *changed == 1 { "it" } else { "them" }
                ),
                false,
                false,
            )),
            LookupState::NotFound { name } => {
                Some((format!("Nothing found for “{name}”: try another name"), true, false))
            }
            LookupState::Failed(e) => Some((e.clone(), true, e.contains("Settings"))),
        }
    }
}

/// What the user chose in the "Which game is it?" list.
pub(super) enum PickOutcome {
    Keep,
    Pick(Pick),
    /// Use what the other sources found (or nothing).
    NoneOfThese,
    Cancel,
}

/// "Which game is it?": the games an ambiguous name matched (name, year, platforms).
pub(super) fn pick_dialog(
    ctx: &egui::Context,
    name: &str,
    c: &[Candidate],
    pal: &Pal,
    fam: &egui::FontFamily,
) -> PickOutcome {
    let mut out = PickOutcome::Keep;
    let (_, close) = modal(ctx, "ls_edit_lookup_pick", pal, 460.0, |ui| {
        super::super::dialogs::title(
            ui,
            "Which game is it?",
            Some(&format!("Several games match “{name}”. Pick the right one.")),
            pal,
            fam,
        );
        ui.add_space(8.0);
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for cand in c {
                let mut sub: Vec<String> = vec![];
                if let Some(y) = cand.year {
                    sub.push(y.to_string());
                }
                if !cand.platforms.is_empty() {
                    sub.push(cand.platforms.clone());
                }
                sub.push(cand.source.to_string());
                let text = egui::text::LayoutJob::default();
                let mut job = text;
                job.append(
                    &cand.name,
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::proportional(14.0),
                        color: pal.text,
                        ..Default::default()
                    },
                );
                job.append(
                    &format!("\n{}", sub.join(" · ")),
                    0.0,
                    egui::TextFormat {
                        font_id: egui::FontId::proportional(12.0),
                        color: pal.dim,
                        ..Default::default()
                    },
                );
                job.wrap.max_width = ui.available_width() - 20.0;
                let b = egui::Button::new(job).min_size(vec2(ui.available_width(), 44.0)).wrap();
                if ui.add(b).clicked() {
                    out = PickOutcome::Pick(cand.pick.clone());
                }
            }
        });
        let f = footer(ui, pal, Buttons::default().cancel("Cancel"), |ui| {
            if plain_button(ui, "None of these").on_hover_text("Keep only what the other sources found").clicked() {
                out = PickOutcome::NoneOfThese;
            }
        });
        if f.cancel {
            out = PickOutcome::Cancel;
        }
    });
    if close && matches!(out, PickOutcome::Keep) {
        out = PickOutcome::Cancel;
    }
    out
}
