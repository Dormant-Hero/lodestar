//! Emulated games in controller mode: the quick menu's "Play in a window", "Play with…" (another
//! emulator / profile, once or from now on) and "<Emulator> settings…" (the emulator's own
//! window, no game: controls, graphics, firmware).

use super::menu::{Act, Item, Menu, MenuKind};
use crate::app::AppState;
use crate::emulation::detect::Emulator;
use crate::emulation::registry::{self, Registry};
use crate::model::{Game, GameId, LaunchSpec};

/// The emulator `g` starts with: the published choice (per game > per folder > per console >
/// first that runs it), else the one picked when the ROM was scanned.
pub(super) fn game_emulator(reg: &Registry, g: &Game) -> Option<Emulator> {
    let e = g.emu.as_ref()?;
    if let Some(x) = crate::emu_app::play_emulator(reg, g) {
        return Some(x);
    }
    let pick = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let (want_emu, want_profile) = (pick(&g.edits.emulator), pick(&g.edits.emulator_profile));
    let folder = reg.config.rom_folders.get(&e.folder);
    if let Some((emu, _)) =
        registry::choose(reg, &e.platform, &e.rom, want_emu.as_deref(), want_profile.as_deref(), folder)
    {
        return Some(emu);
    }
    let id = want_emu.as_deref().unwrap_or(&e.emulator);
    reg.emulators.iter().find(|x| !id.is_empty() && x.id == id).cloned()
}

/// Where a game comes from, for the menu subtitle and details line: an emulated game's console
/// ("Sony PlayStation 2") rather than "Emulators".
pub(super) fn source_label(g: &Game) -> String {
    match g.emu.as_ref().filter(|e| !e.platform.is_empty()) {
        Some(e) => crate::library::platform_label(&e.platform),
        None => g.source_name().to_string(),
    }
}

/// Quick-menu entries after Play for an emulated game (none for other games, or while no
/// emulator is found: Play then says what is missing).
pub(super) fn quick_items(reg: Option<&Registry>, g: &Game) -> Vec<Item> {
    let Some(emu) = reg.filter(|_| g.emu.is_some()).and_then(|r| game_emulator(r, g)) else {
        return vec![];
    };
    let mut v = vec![Item::action("Play in a window", Act::PlayWindowed(g.id))];
    if reg.is_some_and(|r| !crate::emu_app::choices(r, g).is_empty()) {
        v.push(Item::action("Play with…", Act::PlayWithMenu(g.id)));
    }
    let name = emu.name_among(reg.map(|r| r.emulators.as_slice()).unwrap_or_default());
    v.push(Item::action(format!("{name} settings…"), Act::OpenEmulator(g.id)));
    v
}

fn always_id(id: GameId) -> egui::Id {
    egui::Id::NULL.with(("ls_ctl_play_with_always", id))
}

/// Is "Always use this for this game" on in the Play with list?
pub(super) fn always(st: &AppState, id: GameId) -> bool {
    st.ctx().data(|d| d.get_temp::<bool>(always_id(id))).unwrap_or(false)
}

/// The Play with list: one line per emulator + profile (the current one says so), then the
/// "Always use this for this game" switch.
pub(super) fn play_with_items(st: &AppState, id: GameId) -> Vec<Item> {
    let Some(g) = st.library.get(id) else { return vec![] };
    let mut v: Vec<Item> = crate::emu_app::choices_now(g)
        .into_iter()
        .map(|c| Item::value(c.label, if c.current { "Current" } else { "" }, Act::PlayWith(id, c.emulator, c.profile)))
        .collect();
    if !v.is_empty() {
        v.push(Item::value(
            "Always use this for this game",
            if always(st, id) { "On" } else { "Off" },
            Act::ToggleAlwaysUse(id),
        ));
    }
    v
}

/// Act::PlayWithMenu.
pub(super) fn open_play_with(ui: &mut super::ControllerUi, st: &mut AppState, id: GameId, now: f64) {
    st.ctx().data_mut(|d| d.remove::<bool>(always_id(id)));
    let name = crate::ui::game_name(st, id);
    let items = play_with_items(st, id);
    if items.is_empty() {
        st.notify("No other emulator for this game");
        return;
    }
    ui.menus.push(Menu::new(MenuKind::PlayWith(id), "Play with", items, now).with_subtitle(name));
}

/// Act::ToggleAlwaysUse: flip the switch and redraw the list (the cursor stays).
pub(super) fn toggle_always(ui: &mut super::ControllerUi, st: &mut AppState, id: GameId) {
    let on = !always(st, id);
    st.ctx().data_mut(|d| d.insert_temp(always_id(id), on));
    let items = play_with_items(st, id);
    for m in ui.menus.iter_mut().filter(|m| m.kind == MenuKind::PlayWith(id)) {
        m.replace_items(items.clone());
    }
}

fn choose_always_id(id: GameId) -> egui::Id {
    egui::Id::NULL.with(("ls_ctl_choose_always", id))
}

/// Is "Always use this for this game" on in "Choose what to play"?
pub(super) fn choose_always(st: &AppState, id: GameId) -> bool {
    st.ctx().data(|d| d.get_temp::<bool>(choose_always_id(id))).unwrap_or(false)
}

fn choose_items(st: &AppState, id: GameId, entries: &[crate::emulation::roms::Entry]) -> Vec<Item> {
    let mut v: Vec<Item> =
        entries.iter().map(|e| Item::action(e.label.clone(), Act::PlayChoice(id, e.choice.clone()))).collect();
    v.push(Item::value(
        "Always use this for this game",
        if choose_always(st, id) { "On" } else { "Off" },
        Act::ToggleChooseAlways(id),
    ));
    v
}

/// Play (A on a game, the game menu's Play): start it, or, when it has several ways to play
/// and none is remembered, "Choose what to play" (A starts a line, B goes back).
pub(super) fn play_or_choose(ui: &mut super::ControllerUi, st: &mut AppState, id: GameId, now: f64) {
    ui.close_menus(st);
    if let Some(entries) = st.play_or_choose(id) {
        st.ctx().data_mut(|d| d.remove::<bool>(choose_always_id(id)));
        let name = crate::ui::game_name(st, id);
        let items = choose_items(st, id, &entries);
        ui.menus.push(Menu::new(MenuKind::ChoosePlay(id), "Choose what to play", items, now).with_subtitle(name));
    }
}

/// Act::ToggleChooseAlways: flip the switch and redraw the list (the cursor stays).
pub(super) fn toggle_choose_always(ui: &mut super::ControllerUi, st: &mut AppState, id: GameId) {
    let on = !choose_always(st, id);
    st.ctx().data_mut(|d| d.insert_temp(choose_always_id(id), on));
    let Some(g) = st.library.get(id) else { return };
    let items = choose_items(st, id, &crate::emu_app::play_entries_now(g));
    for m in ui.menus.iter_mut().filter(|m| m.kind == MenuKind::ChoosePlay(id)) {
        m.replace_items(items.clone());
    }
}

/// Act::OpenEmulator: start the game's emulator on its own, as a plain detached program (no
/// playtime, nothing tracked).
pub(super) fn open(st: &mut AppState, id: GameId) {
    let Some(g) = st.library.get(id).cloned() else { return };
    let Some(emu) = registry::current().and_then(|r| game_emulator(&r, &g)) else {
        let why = registry::missing_reason(&g).unwrap_or_else(|| "No emulator found for this game".into());
        st.notify_error(why);
        return;
    };
    if st.no_launch {
        log::warn!("screenshot run: not opening {}", emu.name);
        st.notify(format!("Open skipped (screenshot run): {}", emu.name));
        return;
    }
    let LaunchSpec::Command { program, args, cwd, env } = registry::open_spec(&emu) else {
        st.notify_error(format!("Couldn't open {}", emu.name));
        return;
    };
    let name = emu.name.clone();
    let spawned = std::thread::Builder::new().name("emulator-open".into()).spawn(move || {
        use std::os::unix::process::CommandExt as _;
        let mut cmd = std::process::Command::new(&program);
        cmd.args(&args)
            .envs(env)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .process_group(0);
        if let Some(d) = cwd.filter(|d| d.is_dir()) {
            cmd.current_dir(d);
        }
        match cmd.spawn() {
            // Reaped here so it never lingers as a zombie.
            Ok(mut child) => {
                let _ = child.wait();
            }
            Err(e) => log::warn!("open emulator {program}: {e}"),
        }
    });
    match spawned {
        Ok(_) => st.notify(format!("Opening {name}")),
        Err(e) => st.notify_error(format!("Couldn't open {name}: {e}")),
    }
}
