//! What the user asked for while a frame was drawn ([`Action`]), applied after drawing.

use super::*;

/// Something the user asked for this frame; applied after drawing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Action {
    Select(GameId),
    Play(GameId),
    Install(GameId),
    /// Stop / cancel this game's launch (ui/launching.rs `stop`).
    Stop(GameId),
    ToggleFavorite(GameId),
    Hide(GameId),
    Unhide(GameId),
    Rename(GameId),
    EditTags(GameId),
    LaunchOptions(GameId),
    FetchMetadata(GameId),
    OpenFolder(GameId),
    Remove(GameId),
    /// Remove games that are no longer in their library (asks first, Undo).
    RemoveMissing(Vec<GameId>),
    /// Add Game → Manually… (Edit Game Details on a new game).
    AddManual,
    /// Add Game → Pick a program…
    AddProgram,
    /// Add Game → Scan for games… (the import checklist).
    AddScan,
    /// Add Game → Emulated games… (the ROM folder import).
    AddRoms,
    /// Add Game → Cloud game… (GeForce NOW, Xbox Cloud Gaming …).
    AddCloud,
    /// Edit Game Details on a tab (F3, pencil, double-click the title, right-click → Edit…).
    Edit(GameId, edit::Tab),
    /// Filter the library by a value clicked in the details (true: add to the current filter
    /// instead of replacing it, like Ctrl+click in the explorer).
    FilterBy(FilterValue, bool),
    /// Remove one active filter (a chip's ×).
    Unfilter(FilterValue),
    SetCompletion(GameId, Option<CompletionStatus>),
    /// Multi-selection → Set completion status: one status for every game (one Undo).
    SetCompletionMany(Vec<GameId>, Option<CompletionStatus>),
    /// Right-click → User score: one score (0..=100, None = clear) for these games (one Undo).
    SetScore(Vec<GameId>, Option<u8>),
    /// Top bar dice / Ctrl+R: select a random game of the view.
    RandomGame,
    /// Run one of the game's own actions (`edits.actions[i]`).
    PlayExtra(GameId, usize),
    /// A line of "Choose what to play" (Play ▾).
    PlayChoice(GameId, crate::model::PlayChoice),
    /// Play ▾ → Ask every time (forget "Always use this").
    ForgetPlayChoice(GameId),
    /// Right-click → Add to another game as a ROM…
    AddAsRom(GameId),
    /// Play ▾ → Play in a window (emulated games: the emulator without its fullscreen words).
    PlayWindowed(GameId),
    /// Play ▾ → "<Emulator> settings…" (the emulator's own window, no game, no playtime).
    OpenEmulator(GameId),
    /// Play ▾ → Play with ▸ (game, emulator, profile, always use it for this game).
    PlayWith(GameId, String, String, bool),
    OpenUrl(String),
    CreateShortcut(GameId),
    CalcSize(GameId),
    Settings(settings::Tab),
    /// The Add-ons window on a page.
    Addons(addons::Section),
    /// The theme creator: its start, or a user theme (key) open for editing.
    ThemeCreator(Option<String>),
    /// Uninstall through the game's store / add-on.
    Uninstall(GameId),
    About,
    /// About → Credits (thanks, sources, licences).
    Credits,
    /// The built-in changelog.
    WhatsNew,
    /// Open the artwork editor on a slot.
    EditArt(GameId, crate::art::ArtSlot),
    /// The full-window artwork viewer, starting at a slot.
    ViewArt(GameId, crate::art::ArtSlot),
    /// Open the bulk "Download artwork" dialog.
    BulkArt(artwork::BulkScope),
    /// Ctrl+click: add / remove a game from the multi-selection.
    SelectToggle(GameId),
    /// Shift+click: select the range from the anchor to this game.
    SelectRange(GameId),
    ClearMulti,
    /// Hide / unhide several games (a multi-selection).
    HideMany(Vec<GameId>),
    UnhideMany(Vec<GameId>),
    /// "Not a game": out of every view, never re-added by a rescan.
    NotAGame(Vec<GameId>),
    /// "…and everything named like this": the pattern dialog for this game's name.
    NotAGameLike(GameId),
    /// "Game options for N games…" (bulk game options).
    BulkOptions(Vec<GameId>),
    /// Set (or with None, clear) the user's controller-support setting.
    SetController(GameId, Option<ControllerSupport>),
    /// Show one of the user's collections (None: back to All games).
    SelectCollection(Option<u64>),
    /// Add games to a manual collection.
    AddToCollection(u64, Vec<GameId>),
    /// Take games out of a manual collection (with Undo).
    RemoveFromCollection(u64, Vec<GameId>),
    /// "New collection…" (name prompt), with these games in it.
    NewCollection(Vec<GameId>),
    /// "New smart collection from this filter…" (name prompt).
    NewSmartCollection,
    RenameCollection(u64),
    /// Delete a collection (with Undo; its games stay in the library).
    DeleteCollection(u64),
    /// Move a collection up (-1) or down (+1) in the sidebar.
    MoveCollection(u64, i32),
    /// Replace a smart collection's rule with the current filter.
    UpdateSmartCollection(u64),
    /// Menu → Statistics (stats.rs).
    Statistics,
}

impl DesktopUi {
    pub(crate) fn push(&mut self, a: Action) {
        self.actions.push(a);
    }

    pub(super) fn apply_actions(&mut self, ctx: &egui::Context, st: &mut AppState) {
        for a in std::mem::take(&mut self.actions) {
            self.apply(ctx, st, a);
        }
    }

    fn apply(&mut self, ctx: &egui::Context, st: &mut AppState, a: Action) {
        match a {
            Action::Select(id) => {
                self.multi.clear();
                self.multi_anchor = Some(id);
                self.focus(st, id);
            }
            Action::SelectToggle(id) => self.select_toggle(st, id),
            Action::SelectRange(id) => self.select_range(st, id),
            Action::ClearMulti => self.multi.clear(),
            Action::SetController(id, v) => st.set_controller_override(id, v),
            Action::SelectCollection(_)
            | Action::AddToCollection(..)
            | Action::RemoveFromCollection(..)
            | Action::NewCollection(_)
            | Action::NewSmartCollection
            | Action::RenameCollection(_)
            | Action::DeleteCollection(_)
            | Action::MoveCollection(..)
            | Action::UpdateSmartCollection(_) => self.collection_action(ctx, st, a),
            Action::EditArt(id, slot) => {
                // The artwork pencils open Edit Game Details on its Media tab.
                if let Some(g) = st.library.get(id) {
                    let mut d = edit::EditDlg::for_game(st, g);
                    d.set_tab(st, edit::Tab::Media);
                    d.set_art_slot(slot);
                    self.dlg.edit = Some(d);
                }
            }
            Action::ViewArt(id, slot) => {
                if let Some(g) = st.library.get(id) {
                    self.dlg.viewer = viewer::ViewerDlg::open(ctx, g, slot);
                }
            }
            Action::BulkArt(scope) => {
                let d = self.bulk_dlg(st, scope);
                self.dlg.bulk = Some(d);
            }
            Action::Play(id) => Self::play(ctx, st, id),
            Action::Install(id) => Self::install(st, id),
            Action::Stop(id) => crate::ui::launching::stop(st, id),
            Action::ToggleFavorite(id) => st.toggle_favorite(id),
            Action::Hide(id) => {
                if st.library.get(id).is_some_and(|g| g.hidden) {
                    st.set_hidden(id, false);
                } else {
                    self.hide_games(ctx, st, vec![id]);
                }
            }
            Action::HideMany(ids) => self.hide_games(ctx, st, ids),
            Action::Unhide(id) => st.set_hidden(id, false),
            Action::UnhideMany(ids) => st.set_hidden_many(&ids, false),
            Action::NotAGame(ids) => self.not_a_game(ctx, st, ids),
            Action::NotAGameLike(id) => {
                if let Some(g) = st.library.get(id) {
                    self.dlg.exclude_like = Some(cleanup::ExcludeLikeDlg::new(g));
                }
            }
            Action::BulkOptions(ids) => {
                self.dlg.bulk_opts = cleanup::BulkOptsDlg::new(st, ids);
            }
            Action::Rename(id) => {
                if let Some(g) = st.library.get(id) {
                    self.dlg.rename = Some(dialogs::RenameDlg::new(g));
                }
            }
            Action::EditTags(id) => {
                if let Some(g) = st.library.get(id) {
                    self.dlg.tags = Some(dialogs::TagsDlg::new(g));
                }
            }
            Action::LaunchOptions(id) => {
                if let Some(g) = st.library.get(id) {
                    self.dlg.launch = Some(dialogs::LaunchDlg::new(g));
                }
            }
            Action::FetchMetadata(id) => {
                // Edit Game Details with the lookup started: its answer shows in the fields
                // (and a name matching several games asks which one) before Save.
                if let Some(g) = st.library.get(id).cloned() {
                    let mut d = edit::EditDlg::for_game(st, &g);
                    d.start_lookup(st, Some(&g), None);
                    self.dlg.edit = Some(d);
                }
            }
            Action::OpenFolder(id) => {
                let dir = st.library.get(id).and_then(|g| g.install_dir.clone());
                match dir {
                    Some(d) => st.open_path(&d),
                    None => st.notify_error("No install folder known for this game"),
                }
            }
            Action::Remove(id) => self.dlg.remove = Some(id),
            Action::RemoveMissing(ids) => self.dlg.remove_missing = Some(ids),
            Action::AddManual => {
                self.dlg.edit = Some(edit::EditDlg::new_game(st, crate::edit_app::NewGame::default()));
            }
            Action::AddProgram => self.dlg.program = Some(add::ProgramDlg::pick(ctx)),
            Action::AddScan => self.dlg.import = Some(add::ImportDlg::open(st, None)),
            Action::AddRoms => self.dlg.roms = Some(emulation_import::RomImportDlg::open(st, None)),
            Action::AddCloud => self.dlg.cloud = Some(add_cloud::CloudDlg::open(ctx, None)),
            Action::Edit(id, tab) => {
                if let Some(g) = st.library.get(id) {
                    let mut d = edit::EditDlg::for_game(st, g);
                    d.set_tab(st, tab);
                    self.dlg.edit = Some(d);
                }
            }
            Action::FilterBy(v, add) => self.filter_by(v, add),
            Action::Unfilter(v) => self.unfilter(&v),
            Action::SetCompletion(id, c) => st.set_completion(id, c),
            Action::SetCompletionMany(ids, c) => self.set_completion_many(ctx, st, ids, c),
            Action::SetScore(ids, score) => self.set_score_many(ctx, st, ids, score),
            Action::RandomGame => self.random_game(ctx, st),
            Action::PlayExtra(id, i) => {
                if Self::not_running(st, id) {
                    st.launch_action(id, i);
                }
            }
            Action::PlayWindowed(id) => {
                if Self::not_running(st, id) {
                    st.launch_windowed(id);
                }
            }
            Action::OpenEmulator(id) => emulation_ui::open_game_emulator(st, id),
            Action::PlayWith(id, emu, profile, always) => st.launch_with(id, &emu, &profile, always),
            Action::PlayChoice(id, c) => {
                if Self::not_running(st, id) {
                    st.launch_choice(id, c, false);
                }
            }
            Action::ForgetPlayChoice(id) => st.forget_play_choice(id),
            Action::AddAsRom(id) => emulation_ui::addrom::open(ctx, id),
            Action::OpenUrl(u) => st.open_url(&u),
            Action::CreateShortcut(id) => st.create_desktop_shortcut(id),
            Action::CalcSize(id) => st.calc_install_size(id),
            Action::Settings(tab) => match &mut self.dlg.settings {
                // Already open (its own window, under another dialog): that page, in front.
                Some(d) => {
                    d.tab = tab;
                    self.own_focus(st, own_window::Win::Settings);
                }
                None => self.dlg.settings = Some(settings::SettingsDlg::new(tab)),
            },
            Action::Addons(section) => self.dlg.addons = Some(self.addons_dlg(section)),
            Action::ThemeCreator(key) => match &mut self.dlg.creator {
                Some(_) => self.own_focus(st, own_window::Win::Creator),
                None => {
                    self.dlg.creator = Some(match key {
                        Some(k) => theme_creator::CreatorDlg::open(st, &k),
                        None => theme_creator::CreatorDlg::new(),
                    })
                }
            },
            Action::Uninstall(id) => st.uninstall_game(id),
            Action::About => self.dlg.about = true,
            Action::Credits => self.dlg.credits = Some(credits::CreditsDlg::default()),
            Action::WhatsNew => self.dlg.whats_new = true,
            Action::Statistics => self.stats = Some(stats::StatsPage::default()),
        }
    }

    /// Ctrl+click: add `id` to the multi-selection or take it out (the focused game joins it
    /// on the first Ctrl+click; one game left is a plain selection again).
    fn select_toggle(&mut self, st: &AppState, id: GameId) {
        if self.multi.is_empty()
            && let Some(cur) = self.selected
            && cur != id
        {
            self.multi.insert(cur);
        }
        if !self.multi.remove(&id) {
            self.multi.insert(id);
            self.focus(st, id);
        }
        if self.multi.len() < 2 {
            if let Some(&only) = self.multi.iter().next() {
                self.focus(st, only);
            }
            self.multi.clear();
        }
        self.multi_anchor = Some(id);
    }

    /// Shift+click: select the shown games from the anchor to `id`.
    fn select_range(&mut self, st: &AppState, id: GameId) {
        let items = &self.groups.items;
        let pos_of = |g: GameId| items.iter().position(|&i| st.library.games[i].id == g);
        let anchor = self.multi_anchor.or(self.selected).and_then(pos_of);
        let clicked = self.groups.clicked_pos.filter(|&p| items.get(p).is_some_and(|&i| st.library.games[i].id == id));
        if let (Some(a), Some(b)) = (anchor, clicked.or_else(|| pos_of(id))) {
            let (lo, hi) = (a.min(b), a.max(b));
            self.multi = items[lo..=hi].iter().map(|&i| st.library.games[i].id).collect();
            if self.multi.len() < 2 {
                self.multi.clear();
            }
        }
        self.focus(st, id);
    }

    /// Play: launch an installed game (or ask how to play it), install one that isn't.
    fn play(ctx: &egui::Context, st: &mut AppState, id: GameId) {
        let installed = st.library.get(id).is_some_and(|g| g.installed);
        if st.running_game() == Some(id) {
            st.notify("Already running");
        } else if installed {
            if !emulation_ui::play_needs_allow(ctx, st, id) {
                // Several ways to play: "Choose what to play" opens instead.
                emulation_ui::choose::play(ctx, st, id);
            }
        } else {
            Self::install(st, id);
        }
    }

    /// False (after saying so) when `id` is already running: no second launch of it.
    fn not_running(st: &mut AppState, id: GameId) -> bool {
        if st.running_game() == Some(id) {
            st.notify("Already running");
            return false;
        }
        true
    }

    /// The Add-ons window on a page, carrying over a theme import still running.
    fn addons_dlg(&mut self, section: addons::Section) -> addons::AddonsDlg {
        let mut d = addons::AddonsDlg::new(section);
        if let Some(t) = self.dlg.theme_job.take() {
            d.themes = t;
        }
        d
    }

    /// Install through the game's store (or add-on); a clear message when it can't be.
    fn install(st: &mut AppState, id: GameId) {
        if st.can_install(id) {
            st.install_game(id);
            return;
        }
        let Some(g) = st.library.get(id) else { return };
        let msg = match g.source {
            Source::Manual | Source::Desktop => {
                format!("{} isn't installed. Set its program in Edit… → Actions.", g.display_name())
            }
            Source::Emulator => {
                format!("{}: ROM not found — is the drive connected? (Settings → Emulation)", g.display_name())
            }
            s => format!("{} isn't installed. Install it from {}.", g.display_name(), s.display()),
        };
        st.notify_error(msg);
    }
}
