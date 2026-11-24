//! Edit Game Details (F3, the pencil on the game page, double-click the title, right-click →
//! Edit…), also used for Add Game → Manually… on a new game. Tabs: General, Advanced, Media,
//! Links, Installation, Actions, Scripts.
//!
//! Every General field shows the source's value until the user changes it; changed fields are
//! saved as hand edits (`UserEdits`, applied over the source on load, so rescans never overwrite
//! them) and show a small "reset to source" arrow. Nothing is written until Save, except
//! artwork (the Media tab is the artwork editor, which applies right away).

use super::add::{Pick, PickKind};
use super::artwork::{self, ArtDlg, ArtOutcome};
use super::details::format_size;
use super::dialogs::{hint_text, modal, plain_button, primary_button};
use super::{Pal, platform_label};
use crate::app::AppState;
use crate::art::ArtSlot;
use crate::edit_app::NewGame;
use crate::model::*;
use crate::ui::widgets::{Icon, icon_button, lazy_spinner, mix, paint_icon, with_alpha};
use egui::{RichText, Sense, Ui, pos2, vec2};
use std::path::PathBuf;

// The form widgets and the tabs, in sibling files (paths relative to ui/desktop/).
mod form;

mod tabs;

mod actions;

mod lookup;

pub(crate) use actions::*;
pub(crate) use form::*;
use tabs::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Tab {
    #[default]
    General,
    Advanced,
    Media,
    Links,
    Installation,
    Actions,
    Scripts,
}

impl Tab {
    pub const ALL: [Tab; 7] =
        [Tab::General, Tab::Advanced, Tab::Media, Tab::Links, Tab::Installation, Tab::Actions, Tab::Scripts];

    pub fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Advanced => "Advanced",
            Tab::Media => "Media",
            Tab::Links => "Links",
            Tab::Installation => "Installation",
            Tab::Actions => "Actions",
            Tab::Scripts => "Scripts",
        }
    }

    /// From a testing-hook key ("media", "links" …); General for anything else.
    pub fn from_key(k: &str) -> Tab {
        Tab::ALL.into_iter().find(|t| t.label().eq_ignore_ascii_case(k)).unwrap_or_default()
    }
}

/// What a file / folder picker started from this dialog fills in.
#[derive(Clone, Copy, Debug, PartialEq)]
enum PickFor {
    InstallDir,
    Manual,
    ActionPath(usize),
    ActionCwd(usize),
    NewArt(ArtSlot),
}

pub(crate) struct EditDlg {
    /// None: a new game (Add Game → Manually… / Pick a program → Edit details…).
    pub id: Option<GameId>,
    pub tab: Tab,
    title_name: String,
    source: Source,
    lib_gen: u64,
    // General
    name: Field,
    sort_name: String,
    platform: Field,
    genres: Field,
    developers: Field,
    publishers: Field,
    features: Field,
    tags: String,
    release_date: Field,
    release_year: Field,
    series: Field,
    age_rating: Field,
    region: Field,
    completion: Option<CompletionStatus>,
    controller: Option<ControllerSupport>,
    /// What "Automatic" means for controller support ("Automatic (Steam says: Full)").
    controller_auto: String,
    user_score: Field,
    critic_score: Field,
    community_score: Field,
    description: Field,
    // Advanced
    last_played: String,
    playtime: String,
    play_count: String,
    added: String,
    version: Field,
    manual: Field,
    hidden: bool,
    favorite: bool,
    notes: String,
    // Links
    links: Vec<GameLink>,
    auto_links: Vec<GameLink>,
    hide_auto_links: bool,
    // Installation
    install_dir: Field,
    installed: BoolField,
    /// Emulated games: the RetroAchievements game id ("" = matched automatically).
    ra_game_id: String,
    /// Emulated games: emulator / profile choice (owned by `edit_emulation.rs`).
    pub(super) emu: super::edit_emulation::EmuEdit,
    /// Cloud games: Opens with / Full screen (owned by `add_cloud.rs`).
    pub(super) cloud: Option<super::add_cloud::CloudEdit>,
    // Actions / scripts
    actions: Vec<GameAction>,
    /// How the source starts the game (shown above the actions).
    source_launch: Option<String>,
    scripts: GameScripts,
    // Media
    art: Option<ArtDlg>,
    /// New games: artwork files picked before the game exists.
    new_art: Vec<(ArtSlot, String)>,
    /// New games: the launch the add flow found (Pick a program …).
    template: NewGame,
    /// The values this dialog started with (to save only what changed outside `UserEdits`).
    initial: Initial,
    picker: Option<(PickFor, Pick)>,
    /// Per action: the action it was computed for and its "Will run" line (or the problem).
    previews: Vec<Option<(GameAction, Result<String, String>)>>,
    /// `emu` as the previews were made with.
    emu_seen: super::edit_emulation::EmuEdit,
    /// New game only: the library game with the same name (or program), for the footer; worked
    /// out again when the name changes.
    dup: Option<(String, Option<String>)>,
    error: Option<String>,
    focus_name: bool,
    /// Download metadata… (edit/lookup.rs).
    lookup: lookup::LookupState,
    /// The lookup answer shown in the fields: stored on Save, dropped by Cancel.
    found: Option<crate::metadata::FetchResult>,
}

/// Library values the dialog only writes back when the user changed them.
#[derive(Clone, Debug, Default, PartialEq)]
struct Initial {
    tags: Vec<String>,
    favorite: bool,
    hidden: bool,
    controller: Option<ControllerSupport>,
    last_played: String,
    playtime: String,
    play_count: String,
    added: String,
    name: Field,
}

/// What the dialog wants after this frame.
pub(crate) enum EditOutcome {
    Keep,
    Close,
    /// A new game was added: select it.
    Added(GameId),
    /// Keep the dialog and open Settings → Metadata on top (artwork API keys).
    OpenSettings,
}

/// "1234" → Some(1234), "" → None (matched automatically); "12a" / "0" → Err. A link to the
/// game's page ("retroachievements.org/game/1234") counts as its id.
pub(super) fn parse_ra_id(s: &str) -> Result<Option<u32>, ()> {
    let s = s.trim().trim_end_matches('/');
    if s.is_empty() {
        return Ok(None);
    }
    let s = s.rsplit_once("/game/").map_or(s, |(_, id)| id);
    match s.parse::<u32>() {
        Ok(n) if n > 0 => Ok(Some(n)),
        _ => Err(()),
    }
}

fn opt_num<T: ToString>(v: Option<T>) -> Option<String> {
    v.map(|v| v.to_string())
}

/// A year / score edit as text: a cleared one (`CLEARED_YEAR` / `CLEARED_SCORE`) is empty.
fn edit_num<T: ToString + PartialEq>(v: Option<T>, cleared: T) -> Option<String> {
    v.map(|v| if v == cleared { String::new() } else { v.to_string() })
}

impl EditDlg {
    pub fn for_game(st: &AppState, g: &Game) -> EditDlg {
        EditDlg::from_game(g, st.library.generation)
    }

    fn from_game(g: &Game, lib_gen: u64) -> EditDlg {
        let e = &g.edits;
        let sm = &g.source_meta;
        let src = |v: &Option<String>| Some(v.clone().unwrap_or_default());
        let tags = g.tags.join(", ");
        let last_played = g.last_played.map(format_date).unwrap_or_default();
        let playtime = format_duration(g.playtime_secs);
        let play_count = g.play_count.to_string();
        let added = if g.added > 0 { format_date(g.added) } else { String::new() };
        let name = Field {
            text: g.display_name().to_string(),
            source: Some(g.name.clone()),
            over: g.user_name.is_some(),
            updated: false,
        };
        EditDlg {
            id: Some(g.id),
            tab: Tab::General,
            title_name: g.display_name().to_string(),
            source: g.source,
            lib_gen,
            name: name.clone(),
            sort_name: e.sort_name.clone().unwrap_or_default(),
            platform: Field::new(
                e.platform.clone(),
                if e.platform.is_some() {
                    None
                } else {
                    Some(g.platform.as_deref().map(platform_label).unwrap_or_default().to_string())
                },
            ),
            genres: Field::new(e.genres.as_ref().map(|v| v.join(", ")), Some(sm.genres.join(", "))),
            developers: Field::new(e.developer.clone(), src(&sm.developer)),
            publishers: Field::new(e.publisher.clone(), src(&sm.publisher)),
            features: Field::new(e.features.as_ref().map(|v| v.join(", ")), Some(sm.features.join(", "))),
            tags: tags.clone(),
            release_date: Field::new(e.release_date.clone(), src(&sm.release_date)),
            release_year: Field::new(
                edit_num(e.release_year, CLEARED_YEAR),
                Some(opt_num(sm.release_year).unwrap_or_default()),
            ),
            series: Field::new(e.series.clone(), src(&sm.series)),
            age_rating: Field::own(e.age_rating.clone()),
            region: Field::own(e.region.clone()),
            completion: e.completion,
            controller: g.controller_override,
            controller_auto: super::controller_auto_label(g),
            user_score: Field::own(opt_num(e.user_score)),
            critic_score: Field::new(
                edit_num(e.critic_score, CLEARED_SCORE),
                Some(opt_num(sm.critic_score).unwrap_or_default()),
            ),
            community_score: Field::new(
                edit_num(e.community_score, CLEARED_SCORE),
                Some(opt_num(sm.community_score).unwrap_or_default()),
            ),
            description: Field::new(e.description.clone(), src(&sm.description)),
            last_played: last_played.clone(),
            playtime: playtime.clone(),
            play_count: play_count.clone(),
            added: added.clone(),
            version: Field::own(e.version.clone()),
            manual: Field::own(e.manual.clone()),
            hidden: g.hidden,
            favorite: g.favorite,
            notes: e.notes.clone(),
            links: e.links.clone(),
            auto_links: g.auto_links(),
            hide_auto_links: e.hide_auto_links,
            install_dir: Field::new(
                e.install_dir.as_ref().map(|p| p.display().to_string()),
                if e.install_dir.is_some() {
                    None
                } else {
                    Some(g.install_dir.as_ref().map(|p| p.display().to_string()).unwrap_or_default())
                },
            ),
            installed: BoolField {
                val: g.installed,
                source: e.installed.is_none().then_some(g.installed),
                over: e.installed.is_some(),
            },
            ra_game_id: opt_num(e.ra_game_id).unwrap_or_default(),
            emu: super::edit_emulation::EmuEdit::from_game(g),
            cloud: super::add_cloud::CloudEdit::of(g),
            actions: e.actions.clone(),
            source_launch: describe_launch(g),
            scripts: e.scripts.clone(),
            art: None,
            new_art: vec![],
            template: NewGame::default(),
            initial: Initial {
                tags: g.tags.clone(),
                favorite: g.favorite,
                hidden: g.hidden,
                controller: g.controller_override,
                last_played,
                playtime,
                play_count,
                added,
                name,
            },
            picker: None,
            previews: vec![],
            emu_seen: Default::default(),
            dup: None,
            error: None,
            focus_name: false,
            lookup: Default::default(),
            found: None,
        }
    }

    /// Edit Game Details on a game that doesn't exist yet; Save adds it.
    pub fn new_game(st: &AppState, template: NewGame) -> EditDlg {
        let mut g = Game::blank(&template.name);
        g.platform = template.platform.clone();
        g.install_dir = template.install_dir.clone();
        g.launch = template.launch.clone();
        g.edits = template.edits.clone();
        g.added = 0;
        let mut d = EditDlg::for_game(st, &g);
        d.id = None;
        d.title_name = if template.name.trim().is_empty() { "New game".into() } else { template.name.clone() };
        // Everything typed is the user's own: there is no source to fall back to.
        d.name = Field::own(Some(template.name.clone()).filter(|n| !n.is_empty()));
        d.initial.name = d.name.clone();
        d.platform = Field::own(template.platform.as_deref().map(|p| platform_label(p).into()));
        d.install_dir = Field::own(template.install_dir.as_ref().map(|p| p.display().to_string()));
        d.installed = BoolField { val: true, source: Some(true), over: false };
        d.last_played.clear();
        d.playtime.clear();
        d.play_count.clear();
        d.initial.last_played.clear();
        d.initial.playtime.clear();
        d.initial.play_count.clear();
        d.controller_auto = "Automatic (not known)".into();
        d.source_launch = template.launch.as_ref().map(describe_spec);
        for slot in ArtSlot::ALL {
            if let Some(p) = slot.get(&template.art) {
                d.new_art.push((slot, p.display().to_string()));
            }
        }
        d.template = template;
        d.focus_name = true;
        d
    }

    pub fn set_tab(&mut self, st: &AppState, tab: Tab) {
        self.tab = tab;
        if tab == Tab::Media
            && self.art.is_none()
            && let Some(g) = self.id.and_then(|id| st.library.get(id))
        {
            self.art = Some(ArtDlg::new(g, ArtSlot::Cover));
        }
    }

    pub fn set_art_slot(&mut self, slot: ArtSlot) {
        if let Some(a) = &mut self.art {
            a.slot = slot;
        }
    }

    /// Fields that follow the source pick up its new values (Download metadata, a rescan).
    fn follow_source(&mut self, g: &Game, st: &AppState) {
        // A lookup answer shown (not saved yet) stays in the fields.
        let Some(g) = self.followed_game(st, g) else { return };
        let fresh = EditDlg::from_game(&g, st.library.generation);
        self.name.follow(&fresh.name);
        self.platform.follow(&fresh.platform);
        self.genres.follow(&fresh.genres);
        self.developers.follow(&fresh.developers);
        self.publishers.follow(&fresh.publishers);
        self.release_year.follow(&fresh.release_year);
        self.release_date.follow(&fresh.release_date);
        self.features.follow(&fresh.features);
        self.series.follow(&fresh.series);
        self.critic_score.follow(&fresh.critic_score);
        self.community_score.follow(&fresh.community_score);
        self.description.follow(&fresh.description);
        self.install_dir.follow(&fresh.install_dir);
        if !self.installed.over {
            self.installed = fresh.installed;
        }
        self.controller_auto = fresh.controller_auto;
        self.auto_links = fresh.auto_links;
        self.lib_gen = st.library.generation;
    }

    /// Recompute the "Will run" lines of actions that changed since last time (resolving
    /// touches the disk a little, so not every frame).
    fn refresh_previews(&mut self, game: Option<&Game>) {
        self.previews.resize(self.actions.len(), None);
        let mut base: Option<Game> = None;
        let emulated = self.emu.emulated;
        // The emulator choice / switches being edited apply to the "Will run" lines too.
        if self.emu_seen != self.emu {
            self.emu_seen = self.emu.clone();
            self.previews.clear();
            self.previews.resize(self.actions.len(), None);
        }
        for (i, a) in self.actions.iter().enumerate() {
            // Emulated games: an emulator action, or a program action without a program (= the
            // game's emulator), has a command without a path.
            let uses_emulator = emulated && matches!(a.kind, ActionKind::Emulator | ActionKind::Command);
            if a.path.trim().is_empty() && !uses_emulator {
                self.previews[i] = None;
                continue;
            }
            if matches!(&self.previews[i], Some((done, _)) if done == a) {
                continue;
            }
            let g = base.get_or_insert_with(|| {
                let mut g = game.cloned().unwrap_or_else(|| Game::blank(&self.name.text));
                let dir = self.install_dir.text.trim();
                if !dir.is_empty() {
                    g.install_dir = Some(PathBuf::from(dir));
                }
                g.user_name = Some(self.name.text.trim().to_string());
                self.emu.apply_to(&mut g.edits);
                g
            });
            let r = crate::launch::preview_action(g, a).map(|r| r.display).map_err(|e| format!("{e:#}"));
            self.previews[i] = Some((a.clone(), r));
        }
    }

    /// A new game whose name (or program) is already in the library: "Name (Source)", or
    /// "Name (Source, hidden)". Cached per name.
    fn duplicate_hint(&mut self, st: &AppState) -> Option<String> {
        if self.id.is_some() {
            return None;
        }
        let name = self.name.text.trim();
        if self.dup.as_ref().is_some_and(|(n, _)| n == name) {
            return self.dup.as_ref().and_then(|(_, h)| h.clone());
        }
        let hint = crate::addgame::find_duplicates(&st.library, name, self.template.launch.as_ref(), None)
            .first()
            .and_then(|m| st.library.get(m.id))
            .map(|g| {
                let hidden = if g.hidden { ", hidden" } else { "" };
                format!("{} ({}{hidden})", g.display_name(), g.source.display())
            });
        self.dup = Some((name.to_string(), hint.clone()));
        hint
    }

    /// Check the fields that must parse; the first problem, for the footer.
    fn problem(&self) -> Option<String> {
        if self.name.text.trim().is_empty() {
            return Some("Give the game a name".into());
        }
        for (label, f) in [
            ("User score", &self.user_score),
            ("Critic score", &self.critic_score),
            ("Community score", &self.community_score),
        ] {
            if parse_score(&f.text).is_err() {
                return Some(format!("{label}: a number from 0 to 100"));
            }
        }
        let y = self.release_year.text.trim();
        if !y.is_empty() && y.parse::<i32>().is_err() {
            return Some("Release year: a year like 2023".into());
        }
        if parse_ra_id(&self.ra_game_id).is_err() {
            return Some("RetroAchievements game id: a number like 1234 (or empty)".into());
        }
        let rd = self.release_date.text.trim();
        if !rd.is_empty() && parse_date(rd).is_none() {
            return Some("Release date: YYYY-MM-DD".into());
        }
        for (label, v) in [("Last played", &self.last_played), ("Added", &self.added)] {
            if !v.trim().is_empty() && parse_date(v).is_none() {
                return Some(format!("{label}: YYYY-MM-DD"));
            }
        }
        if parse_duration(&self.playtime).is_none() {
            return Some("Play time: like 12h 30m".into());
        }
        if !self.play_count.trim().is_empty() && self.play_count.trim().parse::<u32>().is_err() {
            return Some("Play count: a whole number".into());
        }
        None
    }

    /// The hand edits as edited, on top of `base` (fields this dialog doesn't know are kept).
    pub(crate) fn build_edits(&self, base: &UserEdits) -> UserEdits {
        let mut e = base.clone();
        e.sort_name = Some(self.sort_name.trim().to_string()).filter(|s| !s.is_empty());
        e.platform = self.platform.value();
        // Cleared by the user = an empty list (not "use the source's").
        e.genres = self.genres.value().map(|v| split_list(&v));
        e.developer = self.developers.value();
        e.publisher = self.publishers.value();
        // Cleared by the user = an empty list (not "use the source's").
        e.features = self.features.value().map(|v| split_list(&v));
        e.release_date = self.release_date.value();
        // A full release date and no year of its own: the date's year.
        let date_year = || {
            let d = e.release_date.as_deref()?;
            parse_date(d)?;
            d.get(..4)?.parse().ok()
        };
        e.release_year = match self.release_year.value() {
            // Cleared by the user: stays cleared (unless they typed a full date).
            Some(y) if y.is_empty() => date_year().or(Some(CLEARED_YEAR)),
            Some(y) => y.parse().ok().filter(|y| *y != CLEARED_YEAR),
            None => date_year(),
        };
        e.series = self.series.value();
        e.age_rating = self.age_rating.value();
        e.region = self.region.value();
        e.completion = self.completion;
        e.user_score = parse_score(&self.user_score.text).ok().flatten();
        // Scores from IGDB stay the source's until the user types their own.
        // Cleared by the user: stays cleared.
        let own_score =
            |f: &Field| f.over.then(|| parse_score(&f.text).ok().flatten().or(Some(CLEARED_SCORE))).flatten();
        e.critic_score = own_score(&self.critic_score);
        e.community_score = own_score(&self.community_score);
        e.description = self.description.value();
        e.version = self.version.value();
        e.manual = self.manual.value();
        e.notes = self.notes.trim_end().to_string();
        // Game options (HDR, Gamescope …) are edited in Launch options only: kept as they are.
        e.install_dir = self.install_dir.value().map(PathBuf::from);
        e.installed = self.installed.over.then_some(self.installed.val);
        e.ra_game_id = parse_ra_id(&self.ra_game_id).ok().flatten();
        // None = automatic (per console / ROM folder / first emulator found); the game's switches.
        self.emu.apply_to(&mut e);
        e.links = self
            .links
            .iter()
            .filter(|l| !l.url.trim().is_empty())
            .map(|l| GameLink {
                name: if l.name.trim().is_empty() { link_name(&l.url) } else { l.name.trim().to_string() },
                url: l.url.trim().to_string(),
            })
            .collect();
        e.hide_auto_links = self.hide_auto_links;
        e.actions = self
            .actions
            .iter()
            .enumerate()
            .map(|(i, a)| GameAction {
                name: if a.name.trim().is_empty() { format!("Action {}", i + 1) } else { a.name.trim().to_string() },
                path: a.path.trim().to_string(),
                args: a.args.trim().to_string(),
                cwd: a.cwd.as_deref().map(str::trim).filter(|c| !c.is_empty()).map(str::to_string),
                ..a.clone()
            })
            .collect();
        e.scripts = self.scripts.clone();
        e
    }

    /// Write everything to the library. Ok(Some(id)) for a new game.
    fn save(&self, st: &mut AppState) -> Result<Option<GameId>, String> {
        if let Some(p) = self.problem() {
            return Err(p);
        }
        let (id, new) = match self.id {
            Some(id) => {
                if st.library.get(id).is_none() {
                    return Err("This game is no longer in the library".into());
                }
                // Download metadata…'s answer first: the edits below go over it.
                if let Some(r) = &self.found {
                    st.store_lookup(id, r);
                }
                let Some(g) = st.library.get(id) else {
                    return Err("This game is no longer in the library".into());
                };
                // Install size is calculated in the background: keep its latest value.
                let mut base = g.edits.clone();
                base.install_size = g.edits.install_size;
                let e = self.build_edits(&base);
                if e != g.edits {
                    st.set_edits(id, e);
                }
                if self.name != self.initial.name {
                    st.rename(id, self.name.value());
                }
                (id, false)
            }
            None => {
                let edits = self.build_edits(&self.template.edits);
                let mut art = self.template.art.clone();
                for (slot, p) in &self.new_art {
                    let p = p.trim();
                    if !p.is_empty() {
                        slot.set(&mut art, Some(PathBuf::from(p)));
                    }
                }
                let launch = self
                    .template
                    .launch
                    .clone()
                    .or_else(|| edits.actions.iter().find(|a| a.play).and_then(action_spec));
                let g = NewGame {
                    name: self.name.text.trim().to_string(),
                    launch,
                    art,
                    install_dir: edits.install_dir.clone(),
                    platform: edits.platform.clone(),
                    edits,
                };
                match st.add_game(g) {
                    Some(id) => {
                        if let Some(r) = &self.found {
                            st.store_lookup(id, r);
                        }
                        (id, true)
                    }
                    None => return Err("Couldn't add the game (see the notification)".into()),
                }
            }
        };
        let tags = split_list(&self.tags);
        if tags != self.initial.tags {
            st.set_tags(id, tags);
        }
        if self.favorite != self.initial.favorite && st.library.get(id).is_some_and(|g| g.favorite != self.favorite) {
            st.toggle_favorite(id);
        }
        if self.hidden != self.initial.hidden {
            st.set_hidden(id, self.hidden);
        }
        if self.controller != self.initial.controller {
            st.set_controller_override(id, self.controller);
        }
        let stats_changed = self.playtime != self.initial.playtime
            || self.play_count != self.initial.play_count
            || self.last_played != self.initial.last_played;
        if stats_changed {
            let secs = parse_duration(&self.playtime).unwrap_or(0);
            let count = self.play_count.trim().parse().unwrap_or(0);
            let last = parse_date(&self.last_played);
            st.set_play_stats(id, secs, count, last);
        }
        if self.added != self.initial.added
            && let Some(t) = parse_date(&self.added)
        {
            st.set_added(id, t);
        }
        if let Some(c) = self.cloud.as_ref().filter(|c| c.changed()) {
            st.set_cloud_entry(id, &c.entry);
        }
        Ok(new.then_some(id))
    }
}

/// The game's own launch in words, for the Actions tab ("Steam (app 1086940)").
fn describe_launch(g: &Game) -> Option<String> {
    // A game added by hand without a command has an empty one: say so instead of showing ''.
    g.launch
        .as_ref()
        .filter(|l| !matches!(l, LaunchSpec::Command { program, .. } if program.trim().is_empty()))
        .map(describe_spec)
}

fn describe_spec(l: &LaunchSpec) -> String {
    match l {
        LaunchSpec::Steam { appid } => format!("Through Steam (app {appid})"),
        LaunchSpec::Url(u) => u.clone(),
        LaunchSpec::Command { program, args, .. } => {
            let mut words = vec![crate::launch::shell_quote(program)];
            words.extend(args.iter().map(|a| crate::launch::shell_quote(a)));
            words.join(" ")
        }
    }
}

/// A launch spec for a new game's Play action, when it has no variables to expand.
fn action_spec(a: &GameAction) -> Option<LaunchSpec> {
    if a.path.contains('{') || a.args.contains('{') || a.path.trim().is_empty() {
        return None;
    }
    match a.kind {
        ActionKind::Url => Some(LaunchSpec::Url(a.path.trim().into())),
        ActionKind::File | ActionKind::Emulator => None,
        ActionKind::Command => {
            let args = crate::launch::split_command(&a.args).ok()?;
            Some(LaunchSpec::Command {
                program: a.path.trim().into(),
                args,
                cwd: a.cwd.as_deref().filter(|c| !c.trim().is_empty()).map(PathBuf::from),
                env: vec![],
            })
        }
    }
}

/// "https://www.protondb.com/app/1" → "protondb.com".
pub(crate) fn link_name(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    host.trim_start_matches("www.").to_string()
}

const VARS_HELP: &str = "You can use {InstallDir}, {Name}, {GameId}, {Prefix} (the Wine prefix) and \
     {Proton} (the Proton folder) in the path, arguments and working folder.";

impl super::DesktopUi {
    /// Draw Edit Game Details; returns what to do with it.
    pub(super) fn edit_dialog(&mut self, ctx: &egui::Context, st: &mut AppState, d: &mut EditDlg) -> EditOutcome {
        let pal = self.pal;
        let fam = self.fonts.bold.clone();
        // The game went away (removed / merged meanwhile).
        let game = match d.id {
            Some(id) => match st.library.get(id) {
                Some(g) => Some(g.clone()),
                None => return EditOutcome::Close,
            },
            None => None,
        };
        if let Some(g) = &game
            && d.lib_gen != st.library.generation
        {
            d.follow_source(g, st);
        }
        d.poll_lookup(st, game.as_ref());
        d.take_picked_file();

        let screen = ctx.content_rect();
        let width = (screen.width() - 64.0).clamp(560.0, 880.0);
        // In its own window the body takes all the height the window gives it.
        let body_h = if super::own_window::in_own_window(ctx) {
            (screen.height() - 170.0).max(240.0)
        } else {
            (screen.height() - 190.0).clamp(240.0, 600.0)
        };
        // Settings over the artwork editor (in the main window, or in its own window while this
        // one waits behind a veil).
        let covered = self.dlg.settings.is_some();
        let mut out = EditOutcome::Keep;
        let mut save = false;
        let (_, close) = modal(ctx, "ls_edit_dlg", &pal, width, |ui| {
            edit_header(ui, st, d, &pal, &fam);
            // The body has the same height on every tab, so the dialog doesn't jump.
            if d.tab == Tab::Media && d.id.is_some() {
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), body_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_height(body_h);
                        if let Some(a) = &mut d.art {
                            a.covered = covered;
                            if let ArtOutcome::OpenSettings = artwork::art_editor(ui, st, a, &pal, &fam, body_h) {
                                out = EditOutcome::OpenSettings;
                            }
                        }
                    },
                );
            } else {
                egui::ScrollArea::vertical()
                    .id_salt(("ls_edit_body", d.tab.label()))
                    .max_height(body_h)
                    .min_scrolled_height(body_h)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 6.0;
                        match d.tab {
                            Tab::General => general_tab(ui, st, d, &pal),
                            Tab::Advanced => advanced_tab(ui, d, &pal),
                            Tab::Media => new_media_tab(ui, d, &pal),
                            Tab::Links => links_tab(ui, st, d, &pal),
                            Tab::Installation => install_tab(ui, st, d, &pal),
                            Tab::Actions => {
                                d.refresh_previews(game.as_ref());
                                actions_tab(ui, st, d, &pal)
                            }
                            Tab::Scripts => scripts_tab(ui, st, d, &pal),
                        }
                    });
            }
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            let (footer_save, footer_out) = edit_footer(ui, st, d, game.as_ref(), &pal);
            save = footer_save;
            if let Some(o) = footer_out {
                out = o;
            }
        });
        let picking_now = self.lookup_pick(ctx, st, d, game.as_ref());
        if save {
            match d.save(st) {
                Ok(Some(id)) => return EditOutcome::Added(id),
                Ok(None) => return EditOutcome::Close,
                Err(e) => d.error = Some(e),
            }
        }
        // Esc / click outside: close, unless a file picker is open (its result would be lost).
        let picking = d.picker.is_some() || d.art.as_ref().is_some_and(|a| a.picking());
        if close && matches!(out, EditOutcome::Keep) && !picking && !picking_now {
            out = EditOutcome::Close;
        }
        out
    }

    /// "Which game is it?" while a lookup found several. True while it is open.
    fn lookup_pick(&self, ctx: &egui::Context, st: &mut AppState, d: &mut EditDlg, game: Option<&Game>) -> bool {
        let lookup::LookupState::Picking { name, partial } = &d.lookup else {
            return false;
        };
        let (name, partial) = (name.clone(), partial.clone());
        match lookup::pick_dialog(ctx, &name, &partial.candidates, &self.pal, &self.fonts.bold) {
            lookup::PickOutcome::Keep => {}
            lookup::PickOutcome::Pick(p) => d.start_lookup(st, game, Some(p)),
            lookup::PickOutcome::NoneOfThese => {
                let mut r = *partial;
                r.candidates.clear();
                d.apply_lookup(st, game, r, name);
            }
            lookup::PickOutcome::Cancel => d.lookup = lookup::LookupState::Idle,
        }
        true
    }
}

impl EditDlg {
    /// A file / folder picker's answer, into the field it was opened for.
    fn take_picked_file(&mut self) {
        let Some((what, pick)) = &self.picker else {
            return;
        };
        let Some(res) = pick.poll() else {
            return;
        };
        let what = *what;
        self.picker = None;
        let Some(p) = res.into_iter().next() else {
            return;
        };
        let text = p.display().to_string();
        match what {
            PickFor::InstallDir => {
                self.install_dir.text = text;
                self.install_dir.touched();
            }
            PickFor::Manual => {
                self.manual.text = text;
                self.manual.touched();
            }
            PickFor::ActionPath(i) => {
                if let Some(a) = self.actions.get_mut(i) {
                    a.path = text;
                }
            }
            PickFor::ActionCwd(i) => {
                if let Some(a) = self.actions.get_mut(i) {
                    a.cwd = Some(text);
                }
            }
            PickFor::NewArt(slot) => {
                self.new_art.retain(|(s, _)| *s != slot);
                self.new_art.push((slot, text));
            }
        }
    }
}

/// The dialog's title, the game's name and the tabs.
fn edit_header(ui: &mut Ui, st: &mut AppState, d: &mut EditDlg, pal: &Pal, fam: &egui::FontFamily) {
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(if d.id.is_some() { "Edit game details" } else { "Add a game" })
                .family(fam.clone())
                .size(19.0)
                .strong()
                .color(pal.text),
        );
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(&d.title_name).size(14.0).color(pal.dim)).truncate());
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for t in Tab::ALL {
            if tab_button(ui, t.label(), d.tab == t, pal).clicked() {
                d.set_tab(st, t);
            }
        }
    });
    let y = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1.0, with_alpha(pal.border, 160)));
    ui.add_space(12.0);
}

/// Save / Cancel with a note beside them, and Download metadata… with the lookup's status.
/// Returns (Save clicked, what else to do).
fn edit_footer(
    ui: &mut Ui,
    st: &mut AppState,
    d: &mut EditDlg,
    game: Option<&Game>,
    pal: &Pal,
) -> (bool, Option<EditOutcome>) {
    let mut save = false;
    let mut out = None;
    let problem = d.problem();
    // Save first on the left, like every dialog's primary button; the lookup on the right.
    ui.horizontal(|ui| {
        let label = if d.id.is_some() { "Save" } else { "Add game" };
        let r = primary_button(ui, label, pal, problem.is_none());
        if r.clicked() {
            save = true;
        }
        if plain_button(ui, "Cancel").clicked() {
            out = Some(EditOutcome::Close);
        }
        let note: Option<(String, egui::Color32)> = if let Some(p) = problem.as_deref().or(d.error.as_deref()) {
            // A missing name isn't a mistake yet: say it quietly.
            let quiet = d.name.text.trim().is_empty() && d.error.is_none();
            Some((p.to_string(), if quiet { pal.dim } else { pal.danger }))
        } else if let Some(other) = d.duplicate_hint(st) {
            Some((format!("Already in your library: {other}"), pal.dim))
        } else if d.id.is_none() && !d.actions.iter().any(|a| a.play) && d.template.launch.is_none() {
            Some(("No Play action yet (Actions tab)".to_string(), pal.dim))
        } else {
            None
        };
        // The note beside Save and the lookup's status beside its button share the
        // room between them.
        if let Some((text, col)) = note {
            let w = (ui.available_width() - 160.0) * 0.5;
            ui.scope(|ui| {
                ui.set_max_width(w.max(60.0));
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(col)).truncate());
            });
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let can_look = !d.name.text.trim().is_empty() && !d.lookup_busy();
            if ui
                .add_enabled(can_look, egui::Button::new("Download metadata…").min_size(vec2(0.0, 30.0)))
                .on_hover_text(
                    "Look the game up online by the name (and platform) above. What it finds \
                     replaces details looked up before; your own values stay. Nothing is \
                     saved until Save.",
                )
                .clicked()
            {
                d.start_lookup(st, game, None);
            }
            // Right to left: the status text, then "Settings…", then the spinner.
            if let Some((text, bad, settings)) = d.lookup_status() {
                let col = if bad { pal.danger } else { pal.dim };
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(col)).truncate());
                if settings && ui.link("Settings…").clicked() {
                    out = Some(EditOutcome::OpenSettings);
                }
                if d.lookup_busy() {
                    lazy_spinner(ui, 14.0, pal.accent);
                }
            }
        });
    });
    (save, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_round_trip() {
        assert_eq!(parse_date("1970-01-01"), Some(0));
        let t = parse_date("2024-02-29").unwrap();
        assert_eq!(format_date(t), "2024-02-29");
        assert_eq!(parse_date("2023-02-29"), None);
        assert_eq!(parse_date("2023-13-01"), None);
        assert_eq!(parse_date("yesterday"), None);
        assert_eq!(format_date(1_700_000_000), "2023-11-14");
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration(""), Some(0));
        assert_eq!(parse_duration("12h 30m"), Some(12 * 3600 + 30 * 60));
        assert_eq!(parse_duration("45m"), Some(45 * 60));
        assert_eq!(parse_duration("1:05"), Some(3900));
        assert_eq!(parse_duration("1.5"), Some(5400));
        assert_eq!(parse_duration("12 hours"), None);
        assert_eq!(parse_duration("5"), Some(5 * 3600));
        assert_eq!(format_duration(12 * 3600 + 30 * 60), "12h 30m");
        assert_eq!(format_duration(0), "0m");
        assert_eq!(parse_duration(&format_duration(7 * 3600)), Some(7 * 3600));
    }

    #[test]
    fn fields_follow_the_source_until_changed() {
        let mut f = Field::new(None, Some("Larian".into()));
        assert!(!f.over && f.text == "Larian");
        f.text = "Larian Studios".into();
        f.touched();
        assert!(f.over);
        assert_eq!(f.value().as_deref(), Some("Larian Studios"));
        f.text = " Larian ".into();
        f.touched();
        assert!(!f.over, "back to the source's value");
        assert_eq!(f.value(), None);
        let mut hidden = Field::new(Some("Mine".into()), None);
        hidden.reset();
        assert!(!hidden.over && hidden.text.is_empty());
    }

    #[test]
    fn edits_from_the_form() {
        let mut g = super::super::tests::fake_game(7, "Hollow Knight", Source::Steam);
        g.meta.genres = vec!["Action".into()];
        g = g.with_edits_applied();
        g.edits.install_size = Some(42);
        let mut d = EditDlg::from_game(&g, 0);
        d.genres.text = "Action, Metroidvania".into();
        d.genres.touched();
        d.release_date.text = "2017-02-24".into();
        d.release_date.touched();
        d.user_score.text = "95".into();
        d.links.push(GameLink { name: String::new(), url: "https://hollowknight.wiki/".into() });
        d.actions.push(GameAction {
            name: " ".into(),
            path: " {InstallDir}/hk ".into(),
            cwd: Some("  ".into()),
            ..Default::default()
        });
        let e = d.build_edits(&g.edits);
        assert_eq!(e.genres.as_deref(), Some(&["Action".to_string(), "Metroidvania".to_string()][..]));
        assert_eq!(e.release_year, Some(2017), "year from the date");
        assert_eq!(e.user_score, Some(95));
        assert_eq!(e.developer, None, "untouched fields stay the source's");
        assert_eq!(e.install_size, Some(42), "fields the form doesn't show are kept");
        assert_eq!(e.links[0].name, "hollowknight.wiki");
        assert_eq!(e.actions[0].name, "Action 1");
        assert_eq!(e.actions[0].path, "{InstallDir}/hk");
        assert_eq!(e.actions[0].cwd, None);
        assert!(d.problem().is_none());
        d.user_score.text = "101".into();
        assert!(d.problem().is_some());
    }

    #[test]
    fn the_edit_dialog_leaves_game_options_alone() {
        use crate::game_options::Tri;
        for hdr in [Tri::Default, Tri::On, Tri::Off] {
            let mut g = super::super::tests::fake_game(8, "Ori", Source::Steam);
            g.edits.game_options.hdr = hdr;
            g.edits.game_options.gamescope = Tri::On;
            let d = EditDlg::from_game(&g, 0);
            let e = d.build_edits(&g.edits);
            assert_eq!(e.game_options, g.edits.game_options, "HDR {hdr:?} is edited in Launch options only");
        }
    }

    /// A tester's case: an emulated "NPIA00010" (the scanner knows a year) was looked up as
    /// something wrong; the user cleared a few fields to get rid of it.
    fn wrongly_matched_game() -> (crate::db::Db, GameId) {
        let mut db = crate::db::Db::open_memory().unwrap();
        let mut sg = ScannedGame::new(Source::Emulator, "ps3/NPIA00010", "NPIA00010");
        sg.meta.release_year = Some(2008);
        db.merge_scan(&[sg], &[Source::Emulator]).unwrap();
        let id = db.load_all().unwrap()[0].id;
        let wrong = Metadata {
            description: Some("Some other game".into()),
            developer: Some("Wrong Dev".into()),
            publisher: Some("Wrong Pub".into()),
            genres: vec!["Racing".into()],
            features: vec!["Split screen".into()],
            series: Some("Wrong series".into()),
            critic_score: Some(40),
            protondb: Some("gold".into()),
            ..Default::default()
        };
        db.set_fetched(id, &Art::default(), &wrong).unwrap();
        let cleared = UserEdits {
            developer: Some(String::new()),
            release_year: Some(CLEARED_YEAR),
            critic_score: Some(CLEARED_SCORE),
            ..Default::default()
        };
        db.set_edits(id, &cleared).unwrap();
        (db, id)
    }

    #[test]
    fn a_cleared_year_or_score_stays_cleared() {
        let (db, id) = wrongly_matched_game();
        let g = db.get(id).unwrap().unwrap();
        assert_eq!(g.meta.release_year, None, "the scanner's year doesn't come back");
        assert_eq!(g.meta.critic_score, None);
        assert_eq!(g.meta.developer, None);
        // The form shows them empty, as the user's own (cleared) values, and saves them so.
        let d = EditDlg::from_game(&g, 0);
        assert!(d.release_year.over && d.release_year.text.is_empty());
        assert!(d.critic_score.over && d.critic_score.text.is_empty());
        let e = d.build_edits(&g.edits);
        assert_eq!((e.release_year, e.critic_score), (Some(CLEARED_YEAR), Some(CLEARED_SCORE)));
        // Clearing in the form: the same markers (a typed full date still gives its year).
        let mut d = EditDlg::from_game(&g, 0);
        d.release_year = Field::new(None, Some("2008".into()));
        d.release_year.text.clear();
        d.release_year.touched();
        assert_eq!(d.build_edits(&g.edits).release_year, Some(CLEARED_YEAR));
        d.release_date.text = "2008-12-11".into();
        d.release_date.touched();
        assert_eq!(d.build_edits(&g.edits).release_year, Some(2008));
    }

    #[test]
    fn download_metadata_replaces_the_old_match_in_one_step() {
        let (db, id) = wrongly_matched_game();
        let g = db.get(id).unwrap().unwrap();
        // Edit → type the new name and an own series → Download metadata…
        let mut d = EditDlg::from_game(&g, 0);
        d.name.text = "PlayStation Home".into();
        d.name.touched();
        d.series.text = "My series".into();
        d.series.touched();
        assert_eq!(d.typed_game(Some(&g)).display_name(), "PlayStation Home", "the typed name is asked for");
        let found = Metadata {
            description: Some("PlayStation Home was a virtual 3D social gaming platform.".into()),
            developer: Some("SCE London Studio".into()),
            genres: vec!["Social".into()],
            release_year: Some(2008),
            release_date: Some("2008-12-11".into()),
            series: Some("Their series".into()),
            critic_score: Some(70),
            ..Default::default()
        };
        // The dialog shows the answer before Save (nothing written yet).
        let preview = db.preview_fetched(id, &Art::default(), &found).unwrap().unwrap();
        assert_eq!(db.get(id).unwrap().unwrap(), g, "a preview writes nothing");
        let changed = d.take_preview(&preview, 0);
        assert_eq!(d.description.text, "PlayStation Home was a virtual 3D social gaming platform.");
        assert!(d.description.updated && d.genres.updated);
        assert_eq!(d.developers.text, "SCE London Studio", "a cleared field is filled again: the user asked");
        assert!(!d.developers.over);
        assert_eq!(d.release_year.text, "2008");
        assert_eq!(d.critic_score.text, "70");
        assert_eq!(d.publishers.text, "", "nothing left from the old match");
        assert_eq!(d.features.text, "");
        assert_eq!(d.series.text, "My series", "the user's own typing stays");
        assert!(changed >= 8, "{changed}");
        // Save: the answer replaces the old fetch, then the edits go over it.
        let e = d.build_edits(&g.edits);
        assert_eq!((e.developer.as_deref(), e.release_year, e.critic_score), (None, None, None), "markers gone");
        db.replace_fetched(id, &Art::default(), &found).unwrap();
        db.set_edits(id, &e).unwrap();
        db.set_user_name(id, d.name.value().as_deref()).unwrap();
        let after = db.get(id).unwrap().unwrap();
        assert_eq!(after.display_name(), "PlayStation Home");
        assert_eq!(after.meta.developer.as_deref(), Some("SCE London Studio"));
        assert_eq!(after.meta.publisher, None);
        assert!(after.meta.features.is_empty());
        assert_eq!(after.meta.genres, ["Social"]);
        assert_eq!(after.meta.series.as_deref(), Some("My series"));
        assert_eq!(after.meta.release_year, Some(2008));
        assert_eq!(after.meta.critic_score, Some(70));
        assert_eq!(after.meta.protondb.as_deref(), Some("gold"), "the Steam app's facts stay");
        // Edit again: the form matches what is saved (no leftovers, nothing marked).
        let d2 = EditDlg::from_game(&after, 0);
        assert!(!d2.developers.over && !d2.description.updated);
        assert_eq!(d2.publishers.text, "");
    }

    #[test]
    fn the_looked_up_answer_wins_over_the_scanner_and_survives_background_lookups() {
        let (db, id) = wrongly_matched_game();
        db.set_edits(id, &UserEdits::default()).unwrap();
        assert_eq!(db.get(id).unwrap().unwrap().meta.release_year, Some(2008), "the scanner's year");
        // A re-release: the user's lookup says 2011.
        let found = Metadata { release_year: Some(2011), description: Some("Re-release".into()), ..Default::default() };
        db.replace_fetched(id, &Art::default(), &found).unwrap();
        assert_eq!(db.get(id).unwrap().unwrap().meta.release_year, Some(2011));
        // A later background answer (ProtonDB refresh, IGDB gap fill) keeps it.
        db.set_fetched(id, &Art::default(), &Metadata { protondb: Some("silver".into()), ..Default::default() })
            .unwrap();
        let g = db.get(id).unwrap().unwrap();
        assert_eq!((g.meta.release_year, g.meta.protondb.as_deref()), (Some(2011), Some("silver")));
        assert_eq!(g.meta.description.as_deref(), Some("Re-release"));
        // Looking up the old name again (renamed back) replaces it once more.
        let old = Metadata { release_year: Some(2008), description: Some("Original".into()), ..Default::default() };
        db.replace_fetched(id, &Art::default(), &old).unwrap();
        let g = db.get(id).unwrap().unwrap();
        assert_eq!((g.meta.release_year, g.meta.description.as_deref()), (Some(2008), Some("Original")));
    }

    #[test]
    fn a_lookup_keeps_typed_values_and_refills_only_with_something() {
        let fresh = |s: &str| Field { text: String::new(), source: Some(s.into()), over: true, updated: false };
        // Typed by the user: stays.
        let mut f = Field::own(Some("Mine".into()));
        assert!(!f.take_lookup(&fresh("New")));
        assert_eq!(f.text, "Mine");
        // Cleared: refilled when the lookup has a value, stays cleared otherwise.
        let mut f = Field::own(Some(String::new()));
        assert!(!f.take_lookup(&fresh("")));
        assert!(f.over);
        assert!(f.take_lookup(&fresh("New")));
        assert!(!f.over && f.updated && f.text == "New");
        // Following the source: takes the new value, empty too; typing removes the mark.
        let mut f = Field::new(None, Some("Old".into()));
        assert!(f.take_lookup(&fresh("")));
        assert_eq!(f.text, "");
        f.text = "x".into();
        f.touched();
        assert!(!f.updated);
    }

    #[test]
    fn link_names() {
        assert_eq!(link_name("https://www.protondb.com/app/1"), "protondb.com");
        assert_eq!(link_name("example.org"), "example.org");
    }

    #[test]
    fn new_game_play_action_becomes_its_launch() {
        let a = GameAction {
            name: "Play".into(),
            path: "/opt/game/run.sh".into(),
            args: "--full 'two words'".into(),
            play: true,
            ..Default::default()
        };
        match action_spec(&a) {
            Some(LaunchSpec::Command { program, args, .. }) => {
                assert_eq!(program, "/opt/game/run.sh");
                assert_eq!(args, ["--full", "two words"]);
            }
            other => panic!("{other:?}"),
        }
        let var = GameAction { path: "{InstallDir}/run".into(), ..a };
        assert!(action_spec(&var).is_none());
    }
}
