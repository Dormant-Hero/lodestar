//! The context menus of games: one game, several selected games, More, store actions,
//! completion status and controller support.

use super::*;

/// Right-click menu on a game that is part of a multi-selection (`ids`, in view order).
/// `cur`: the user's collection being shown.
pub(crate) fn multi_menu(
    ui: &mut egui::Ui,
    lib: &crate::library::Library,
    ids: &[GameId],
    cur: Option<u64>,
    pal: &Pal,
    actions: &mut Vec<Action>,
) {
    ui.set_min_width(230.0);
    let n = ids.len();
    ui.label(egui::RichText::new(format!("{n} games selected")).size(12.0).color(pal.dim));
    ui.separator();
    let hidden = ids.iter().filter(|&&id| lib.get(id).is_some_and(|g| g.hidden)).count();
    if hidden < n
        && ui.add(egui::Button::new(format!("Hide {}", games_word(n - hidden))).shortcut_text("Del")).clicked()
    {
        actions.push(Action::HideMany(ids.to_vec()));
    }
    if hidden > 0 && ui.button(format!("Unhide {}", games_word(hidden))).clicked() {
        actions.push(Action::UnhideMany(ids.to_vec()));
    }
    // Hand-added games are removed instead (they never come back by themselves).
    let scanned: Vec<GameId> =
        ids.iter().copied().filter(|&id| lib.get(id).is_some_and(|g| g.source != Source::Manual)).collect();
    if !scanned.is_empty()
        && ui
            .button(if scanned.len() == n {
                "Not games".to_string()
            } else {
                format!("Not games ({} of them)", scanned.len())
            })
            .on_hover_text(NOT_A_GAME_TIP)
            .clicked()
    {
        actions.push(Action::NotAGame(scanned));
    }
    let missing: Vec<GameId> =
        ids.iter().copied().filter(|&id| lib.get(id).is_some_and(|g| g.missing.is_some())).collect();
    if !missing.is_empty()
        && ui
            .button(if missing.len() == n {
                "Remove from library…".to_string()
            } else {
                format!("Remove {} from library…", missing_word(missing.len()))
            })
            .on_hover_text("These are no longer in the library they came from")
            .clicked()
    {
        actions.push(Action::RemoveMissing(missing));
    }
    ui.menu_button("Set completion status", |ui| completion_menu_many(ui, lib, ids, actions));
    score::score_row(ui, score::common_score(lib, ids), ids, pal, actions);
    explorer::colls::collection_menu(ui, &lib.user, ids, cur, actions);
    ui.separator();
    if ui.button(format!("Game options for {n} games…")).clicked() {
        actions.push(Action::BulkOptions(ids.to_vec()));
    }
    if ui.button(format!("Download artwork for {n} games…")).clicked() {
        actions.push(Action::BulkArt(artwork::BulkScope::Selection));
    }
    ui.separator();
    if ui.add(egui::Button::new("Clear selection").shortcut_text("Esc")).clicked() {
        actions.push(Action::ClearMulti);
    }
}

/// "1 missing game" / "3 missing games".
fn missing_word(n: usize) -> String {
    if n == 1 { "1 missing game".into() } else { format!("{n} missing games") }
}

/// "1 game" / "12 games".
pub(crate) fn games_word(n: usize) -> String {
    if n == 1 { "1 game".into() } else { format!("{n} games") }
}

pub(super) const NOT_A_GAME_TIP: &str = "Take it out of the library for good: rescans don't bring it back. \
     Settings → Libraries → Hidden & excluded shows it again.";

/// "Not a game ▸" with "Just this one" and "Everything named like “…”…" (scanned games only:
/// a hand-added game is removed instead).
pub(super) fn not_a_game_entries(ui: &mut egui::Ui, g: &Game, actions: &mut Vec<Action>) {
    if g.source == Source::Manual {
        return;
    }
    ui.menu_button("Not a game", |ui| not_a_game_menu(ui, g, actions));
}

/// The "Not a game ▸" submenu.
pub(super) fn not_a_game_menu(ui: &mut egui::Ui, g: &Game, actions: &mut Vec<Action>) {
    ui.set_min_width(200.0);
    if ui.button("Just this one").on_hover_text(NOT_A_GAME_TIP).clicked() {
        actions.push(Action::NotAGame(vec![g.id]));
    }
    let pattern = crate::exclude::suggest_pattern(&g.name);
    if ui
        .button(format!("Everything named like “{}”…", short_pattern(&pattern)))
        .on_hover_text("A name rule for things that keep coming back under new names (“Proton *”); you can change it before it is added")
        .clicked()
    {
        actions.push(Action::NotAGameLike(g.id));
    }
}

/// A pattern short enough for a menu entry.
fn short_pattern(p: &str) -> String {
    const MAX: usize = 36;
    if p.chars().count() <= MAX {
        p.to_string()
    } else {
        let mut s: String = p.chars().take(MAX - 1).collect();
        s.push('…');
        s
    }
}

/// Testing hook helper: show the submenu of `resp` open.
fn keep_submenu_open(ui: &mut egui::Ui, resp: &egui::Response) {
    use egui::containers::menu::{MenuState, SubMenu};
    let sub = SubMenu::id_from_widget_id(resp.id);
    MenuState::from_ui(ui, |s, _| s.open_item = Some(sub));
    ui.ctx().request_repaint();
}

/// The game's own extra actions ("Settings", "Launcher" …) as menu entries (the Play action
/// is what Play runs, so it isn't repeated). True when any was drawn.
pub(crate) fn action_entries(ui: &mut egui::Ui, g: &Game, enabled: bool, actions: &mut Vec<Action>) -> bool {
    let mut any = false;
    for (i, a) in g.edits.actions.iter().enumerate() {
        if a.play {
            continue;
        }
        any = true;
        let name = if a.name.trim().is_empty() { format!("Action {}", i + 1) } else { a.name.clone() };
        if ui.add_enabled(enabled, egui::Button::new(name)).clicked() {
            actions.push(Action::PlayExtra(g.id, i));
        }
    }
    any
}

/// Right-click menu for a game (shared by every view). `caps`: its store actions, when known
/// (they are for the selected game, which a right-click selects).
pub(crate) fn game_menu(
    ui: &mut egui::Ui,
    g: &Game,
    running: bool,
    caps: Option<&StoreCaps>,
    (lib, cur): (&crate::library::Library, Option<u64>),
    pal: &Pal,
    actions: &mut Vec<Action>,
) {
    ui.set_min_width(210.0);
    let can_install = caps.is_some_and(|c| c.can_install);
    let (play_label, enabled) = if running {
        ("Running", false)
    } else if g.installed {
        ("Play", true)
    } else if can_install && g.source == crate::model::Source::Romm {
        ("Download", true)
    } else if can_install && lib.not_in_steam_library(g) {
        // No longer in the Steam library: Install opens it in Steam (steam_owned.rs).
        ("Open in Steam", true)
    } else if can_install {
        ("Install", true)
    } else {
        ("Not installed", false)
    };
    if ui.add_enabled(enabled, egui::Button::new(play_label)).clicked() {
        actions.push(if g.installed { Action::Play(g.id) } else { Action::Install(g.id) });
    }
    super::emulation_ui::play_menu_entries(ui, g, g.installed && !running, actions);
    action_entries(ui, g, g.installed && !running, actions);
    ui.separator();
    let fav = if g.favorite { "Remove from favourites" } else { "Add to favourites" };
    if ui.button(fav).clicked() {
        actions.push(Action::ToggleFavorite(g.id));
    }
    explorer::colls::collection_menu(ui, &lib.user, &[g.id], cur, actions);
    if ui.button(if g.hidden { "Unhide" } else { "Hide" }).clicked() {
        actions.push(if g.hidden { Action::Unhide(g.id) } else { Action::Hide(g.id) });
    }
    not_a_game_entries(ui, g, actions);
    if g.emu.is_some() {
        let any = super::emulation_ui::addrom::has_target(lib, g);
        let hover = if any {
            "This file is another version or mode of a game you have: list it under that game".to_string()
        } else {
            let console =
                g.emu.as_ref().map_or_else(String::new, |e| super::emulation_ui::catalog::console_name(&e.platform));
            format!("No other {console} game in your library")
        };
        let r = ui.add_enabled(any, egui::Button::new("Add to another game as a ROM…"));
        if r.on_hover_text(&hover).on_disabled_hover_text(&hover).clicked() {
            actions.push(Action::AddAsRom(g.id));
        }
    }
    ui.separator();
    if ui.add(egui::Button::new("Edit…").shortcut_text("F3")).clicked() {
        actions.push(Action::Edit(g.id, edit::Tab::General));
    }
    if ui.add(egui::Button::new("Rename…").shortcut_text("F2")).clicked() {
        actions.push(Action::Rename(g.id));
    }
    if ui.button("Edit tags…").clicked() {
        actions.push(Action::EditTags(g.id));
    }
    if ui.button("Launch options…").clicked() {
        actions.push(Action::LaunchOptions(g.id));
    }
    ui.menu_button("Set completion status", |ui| completion_menu(ui, g, actions));
    score::score_row(ui, Some(g.edits.user_score), &[g.id], pal, actions);
    controller_submenu(ui, g, actions);
    if ui.button("Download metadata").clicked() {
        actions.push(Action::FetchMetadata(g.id));
    }
    ui.separator();
    if !g.art.is_empty() && ui.button("View artwork").clicked() {
        actions.push(Action::ViewArt(g.id, crate::art::ArtSlot::Cover));
    }
    if ui.button("Edit artwork…").clicked() {
        actions.push(Action::EditArt(g.id, crate::art::ArtSlot::Cover));
    }
    if ui.button("Download artwork…").clicked() {
        actions.push(Action::BulkArt(artwork::BulkScope::Game(g.id)));
    }
    ui.separator();
    page_entries(ui, g, caps, actions);
    store_entries(ui, g, caps, actions);
    // Hand-added games, and games no longer in their library (missing_ui.rs).
    if g.source == Source::Manual || g.missing.is_some() {
        ui.separator();
        let b = ui.button("Remove from library…");
        let b = match g.missing {
            Some(_) => {
                b.on_hover_text(format!("{}. Removes it from Lodestar; no files are touched.", g.source.missing_text()))
            }
            None => b,
        };
        if b.clicked() {
            actions.push(if g.missing.is_some() { Action::RemoveMissing(vec![g.id]) } else { Action::Remove(g.id) });
        }
    }
}

/// The game page's "•••" (More) menu.
pub(crate) fn more_menu(ui: &mut egui::Ui, g: &Game, caps: Option<&StoreCaps>, actions: &mut Vec<Action>) {
    ui.set_min_width(220.0);
    page_entries(ui, g, caps, actions);
    store_entries(ui, g, caps, actions);
    ui.separator();
    let fav = if g.favorite { "Remove from favourites" } else { "Add to favourites" };
    if ui.button(fav).clicked() {
        actions.push(Action::ToggleFavorite(g.id));
    }
    if ui.button(if g.hidden { "Unhide" } else { "Hide" }).clicked() {
        actions.push(if g.hidden { Action::Unhide(g.id) } else { Action::Hide(g.id) });
    }
    not_a_game_entries(ui, g, actions);
    ui.menu_button("Set completion status", |ui| completion_menu(ui, g, actions));
    controller_submenu(ui, g, actions);
    ui.separator();
    if ui.button("Launch options…").clicked() {
        actions.push(Action::LaunchOptions(g.id));
    }
    if ui.add(egui::Button::new("Edit…").shortcut_text("F3")).clicked() {
        actions.push(Action::Edit(g.id, edit::Tab::General));
    }
}

/// Links, Open install folder, Create desktop shortcut and Calculate install size: the top of the
/// game page's ••• menu, and in the right-click menu (which the controller's X opens) too. A
/// cloud game is never on disk: no folder entries at all rather than greyed ones.
fn page_entries(ui: &mut egui::Ui, g: &Game, caps: Option<&StoreCaps>, actions: &mut Vec<Action>) {
    let links = g.links();
    let r = ui.add_enabled_ui(!links.is_empty(), |ui| {
        ui.menu_button("Links", |ui| {
            ui.set_min_width(180.0);
            for l in &links {
                if ui.button(&l.name).on_hover_text(&l.url).clicked() {
                    actions.push(Action::OpenUrl(l.url.clone()));
                }
            }
        })
    });
    r.response.on_disabled_hover_text("No links yet. Add some in Edit… → Links.");
    let has_dir = g.install_dir.is_some();
    if !g.is_cloud()
        && ui
            .add_enabled(has_dir, egui::Button::new("Open install folder"))
            .on_disabled_hover_text("No install folder known. Set one in Edit… → Installation.")
            .clicked()
    {
        actions.push(Action::OpenFolder(g.id));
    }
    if ui
        .button("Create desktop shortcut")
        .on_hover_text("A launcher on your desktop that starts the game through Lodestar")
        .clicked()
    {
        actions.push(Action::CreateShortcut(g.id));
    }
    let sizing = caps.is_some_and(|c| c.sizing);
    let size_label = if sizing { "Calculating install size…" } else { "Calculate install size" };
    if !g.is_cloud()
        && ui
            .add_enabled(has_dir && !sizing, egui::Button::new(size_label))
            .on_disabled_hover_text(if sizing { "Adding up the install folder…" } else { "No install folder known" })
            .clicked()
    {
        actions.push(Action::CalcSize(g.id));
    }
}

/// "Store page" and "Uninstall" (only when the game's store offers them).
pub(super) fn store_entries(ui: &mut egui::Ui, g: &Game, caps: Option<&StoreCaps>, actions: &mut Vec<Action>) {
    let Some(c) = caps else { return };
    if let Some(url) = &c.store_page
        && ui.button(format!("Open {} store page", store_word(g.source))).on_hover_text(url).clicked()
    {
        actions.push(Action::OpenUrl(url.clone()));
    }
    if g.installed
        && c.can_uninstall
        && ui
            .button("Uninstall…")
            .on_hover_text(format!(
                "Opens {} to uninstall it (Lodestar never deletes game files itself)",
                uninstall_via(g.source)
            ))
            .clicked()
    {
        actions.push(Action::Uninstall(g.id));
    }
}

/// "Steam", "Epic", "GOG" … for "Open Steam store page".
pub(crate) fn store_word(s: Source) -> &'static str {
    match s {
        Source::Epic => "Epic",
        Source::Amazon => "Amazon",
        Source::Addon => "the",
        s => s.display(),
    }
}

/// Who does the uninstalling.
pub(super) fn uninstall_via(s: Source) -> &'static str {
    match s {
        Source::Epic | Source::Gog | Source::Amazon | Source::Heroic => "Heroic",
        Source::Addon => "the add-on",
        Source::BattleNet | Source::Ea | Source::Ubisoft | Source::Rockstar | Source::Riot | Source::HoYoPlay => {
            "the launcher"
        }
        s => s.display(),
    }
}

/// "Set completion status ▸" submenu.
pub(super) fn completion_menu(ui: &mut egui::Ui, g: &Game, actions: &mut Vec<Action>) {
    ui.set_min_width(160.0);
    let cur = g.completion();
    if ui.radio(cur.is_none(), "Not set").clicked() {
        actions.push(Action::SetCompletion(g.id, None));
    }
    ui.separator();
    for c in CompletionStatus::ALL {
        if ui.radio(cur == Some(c), c.label()).clicked() {
            actions.push(Action::SetCompletion(g.id, Some(c)));
        }
    }
    completion_hint(ui, actions);
}

/// "Set completion status ▸" for a multi-selection: one status for all of them (one Undo).
pub(super) fn completion_menu_many(
    ui: &mut egui::Ui,
    lib: &crate::library::Library,
    ids: &[GameId],
    actions: &mut Vec<Action>,
) {
    ui.set_min_width(160.0);
    // Marked only when every selected game has that status.
    let mut cur: Option<Option<CompletionStatus>> = None;
    let mut same = true;
    for g in ids.iter().filter_map(|id| lib.get(*id)) {
        match cur {
            None => cur = Some(g.completion()),
            Some(c) if c != g.completion() => same = false,
            _ => {}
        }
    }
    let cur = cur.filter(|_| same);
    if ui.radio(cur == Some(None), "Not set").clicked() {
        actions.push(Action::SetCompletionMany(ids.to_vec(), None));
    }
    ui.separator();
    for c in CompletionStatus::ALL {
        if ui.radio(cur == Some(Some(c)), c.label()).clicked() {
            actions.push(Action::SetCompletionMany(ids.to_vec(), Some(c)));
        }
    }
    completion_hint(ui, actions);
}

/// A quiet pointer to Settings → General "Set completion status to Playing when launching".
fn completion_hint(ui: &mut egui::Ui, actions: &mut Vec<Action>) {
    ui.separator();
    let dim = ui.visuals().weak_text_color();
    if ui
        .add(
            egui::Button::new(egui::RichText::new("Playing on launch: Settings → General…").size(11.5).color(dim))
                .frame(false),
        )
        .on_hover_text("Lodestar can set a game to Playing by itself when you start it")
        .clicked()
    {
        actions.push(Action::Settings(settings::Tab::General));
    }
}

/// "Controller support ▸" submenu (kept open by the `menu-controller` testing hook).
pub(super) fn controller_submenu(ui: &mut egui::Ui, g: &Game, actions: &mut Vec<Action>) {
    let ctl = ui.menu_button("Controller support", |ui| controller_menu(ui, g, actions));
    if DEBUG_CONTROLLER_SUBMENU.load(std::sync::atomic::Ordering::Relaxed) {
        // Testing hook (`LODESTAR_DESKTOP_OPEN=menu-controller`): show the submenu open.
        keep_submenu_open(ui, &ctl.response);
    }
}

/// Testing hook: keep the game menu's "Controller support" submenu open.
pub(super) static DEBUG_CONTROLLER_SUBMENU: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Short word for a controller-support level ("Full" / "Partial" / "None").
pub(crate) fn controller_word(c: ControllerSupport) -> &'static str {
    match c {
        ControllerSupport::Full => "Full",
        ControllerSupport::Partial => "Partial",
        ControllerSupport::None => "None",
    }
}

/// "Automatic (Steam says: Full)": the menu entry that follows the source's data.
pub(crate) fn controller_auto_label(g: &Game) -> String {
    match g.meta.controller {
        Some(c) if g.source == Source::Steam => {
            format!("Automatic (Steam says: {})", controller_word(c))
        }
        Some(c) => format!("Automatic ({})", controller_word(c)),
        None => "Automatic (not known)".into(),
    }
}

/// "Controller support ▸" submenu: follow the source, or the user's own setting.
pub(super) fn controller_menu(ui: &mut egui::Ui, g: &Game, actions: &mut Vec<Action>) {
    ui.set_min_width(210.0);
    let cur = g.controller_override;
    if ui.radio(cur.is_none(), controller_auto_label(g)).clicked() {
        actions.push(Action::SetController(g.id, None));
    }
    ui.separator();
    for c in [ControllerSupport::Full, ControllerSupport::Partial, ControllerSupport::None] {
        if ui.radio(cur == Some(c), controller_word(c)).clicked() {
            actions.push(Action::SetController(g.id, Some(c)));
        }
    }
}
