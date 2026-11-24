//! Modal dialogs: launch options, rename, tags, remove confirmation, about; dispatch of the
//! others (Edit Game Details, Add Game, artwork, settings).

use super::{DesktopUi, Pal, settings::SettingsDlg};
use crate::app::AppState;
use crate::model::*;
use crate::ui::widgets::mix;
use egui::{Color32, Id, Key, RichText, Ui, vec2};
use std::path::PathBuf;

#[derive(Default)]
pub(crate) struct Dialogs {
    pub launch: Option<LaunchDlg>,
    pub rename: Option<RenameDlg>,
    pub tags: Option<TagsDlg>,
    pub edit: Option<super::edit::EditDlg>,
    pub program: Option<super::add::ProgramDlg>,
    pub import: Option<super::add::ImportDlg>,
    /// Add game → Emulated games… (`emulation_import.rs`).
    pub roms: Option<super::emulation_import::RomImportDlg>,
    /// Add game → Cloud game… (`add_cloud.rs`).
    pub cloud: Option<super::add_cloud::CloudDlg>,
    pub remove: Option<GameId>,
    /// Remove games no longer in their library (`missing_ui.rs`).
    pub remove_missing: Option<Vec<GameId>>,
    pub about: bool,
    /// About → Credits (`credits.rs`).
    pub credits: Option<super::credits::CreditsDlg>,
    /// "What's new" (the built-in changelog).
    pub whats_new: bool,
    /// Changelog → Export… save dialog still running (kept when the changelog closes).
    pub changelog_save: Option<super::whats_new_ui::ChangelogSave>,
    /// Testing hook (`whatsnew-export:<path>`): write the changelog here, no dialog.
    pub changelog_export_to: Option<PathBuf>,
    /// Testing hook (`whatsnew-menu`): show the changelog's Export menu open.
    pub debug_export_menu: bool,
    /// Testing hook (`whatsnew-saving`): pretend a save dialog is up.
    pub debug_export_saving: bool,
    pub settings: Option<SettingsDlg>,
    pub art: Option<super::artwork::ArtDlg>,
    pub bulk: Option<super::artwork::BulkDlg>,
    pub addons: Option<super::addons::AddonsDlg>,
    /// A theme install / import still running after the Add-ons window closed (finishes in the
    /// background with a notice; reopening Add-ons picks it up again).
    pub theme_job: Option<super::addons::themes::Themes>,
    /// "Not a game … and everything named like this".
    pub exclude_like: Option<super::cleanup::ExcludeLikeDlg>,
    /// "Game options for N games…".
    pub bulk_opts: Option<super::cleanup::BulkOptsDlg>,
    /// The full-window artwork viewer.
    pub viewer: Option<super::viewer::ViewerDlg>,
    /// The theme creator (theme_creator.rs).
    pub creator: Option<super::theme_creator::CreatorDlg>,
}

impl Dialogs {
    pub fn any_open(&self) -> bool {
        self.launch.is_some()
            || self.rename.is_some()
            || self.tags.is_some()
            || self.edit.is_some()
            || self.program.is_some()
            || self.import.is_some()
            || self.roms.is_some()
            || self.cloud.is_some()
            || self.remove.is_some()
            || self.remove_missing.is_some()
            || self.about
            || self.credits.is_some()
            || self.settings.is_some()
            || self.art.is_some()
            || self.bulk.is_some()
            || self.addons.is_some()
            || self.creator.is_some()
            || self.exclude_like.is_some()
            || self.bulk_opts.is_some()
            || self.viewer.is_some()
    }
}

/// Standard modal look: window fill, rounded, padded; `width` fixed. In the dialog's own window
/// (own_window.rs) it fills that window instead (its full width) and closes the same ways.
pub(crate) fn modal<R>(
    ctx: &egui::Context,
    id: &str,
    pal: &Pal,
    width: f32,
    add: impl FnOnce(&mut Ui) -> R,
) -> (R, bool) {
    if super::own_window::hosts_modal(ctx, id) {
        return super::own_window::fill_window(ctx, id, pal, add);
    }
    let frame = egui::Frame::new()
        .fill(pal.surface)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(10)
        .inner_margin(egui::Margin::same(20))
        .shadow(egui::epaint::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(120) });
    let resp =
        egui::Modal::new(Id::unique(id)).frame(frame).backdrop_color(Color32::from_black_alpha(140)).show(ctx, |ui| {
            ui.set_width(width);
            add(ui)
        });
    let close = resp.should_close();
    (resp.inner, close)
}

pub(crate) fn title(ui: &mut Ui, text: &str, sub: Option<&str>, pal: &Pal, family: &egui::FontFamily) {
    ui.label(RichText::new(text).family(family.clone()).size(19.0).strong().color(pal.text));
    if let Some(s) = sub {
        ui.label(RichText::new(s).size(13.0).color(pal.dim));
    }
    ui.add_space(10.0);
}

pub(crate) fn primary_button(ui: &mut Ui, text: &str, pal: &Pal, enabled: bool) -> egui::Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(text).color(pal.accent_text).strong())
            .fill(pal.accent)
            .min_size(vec2(90.0, 30.0)),
    )
}

pub(crate) fn plain_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(text).min_size(vec2(80.0, 30.0)))
}

fn field_label(ui: &mut Ui, text: &str, pal: &Pal) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).size(13.0).strong().color(pal.text));
}

pub(crate) fn hint_text(ui: &mut Ui, text: &str, pal: &Pal) {
    ui.label(RichText::new(text).size(12.0).color(pal.dim));
}

/// A small web link under a hint ("dev.twitch.tv/console ↗"). True when clicked; the caller opens
/// the URL with `AppState::open_url` (xdg-open), so screenshot / test runs never start a browser.
pub(crate) fn link_text(ui: &mut Ui, label: &str, url: &str, pal: &Pal) -> bool {
    ui.add(
        egui::Label::new(RichText::new(format!("{label} ↗")).size(12.0).color(pal.accent).underline())
            .sense(egui::Sense::click()),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .on_hover_text(url)
    .clicked()
}

fn mono_box(ui: &mut Ui, text: &str, color: Color32, pal: &Pal) {
    egui::Frame::new()
        .fill(ui.visuals().extreme_bg_color)
        .stroke(egui::Stroke::new(1.0, pal.border))
        .corner_radius(5)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(RichText::new(text).monospace().color(color)).wrap_mode(egui::TextWrapMode::Wrap));
        });
}

// ------------------------------------------------------------------------------- launch options

pub(crate) struct LaunchDlg {
    pub id: GameId,
    name: String,
    source: Source,
    source_opts: Option<String>,
    pub options: String,
    pub use_custom: bool,
    pub command: String,
    pub cwd: String,
    had_override: bool,
    source_id: String,
    /// The source's own command line (for "start from the current command"), when it is a command.
    base_command: Option<(String, Option<PathBuf>)>,
    helper_error: Option<String>,
    /// Per-game game options (UserEdits::game_options) as edited.
    pub opts: crate::game_options::GameOptions,
    had_opts: bool,
    /// Last override + options the preview was computed for, and its result.
    preview_for: Option<(LaunchOverride, crate::game_options::GameOptions)>,
    preview: Result<LaunchPreview, String>,
    /// The published LaunchEnv the preview was computed against (defaults, library options,
    /// caps, monitor): any change there swaps the Arc, so the preview is recomputed.
    preview_env: Option<std::sync::Arc<crate::game_options::LaunchEnv>>,
}

/// The "Will run" result of the current edit.
#[derive(Clone, Debug, Default)]
pub(crate) struct LaunchPreview {
    display: String,
    notes: Vec<String>,
    /// What happened to each game option.
    report: Vec<(crate::game_options::OptionId, Option<String>, crate::game_options::OptionState)>,
    /// Steam games: the launch options to paste into Steam.
    steam_string: Option<String>,
}

impl LaunchDlg {
    pub fn new(g: &Game) -> LaunchDlg {
        let o = g.launch_override.clone().unwrap_or_default();
        LaunchDlg {
            id: g.id,
            name: g.display_name().to_string(),
            source: g.source,
            source_opts: g.source_launch_options.clone().filter(|s| !s.trim().is_empty()),
            use_custom: o.command.as_deref().is_some_and(|c| !c.trim().is_empty()),
            command: o.command.clone().unwrap_or_default(),
            cwd: o.cwd.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            options: o.options.clone(),
            had_override: g.launch_override.as_ref().is_some_and(|o| !o.is_empty()),
            source_id: g.source_id.clone(),
            base_command: match &g.launch {
                Some(LaunchSpec::Command { program, args, cwd, env }) => {
                    let mut words: Vec<String> = vec![];
                    if !env.is_empty() {
                        words.push("env".into());
                        words.extend(env.iter().map(|(k, v)| crate::launch::shell_quote(&format!("{k}={v}"))));
                    }
                    words.push(crate::launch::shell_quote(program));
                    words.extend(args.iter().map(|a| crate::launch::shell_quote(a)));
                    Some((words.join(" "), cwd.clone()))
                }
                _ => None,
            },
            helper_error: None,
            opts: g.edits.game_options.clone(),
            had_opts: !g.edits.game_options.is_default(),
            preview_for: None,
            preview: Ok(LaunchPreview::default()),
            preview_env: None,
        }
    }

    /// The override as currently edited.
    pub fn current(&self) -> LaunchOverride {
        let command = Some(self.command.trim().to_string()).filter(|c| self.use_custom && !c.is_empty());
        let cwd = Some(self.cwd.trim()).filter(|c| self.use_custom && !c.is_empty()).map(PathBuf::from);
        LaunchOverride { options: self.options.trim().to_string(), command, cwd }
    }

    /// Fill the custom command with Faugus' direct umu-run command (reads games.json; click only).
    pub fn fill_faugus_direct(&mut self, cfg: &crate::config::Config) {
        let ctx = crate::scan::ScanContext::from_config(cfg);
        match crate::scan::faugus::direct_launch(&ctx, &self.source_id) {
            Ok(dl) => {
                self.command = dl.command_line();
                self.cwd = dl.cwd.map(|p| p.display().to_string()).unwrap_or_default();
                self.use_custom = true;
                self.helper_error = None;
            }
            Err(e) => self.helper_error = Some(format!("{e:#}")),
        }
    }

    /// Fill the custom command with a direct (Steam-bypassing) command for a Steam game or shortcut.
    pub fn fill_steam_direct(&mut self, cfg: &crate::config::Config) {
        let ctx = crate::scan::ScanContext::from_config(cfg);
        match crate::scan::steam::direct_launch_for_source_id(&ctx, &self.source_id) {
            Ok(dl) => {
                self.command = dl.command_line();
                self.cwd = dl.cwd.display().to_string();
                self.use_custom = true;
                self.helper_error = None;
            }
            Err(e) => self.helper_error = Some(format!("{e:#}")),
        }
    }

    /// Fill the custom command with the source's own command line, to edit it.
    pub fn fill_base_command(&mut self) {
        if let Some((cmd, cwd)) = &self.base_command {
            self.command = cmd.clone();
            if self.cwd.trim().is_empty() {
                self.cwd = cwd.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
            }
            self.use_custom = true;
        }
    }

    fn refresh_preview(&mut self, st: &AppState) {
        let key = (self.current(), self.opts.clone());
        // Settings → Games, the probe finishing, or the window moving to another monitor while
        // this is open all publish a new LaunchEnv: compare it too.
        let env = crate::game_options::launch_env();
        if preview_fresh(self.preview_for.as_ref(), self.preview_env.as_ref(), &key, &env) {
            return;
        }
        self.preview = match st.library.get(self.id) {
            Some(g) => {
                let mut g = g.clone();
                g.launch_override = Some(key.0.clone()).filter(|o| !o.is_empty());
                g.edits.game_options = key.1.clone();
                crate::launch::resolve(&g)
                    .map(|r| LaunchPreview {
                        display: r.display,
                        notes: r.notes,
                        report: r.options,
                        steam_string: r.steam_string,
                    })
                    .map_err(|e| format!("{e:#}"))
            }
            None => Err("This game is no longer in the library".into()),
        };
        self.preview_for = Some(key);
        self.preview_env = Some(env);
    }
}

/// The cached preview still matches the edit and the published environment. (`env` is held while
/// comparing, so a pointer match can't be a freed-and-reused allocation.)
fn preview_fresh(
    for_key: Option<&(LaunchOverride, crate::game_options::GameOptions)>,
    for_env: Option<&std::sync::Arc<crate::game_options::LaunchEnv>>,
    key: &(LaunchOverride, crate::game_options::GameOptions),
    env: &std::sync::Arc<crate::game_options::LaunchEnv>,
) -> bool {
    for_key == Some(key) && for_env.is_some_and(|p| std::sync::Arc::ptr_eq(p, env))
}

/// What Lodestar's launch options can do for this game right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OptionsScope {
    /// Wrappers, variables and arguments (Lodestar runs the command itself).
    Full,
    /// Steam starts the game: only arguments can be passed (`steam -applaunch <id> args`).
    ArgsOnly,
    /// A non-Steam shortcut started through Steam: nothing can be added from here.
    None,
}

impl LaunchDlg {
    pub(crate) fn options_scope(&self) -> OptionsScope {
        if self.source != Source::Steam || self.use_custom {
            OptionsScope::Full
        } else if self.source_id.starts_with("shortcut:") {
            OptionsScope::None
        } else {
            OptionsScope::ArgsOnly
        }
    }
}

#[cfg(test)]
fn is_env_word(w: &str) -> bool {
    crate::launch::split_env_word(w).is_some()
}

impl DesktopUi {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context, st: &mut AppState) {
        let pal = self.pal;
        let fam = self.fonts.bold.clone();
        if let Some(d) = &mut self.dlg.launch
            && !launch_dialog(ctx, st, d, &pal, &fam)
        {
            self.dlg.launch = None;
        }
        if let Some(d) = &mut self.dlg.rename
            && !rename_dialog(ctx, st, d, &pal, &fam)
        {
            self.dlg.rename = None;
        }
        if let Some(d) = &mut self.dlg.tags
            && !tags_dialog(ctx, st, d, &pal, &fam)
        {
            self.dlg.tags = None;
        }
        use super::own_window::Win;
        if let Some(mut d) = self.dlg.edit.take() {
            use super::edit::EditOutcome;
            match self
                .host(ctx, st, Win::Edit, |me, ctx, st| me.edit_dialog(ctx, st, &mut d))
                .unwrap_or(EditOutcome::Keep)
            {
                EditOutcome::Keep => {
                    // Installation → "Set up emulation…".
                    if std::mem::take(&mut d.emu.open_settings) {
                        self.push(super::Action::Settings(super::settings::Tab::Emulation));
                    }
                    self.dlg.edit = Some(d);
                }
                EditOutcome::Close => {}
                EditOutcome::Added(id) => self.select_new(id),
                EditOutcome::OpenSettings => {
                    self.dlg.edit = Some(d);
                    self.push(super::Action::Settings(super::settings::Tab::Metadata));
                }
            }
        }
        if let Some(d) = &mut self.dlg.viewer
            && !super::viewer::viewer(ctx, st, d, &pal, &fam)
        {
            self.dlg.viewer = None;
        }
        self.add_dialogs(ctx, st);
        if self.dlg.roms.is_some() {
            let mut roms = self.dlg.roms.take();
            let a = self
                .host(ctx, st, Win::Roms, |_, ctx, st| super::emulation_import::show(ctx, st, &mut roms, &pal, &fam))
                .flatten();
            self.dlg.roms = roms;
            if let Some(a) = a {
                self.push(a);
            }
        }
        super::emulation_ui::choose::show(ctx, st, &pal);
        if let Some(to) = super::emulation_ui::addrom::show(ctx, st, &pal, &fam) {
            self.select_new(to);
        }
        self.cleanup_dialogs(ctx, st);
        self.missing_dialog(ctx, st);
        if let Some(id) = self.dlg.remove
            && !remove_dialog(ctx, st, id, &pal, &fam)
        {
            self.dlg.remove = None;
        }
        if self.dlg.about {
            let (keep, credits) = about_dialog(ctx, st, &pal, &fam);
            self.dlg.about = keep && !credits;
            if credits {
                self.dlg.credits = Some(super::credits::CreditsDlg::default());
            }
        }
        self.credits_dialog(ctx, st);
        self.whats_new_dialogs(ctx, st);
        // Artwork dialogs before Settings: "Settings → Metadata" opens on top of them.
        if let Some(mut d) = self.dlg.art.take() {
            use super::artwork::ArtOutcome;
            // Settings over it (in the main window, or in its own window while this one waits
            // behind a veil).
            d.covered = self.dlg.settings.is_some();
            match self
                .host(ctx, st, Win::Art, |_, ctx, st| super::artwork::art_dialog(ctx, st, &mut d, &pal, &fam))
                .unwrap_or(ArtOutcome::Keep)
            {
                ArtOutcome::Keep => self.dlg.art = Some(d),
                ArtOutcome::Close => {}
                ArtOutcome::OpenSettings => {
                    self.dlg.art = Some(d);
                    self.push(super::Action::Settings(super::settings::Tab::Metadata));
                }
            }
        }
        if let Some(mut d) = self.dlg.bulk.take() {
            use super::artwork::BulkOutcome;
            match super::artwork::bulk_dialog(ctx, st, &mut d, &pal, &fam) {
                BulkOutcome::Keep => self.dlg.bulk = Some(d),
                BulkOutcome::Close => {}
                BulkOutcome::OpenSettings => {
                    self.dlg.bulk = Some(d);
                    self.push(super::Action::Settings(super::settings::Tab::Metadata));
                }
            }
        }
        if let Some(mut d) = self.dlg.addons.take() {
            use super::addons::AddonsOutcome;
            match self
                .host(ctx, st, Win::Addons, |me, ctx, st| me.addons_dialog(ctx, st, &mut d))
                .unwrap_or(AddonsOutcome::Keep)
            {
                AddonsOutcome::Keep => self.dlg.addons = Some(d),
                AddonsOutcome::Close => {
                    if d.themes.busy() {
                        self.dlg.theme_job = Some(std::mem::take(&mut d.themes));
                    }
                }
                AddonsOutcome::OpenSettings(tab) => {
                    if d.themes.busy() {
                        self.dlg.theme_job = Some(std::mem::take(&mut d.themes));
                    }
                    self.push(super::Action::Settings(tab));
                }
            }
        }
        if let Some(a) = self.dlg.addons.as_mut().and_then(|d| d.themes.open_creator.take()) {
            self.push(super::Action::ThemeCreator(a));
        }
        if let Some(mut d) = self.dlg.creator.take() {
            use super::theme_creator::CreatorOutcome;
            let fam = fam.clone();
            match self
                .host(ctx, st, Win::Creator, |_, ctx, st| super::theme_creator::show(ctx, st, &mut d, &pal, &fam))
                .unwrap_or(CreatorOutcome::Keep)
            {
                CreatorOutcome::Keep => self.dlg.creator = Some(d),
                CreatorOutcome::Close => {
                    st.themes.refresh();
                    if let Some(a) = &mut self.dlg.addons {
                        a.themes.refresh_list(st);
                    }
                }
                CreatorOutcome::Test(key) => {
                    st.themes.refresh();
                    st.set_controller_theme(&key);
                    st.notify(format!("Controller mode uses “{}” now", st.controller_theme.file.theme.name));
                    st.switch_mode(crate::app::Mode::Controller);
                    self.dlg.creator = Some(d);
                }
            }
        }
        if let Some(mut t) = self.dlg.theme_job.take()
            && super::addons::themes::poll_background(st, &mut t)
        {
            self.dlg.theme_job = Some(t);
        }
        if self.dlg.settings.is_some() {
            let mut d = self.dlg.settings.take();
            let keep = self
                .host(ctx, st, Win::Settings, |me, ctx, st| d.as_mut().is_some_and(|d| me.settings_dialog(ctx, st, d)))
                .unwrap_or(true);
            if keep {
                self.dlg.settings = d;
            } else if let Some(section) = d.and_then(|d| d.open_addons) {
                self.push(super::Action::Addons(section));
            }
        }
        // Closed windows forgotten (sizes remembered); the main window's veil while one is open.
        self.own_end(ctx, st);
        // Last: it answers a Play click and sits on top of whatever is open.
        prompt_dialog(ctx, st, &pal, &fam);
    }
}

/// The launch prompts ("another game is running: close it and start the new one?", "quit while
/// a game runs?"): primary answer (Enter), an optional second one, or cancel (Esc, backdrop click).
fn prompt_dialog(ctx: &egui::Context, st: &mut AppState, pal: &Pal, fam: &egui::FontFamily) {
    use crate::ui::launching::{self, PromptAnswer};
    let Some(p) = launching::prompt(st) else {
        return;
    };
    let mut answer = None;
    let (_, close) = modal(ctx, "ls_switch_dlg", pal, 420.0, |ui| {
        title(ui, &p.title, None, pal, fam);
        ui.label(RichText::new(&p.body).color(pal.text));
        ui.add_space(4.0);
        hint_text(ui, &p.hint, pal);
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            let r = primary_button(ui, p.primary, pal, true);
            // Enter accepts, but not a held one (key repeat) from the Play that opened this.
            let enter = ui.input(|i| {
                i.events
                    .iter()
                    .any(|e| matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, repeat: false, .. }))
            });
            if r.clicked() || enter {
                answer = Some(PromptAnswer::Primary);
            }
            if let Some(alt) = p.alt
                && plain_button(ui, alt).clicked()
            {
                answer = Some(PromptAnswer::Alt);
            }
            if plain_button(ui, p.cancel).clicked() {
                answer = Some(PromptAnswer::Cancel);
            }
        });
    });
    if close && answer.is_none() {
        answer = Some(PromptAnswer::Cancel);
    }
    if let Some(a) = answer {
        launching::answer_prompt(st, p.kind, a);
    }
}

fn launch_dialog(ctx: &egui::Context, st: &mut AppState, d: &mut LaunchDlg, pal: &Pal, fam: &egui::FontFamily) -> bool {
    if d.preview_env.is_none() {
        // Just opened: look again for tools installed since the last probe (background; the
        // preview follows when it finishes).
        st.reprobe_game_options();
    }
    d.refresh_preview(st);
    let mut keep = true;
    let (_, close) = modal(ctx, "ls_launch_dlg", pal, 720.0, |ui| {
        title(ui, "Launch options", Some(&format!("{}  ·  {}", d.name, d.source.display())), pal, fam);

        // The "Will run" preview stays pinned below the scrolling part, so the result of an
        // edit is always visible (a long custom command would push it out of view).
        let max_h = (ui.ctx().content_rect().height() - 400.0).max(160.0);
        egui::ScrollArea::vertical().id_salt("ls_launch_scroll").max_height(max_h).auto_shrink([false, true]).show(
            ui,
            |ui| {
                game_options_section(ui, st, d, pal);
                ui.add_space(8.0);
                ui.separator();
                field_label(ui, "Your own launch options (optional)", pal);
                hint_text(
                    ui,
                    "Only needed for things the switches above don't cover. What you type here wins over them.",
                    pal,
                );
                source_options(ui, d, pal);
                own_options(ui, d, pal);
                ui.add_space(10.0);
                custom_command(ui, st, d, pal);
                ui.add_space(4.0);
            },
        );

        ui.add_space(6.0);
        ui.separator();
        field_label(ui, "Will run", pal);
        let ok = egui::ScrollArea::vertical()
            .id_salt("ls_launch_preview")
            .max_height(130.0)
            .auto_shrink([false, true])
            .show(ui, |ui| match &d.preview {
                Ok(LaunchPreview { display, notes, .. }) => {
                    mono_box(ui, display, pal.text, pal);
                    for n in notes {
                        ui.label(RichText::new(n.as_str()).size(12.0).color(pal.fav));
                    }
                    true
                }
                Err(e) => {
                    mono_box(ui, e, pal.danger, pal);
                    false
                }
            })
            .inner;
        ui.add_space(14.0);
        keep = launch_buttons(ui, st, d, pal, ok);
    });
    keep && !close
}

/// The launch options the game's store / launcher sets itself (read-only), and copying them.
fn source_options(ui: &mut Ui, d: &mut LaunchDlg, pal: &Pal) {
    let Some(src) = &d.source_opts else {
        return;
    };
    field_label(ui, &format!("Set in {}", d.source.display()), pal);
    mono_box(ui, src, pal.dim, pal);
    let note = if d.source == Source::Steam {
        "Read-only here. Steam applies these itself whenever it starts the game; \
         Lodestar can only add arguments on top. To replace them (wrappers, \
         variables), use a custom command below."
            .to_string()
    } else {
        format!(
            "Read-only here. {} applies these itself unless you use a custom \
             command below.",
            d.source.display()
        )
    };
    hint_text(ui, &note, pal);
    // For Steam, copying only makes sense once Steam is bypassed.
    let can_copy = d.source != Source::Steam || d.use_custom;
    if can_copy
        && ui
            .small_button("Copy into Lodestar's options")
            .on_hover_text("Start from these and edit them here")
            .clicked()
    {
        d.options = src.clone();
    }
}

/// Lodestar's own launch options, explained for what the source lets them do.
fn own_options(ui: &mut Ui, d: &mut LaunchDlg, pal: &Pal) {
    let scope = d.options_scope();
    let placeholder = match scope {
        OptionsScope::Full => "MANGOHUD=1 gamemoderun %command% -dx11",
        OptionsScope::ArgsOnly => "-dx11 -skipintro   (arguments for the game)",
        OptionsScope::None => "Not available for Steam shortcuts",
    };
    ui.add(
        egui::TextEdit::singleline(&mut d.options)
            .font(egui::TextStyle::Monospace)
            .hint_text(RichText::new(placeholder).family(egui::FontFamily::Monospace).color(mix(pal.dim, pal.bg, 0.45)))
            .desired_width(f32::INFINITY)
            .margin(egui::Margin::symmetric(8, 6)),
    );
    let explain = match scope {
        OptionsScope::Full => {
            "%command% stands for the game's normal command. Words before it that look like VAR=value become \
             environment variables; other words before it wrap the game (gamemoderun, mangohud …). Anything after \
             it — or everything, when there is no %command% — is passed to the game as arguments."
        }
        OptionsScope::ArgsOnly => {
            "Steam starts this game, so everything here is passed to it as arguments. Wrappers and \
             variables (gamemoderun, MANGOHUD=1 …) go in Steam's own launch options — or use “Run without \
             Steam's launch options” below to start it from Lodestar with full control."
        }
        OptionsScope::None => {
            "Steam starts this shortcut and takes no extra options from Lodestar. Set them in Steam, or use \
             a custom command below."
        }
    };
    hint_text(ui, explain, pal);
}

/// "Use a custom command", the helpers that fill one in, and the command with its folder.
fn custom_command(ui: &mut Ui, st: &AppState, d: &mut LaunchDlg, pal: &Pal) {
    ui.checkbox(
        &mut d.use_custom,
        RichText::new("Use a custom command instead of the source's launch").strong().color(pal.text),
    );
    // Helpers that fill in a custom command (and tick the box).
    ui.horizontal_wrapped(|ui| {
        if d.source == Source::Faugus
            && ui
                .small_button("Run directly with umu-run (bypass Faugus)")
                .on_hover_text(
                    "Fills in the command Faugus would run, so the game starts from Lodestar \
                     and only the options set here apply",
                )
                .clicked()
        {
            d.fill_faugus_direct(&st.config);
        }
        if d.source == Source::Steam
            && ui
                .small_button("Run without Steam's launch options")
                .on_hover_text(
                    "Fills in the command Steam would run (the game's executable, through the \
                     same Proton version for Windows games), so only the options set here apply. \
                     Keep Steam running for games that need it.",
                )
                .clicked()
        {
            d.fill_steam_direct(&st.config);
        }
        if let Some((cmd, _)) = &d.base_command
            && ui.small_button("Edit the current command").on_hover_text(cmd.as_str()).clicked()
        {
            d.fill_base_command();
        }
    });
    if let Some(e) = &d.helper_error {
        ui.label(RichText::new(e).size(12.0).color(pal.danger));
    }
    if !d.use_custom {
        return;
    }
    ui.indent("ls_custom_cmd", |ui| {
        field_label(ui, "Custom command", pal);
        ui.add(
            egui::TextEdit::multiline(&mut d.command)
                .desired_rows(2)
                .font(egui::TextStyle::Monospace)
                .hint_text(
                    RichText::new("/path/to/game.x86_64 --flag   or   umu-run /path/Game.exe")
                        .family(egui::FontFamily::Monospace),
                )
                .desired_width(f32::INFINITY)
                .margin(egui::Margin::symmetric(8, 6)),
        );
        hint_text(
            ui,
            "Replaces how the source would start the game (quotes work like a shell; no shell is run). Launch \
             options above still apply around it.",
            pal,
        );
        field_label(ui, "Working folder (optional)", pal);
        ui.add(
            egui::TextEdit::singleline(&mut d.cwd)
                .font(egui::TextStyle::Monospace)
                .hint_text(RichText::new("default: the game's install folder").family(egui::FontFamily::Monospace))
                .desired_width(f32::INFINITY)
                .margin(egui::Margin::symmetric(8, 6)),
        );
    });
}

/// Save (when the preview is `ok`), Cancel and Clear. Returns whether the dialog stays open.
fn launch_buttons(ui: &mut Ui, st: &mut AppState, d: &mut LaunchDlg, pal: &Pal, ok: bool) -> bool {
    let mut keep = true;
    ui.horizontal(|ui| {
        if primary_button(ui, "Save", pal, ok).on_disabled_hover_text("Fix the problem above to save").clicked() {
            st.set_launch_override(d.id, Some(d.current()));
            save_game_options(st, d.id, d.opts.clone());
            keep = false;
        }
        if plain_button(ui, "Cancel").clicked() {
            keep = false;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let can_clear = d.had_override || d.had_opts || !d.current().is_empty() || !d.opts.is_default();
            if ui
                .add_enabled(
                    can_clear,
                    egui::Button::new(RichText::new("Clear").color(pal.danger)).min_size(vec2(80.0, 30.0)),
                )
                .on_hover_text("Remove Lodestar's launch customisation for this game (options and text)")
                .clicked()
            {
                st.set_launch_override(d.id, None);
                save_game_options(st, d.id, Default::default());
                keep = false;
            }
        });
    });
    keep
}

/// Store a game's options in its user edits (only when they changed).
fn save_game_options(st: &mut AppState, id: GameId, opts: crate::game_options::GameOptions) {
    let Some(g) = st.library.get(id) else { return };
    if g.edits.game_options == opts {
        return;
    }
    let mut e = g.edits.clone();
    e.game_options = opts;
    st.set_edits(id, e);
}

/// The switches part of the Launch options dialog: per-game game options with what the current
/// launch does with each.
fn game_options_section(ui: &mut Ui, st: &mut AppState, d: &mut LaunchDlg, pal: &Pal) {
    field_label(ui, "Game options", pal);
    let scope = d.options_scope();
    let (report, steam_string) = match &d.preview {
        Ok(p) => (p.report.clone(), p.steam_string.clone()),
        Err(_) => (vec![], None),
    };
    if scope == OptionsScope::Full {
        hint_text(
            ui,
            "Switch on what you want — Lodestar adds the right variables and wrappers in the right order. \
             Default follows Settings → Games.",
            pal,
        );
    } else {
        let paste = steam_string.filter(|s| s.trim() != "%command%" && !s.trim().is_empty());
        let msg = if scope == OptionsScope::None {
            "Steam starts this shortcut, so Lodestar can't add these. Set them in Steam, or use a custom command below."
        } else if paste.is_some() {
            "Steam starts this game, so these can't be added from Lodestar. Use “Run without Steam's launch options” below, \
             or paste this into the game's launch options in Steam:"
        } else {
            "Steam starts this game, so these can't be added from Lodestar directly. Use “Run without Steam's launch \
             options” below — or switch options on here and Lodestar shows the text to paste into Steam."
        };
        ui.label(RichText::new(msg).size(12.0).color(pal.fav));
        if let Some(s) = paste {
            ui.horizontal(|ui| {
                mono_box(ui, &s, pal.text, pal);
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(s.clone());
                }
            });
        }
    }
    let defaults = crate::game_options::defaults_for(&crate::game_options::launch_env(), d.source.key());
    if crate::game_options::launch_env().by_source.get(d.source.key()).is_some_and(|o| !o.is_default()) {
        hint_text(ui, &format!("Default includes what Settings → Games sets for {} games.", d.source.display()), pal);
    }
    let caps = st.game_opts.caps.clone();
    let cx = super::game_options_ui::EditorCtx {
        caps: st.game_opts.probed.then_some(&*caps),
        global: Some(&defaults),
        report: &report,
        salt: "game",
    };
    super::game_options_ui::options_editor(ui, &mut d.opts, &cx, pal);
}

// ------------------------------------------------------------------------------- rename

pub(crate) struct RenameDlg {
    id: GameId,
    text: String,
    original: String,
    focus: bool,
}

impl RenameDlg {
    pub fn new(g: &Game) -> RenameDlg {
        RenameDlg { id: g.id, text: g.display_name().to_string(), original: g.name.clone(), focus: true }
    }
}

fn rename_dialog(ctx: &egui::Context, st: &mut AppState, d: &mut RenameDlg, pal: &Pal, fam: &egui::FontFamily) -> bool {
    let mut keep = true;
    let (_, close) = modal(ctx, "ls_rename_dlg", pal, 420.0, |ui| {
        title(ui, "Rename", Some(&format!("Original name: {}", d.original)), pal, fam);
        let r = ui.add(
            egui::TextEdit::singleline(&mut d.text).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(8, 6)),
        );
        if d.focus {
            d.focus = false;
            r.request_focus();
            // Start with the whole name selected, so typing replaces it (like F2 elsewhere).
            let mut state = egui::widgets::text_edit::TextEditState::load(ui.ctx(), r.id).unwrap_or_default();
            state.cursor.set_char_range(Some(egui::text::CCursorRange::select_all_str(&d.text)));
            state.store(ui.ctx(), r.id);
        }
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if primary_button(ui, "Save", pal, !d.text.trim().is_empty()).clicked()
                || (enter && !d.text.trim().is_empty())
            {
                let name = if d.text.trim() == d.original { None } else { Some(d.text.clone()) };
                st.rename(d.id, name);
                keep = false;
            }
            if plain_button(ui, "Cancel").clicked() {
                keep = false;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Reset to original").clicked() {
                    st.rename(d.id, None);
                    keep = false;
                }
            });
        });
    });
    keep && !close
}

// ------------------------------------------------------------------------------- tags

pub(crate) struct TagsDlg {
    id: GameId,
    name: String,
    tags: Vec<String>,
    input: String,
    all: Option<Vec<String>>,
    focus: bool,
}

impl TagsDlg {
    pub fn new(g: &Game) -> TagsDlg {
        TagsDlg {
            id: g.id,
            name: g.display_name().to_string(),
            tags: g.tags.clone(),
            input: String::new(),
            all: None,
            focus: true,
        }
    }

    fn add_input(&mut self) {
        for t in self.input.split(',') {
            let t = t.trim();
            if !t.is_empty() && !self.tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                self.tags.push(t.to_string());
            }
        }
        self.input.clear();
    }
}

fn tags_dialog(ctx: &egui::Context, st: &mut AppState, d: &mut TagsDlg, pal: &Pal, fam: &egui::FontFamily) -> bool {
    if d.all.is_none() {
        d.all = Some(st.library.all_tags());
    }
    let mut keep = true;
    let (_, close) = modal(ctx, "ls_tags_dlg", pal, 460.0, |ui| {
        title(ui, "Edit tags", Some(&d.name), pal, fam);
        let mut remove = None;
        let mut save_on_enter = false;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            if d.tags.is_empty() {
                ui.label(RichText::new("No tags yet").color(pal.dim));
            }
            for (i, t) in d.tags.iter().enumerate() {
                let b = egui::Button::new(RichText::new(format!("{t}   ×")).size(13.0).color(pal.accent_text))
                    .fill(pal.accent)
                    .corner_radius(12);
                if ui.add(b).on_hover_text("Remove").clicked() {
                    remove = Some(i);
                }
            }
        });
        if let Some(i) = remove {
            d.tags.remove(i);
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut d.input)
                    .hint_text("New tag (Enter to add, commas for several)")
                    .desired_width(ui.available_width() - 70.0)
                    .margin(egui::Margin::symmetric(8, 6)),
            );
            if d.focus {
                d.focus = false;
                r.request_focus();
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                if d.input.trim().is_empty() {
                    // Enter on an empty field: done editing.
                    save_on_enter = true;
                } else {
                    d.add_input();
                    r.request_focus();
                }
            }
            if ui.add_enabled(!d.input.trim().is_empty(), egui::Button::new("Add")).clicked() {
                d.add_input();
            }
        });
        // Suggestions from tags used elsewhere.
        let q = d.input.trim().to_lowercase();
        let mut picked = None;
        if let Some(all) = &d.all {
            let sugg: Vec<&String> = all
                .iter()
                .filter(|t| {
                    !d.tags.iter().any(|x| x.eq_ignore_ascii_case(t)) && (q.is_empty() || t.to_lowercase().contains(&q))
                })
                .take(16)
                .collect();
            if !sugg.is_empty() {
                ui.add_space(6.0);
                hint_text(ui, "Existing tags:", pal);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    for t in sugg {
                        if ui.add(egui::Button::new(RichText::new(t.as_str()).size(12.5)).corner_radius(12)).clicked() {
                            picked = Some(t.clone());
                        }
                    }
                });
            }
        }
        if let Some(t) = picked {
            d.tags.push(t);
            d.input.clear();
        }
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if primary_button(ui, "Save", pal, true).clicked() || save_on_enter {
                if !d.input.trim().is_empty() {
                    d.add_input();
                }
                st.set_tags(d.id, d.tags.clone());
                keep = false;
            }
            if plain_button(ui, "Cancel").clicked() {
                keep = false;
            }
        });
    });
    keep && !close
}

// ------------------------------------------------------------------------------- remove / about

fn remove_dialog(ctx: &egui::Context, st: &mut AppState, id: GameId, pal: &Pal, fam: &egui::FontFamily) -> bool {
    let Some((name, manual)) = st.library.get(id).map(|g| (g.display_name().to_string(), g.source == Source::Manual))
    else {
        return false;
    };
    let mut keep = true;
    let (_, close) = modal(ctx, "ls_remove_dlg", pal, 400.0, |ui| {
        title(ui, "Remove game?", None, pal, fam);
        let text = if manual {
            format!("“{name}” will be removed from your Lodestar library. Its files are not touched.")
        } else {
            format!(
                "“{name}” will be removed from your library and stay out of it on rescans. Its files \
                 are not touched. To bring it back: Hidden in the sidebar, then right-click → Unhide."
            )
        };
        ui.label(RichText::new(text).color(pal.text));
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            let b = egui::Button::new(RichText::new("Remove").color(Color32::WHITE).strong())
                .fill(pal.danger)
                .min_size(vec2(90.0, 30.0));
            if ui.add(b).clicked() {
                st.remove_game(id);
                keep = false;
            }
            if plain_button(ui, "Cancel").clicked() {
                keep = false;
            }
        });
    });
    keep && !close
}

/// (keep it open, Credits clicked).
fn about_dialog(ctx: &egui::Context, st: &AppState, pal: &Pal, fam: &egui::FontFamily) -> (bool, bool) {
    let mut keep = true;
    let mut credits = false;
    let (_, close) = modal(ctx, "ls_about_dlg", pal, 400.0, |ui| {
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Lodestar").family(fam.clone()).size(26.0).strong().color(pal.text));
            ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(pal.dim));
            ui.add_space(8.0);
            ui.label(RichText::new("A fast game library and controller-mode frontend for Linux.").color(pal.text));
            ui.add_space(6.0);
            ui.label(
                RichText::new(format!("{} games in the library", st.library.games.len())).size(12.5).color(pal.dim),
            );
            ui.label(RichText::new(format!("Config: {}", st.paths.config_file().display())).size(11.5).color(pal.dim));
            ui.label(RichText::new(format!("Library: {}", st.paths.db_file().display())).size(11.5).color(pal.dim));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                // Centre the two buttons (vertical_centered only centres single widgets).
                let w = 80.0 * 2.0 + ui.spacing().item_spacing.x;
                ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
                if plain_button(ui, "Credits").on_hover_text("Thanks, data sources and open-source licences").clicked()
                {
                    credits = true;
                }
                if plain_button(ui, "Close").clicked() {
                    keep = false;
                }
            });
        });
    });
    (keep && !close, credits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_words() {
        assert!(is_env_word("MANGOHUD=1"));
        assert!(is_env_word("DXVK_HUD=fps,gpuload"));
        assert!(!is_env_word("=1"));
        assert!(!is_env_word("1A=2"));
        assert!(!is_env_word("--opt=1"));
        assert!(!is_env_word("gamemoderun"));
    }

    #[test]
    fn preview_follows_the_published_environment() {
        use crate::game_options::{Caps, LaunchEnv};
        use std::sync::Arc;
        let key = (LaunchOverride::default(), crate::game_options::GameOptions::default());
        let env = Arc::new(LaunchEnv::none());
        assert!(!preview_fresh(None, None, &key, &env), "never computed");
        assert!(preview_fresh(Some(&key), Some(&env), &key, &env));
        // The probe finished (or the monitor changed): a new env with the same defaults.
        let probed =
            Arc::new(LaunchEnv { caps: Arc::new(Caps { probed: true, ..Default::default() }), ..LaunchEnv::none() });
        assert!(!preview_fresh(Some(&key), Some(&env), &key, &probed), "caps changed");
        let moved = Arc::new(LaunchEnv { monitor: Some((2560, 1440)), ..LaunchEnv::none() });
        assert!(!preview_fresh(Some(&key), Some(&env), &key, &moved), "monitor changed");
        let mut edited = key.clone();
        edited.0.options = "%command% -dx11".into();
        assert!(!preview_fresh(Some(&key), Some(&env), &edited, &env), "edit changed");
    }

    #[test]
    fn launch_dialog_current_override() {
        let mut g = Game {
            id: 1,
            source: Source::Desktop,
            source_id: "x".into(),
            launch: Some(LaunchSpec::Command { program: "/bin/true".into(), args: vec![], cwd: None, env: vec![] }),
            source_launch_options: Some("  ".into()),
            ..Game::blank("X")
        };
        let mut d = LaunchDlg::new(&g);
        assert!(d.source_opts.is_none());
        assert!(d.current().is_empty());
        d.options = " MANGOHUD=1 %command% ".into();
        d.command = "/opt/x".into(); // ignored while the checkbox is off
        assert_eq!(d.current(), LaunchOverride { options: "MANGOHUD=1 %command%".into(), command: None, cwd: None });
        d.use_custom = true;
        d.cwd = "/opt".into();
        assert_eq!(d.current().command.as_deref(), Some("/opt/x"));
        assert_eq!(d.current().cwd, Some(PathBuf::from("/opt")));
        g.launch_override = Some(d.current());
        let d2 = LaunchDlg::new(&g);
        assert!(d2.use_custom && d2.had_override);
        assert_eq!(d2.current(), d.current());
    }
}
