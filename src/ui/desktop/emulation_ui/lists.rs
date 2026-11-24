//! Settings → Emulation, lower half: the emulators (one row per emulator with the consoles it
//! runs, their games and BIOS, <Emulator> settings…, Start options, ⋯ menu, profiles, arguments, its
//! other copies; Add emulator…). The row's panels are in emulator_panels.rs, the Add emulator form
//! in emulator_add.rs, the emulator / ROM / BIOS folders in folders.rs.

use super::catalog::{self, CatEmu, Catalog};
use super::emulator_add::*;
use super::emulator_panels::*;
use super::*;
use crate::emulation::defs;
use crate::ui::widgets::icon_button;

pub(super) const ARGS_HELP: &str = "{rom} the game file · {rom_dir} its folder · {rom_name} its name without extension · {core} the RetroArch core · {args} where extra arguments and switches go";

// ------------------------------------------------------------------------- emulators

/// Returns (config changed, scan settings changed).
pub(super) fn emulators(
    ui: &mut Ui,
    st: &mut AppState,
    p: &mut EmuPage,
    cat: Option<&Catalog>,
    pal: &Pal,
) -> (bool, bool) {
    let mut ch = false;
    group_title_at(ui, p, "emulators", "EMULATORS", pal);
    ui.add_space(4.0);
    card(ui, pal, |ui| {
        let mut first = true;
        if let Some(dir) = suggestion(&st.config.emulation) {
            first = false;
            ch |= suggestion_row(ui, st, &dir, pal);
        }
        let mut sep = |ui: &mut Ui| {
            if !std::mem::take(&mut first) {
                row_separator(ui, pal);
            }
        };
        match cat {
            None => {
                sep(ui);
                empty_row(ui, "Looking for emulators…", p.demo.is_none(), pal);
            }
            Some(c) if c.groups.is_empty() => {
                sep(ui);
                empty_row(
                    ui,
                    "No emulators yet. Get one on Flathub (Needs an emulator, above, links to it), or add yours here.",
                    false,
                    pal,
                );
            }
            Some(c) => {
                for gi in group_order(c, &p.facts) {
                    sep(ui);
                    ch |= emulator_group(ui, st, p, c, gi, pal);
                }
            }
        }
        ch |= hidden_line(ui, st, pal);
        if p.add_emu.is_some() {
            row_separator(ui, pal);
            indented(ui, |ui| {
                ui.add_space(8.0);
                ch |= add_form(ui, st, p, cat, pal);
            });
        } else {
            footer_button(ui, "Add emulator…", "Any emulator Lodestar didn't find, or your own command", pal, || {
                p.add_emu = Some(AddEmulator::default());
            });
        }
    });
    (ch, false)
}

/// The one "Found emulators in <folder>" suggestion to show (a folder named "Emulators" on a
/// drive that isn't an emulator folder yet and wasn't dismissed).
pub(super) fn suggestion(cfg: &crate::config::EmulationConfig) -> Option<PathBuf> {
    crate::emulation::registry::suggested_dirs().into_iter().find(|d| {
        cfg.auto_emulators
            && !cfg.emulator_dirs.iter().any(|x| d.starts_with(x))
            && !cfg.dismissed_emulator_dirs.contains(d)
    })
}

/// "Found emulators in /mnt/…/Emulators — Add this folder" (and × to dismiss it).
fn suggestion_row(ui: &mut Ui, st: &mut AppState, dir: &Path, pal: &Pal) -> bool {
    let mut add = false;
    let mut dismiss = false;
    let title = format!("Found emulators in {}", short_path(dir, &st.paths.home, 60));
    let sub = [("Not used yet · old copies in it are grouped under each emulator".to_string(), pal.dim)];
    row(ui, pal, &title, &sub, |ui| {
        if icon_button(ui, Icon::Close, false, 26.0, "Don't suggest this folder again").clicked() {
            dismiss = true;
        }
        if primary_button(ui, "Add this folder", pal, true).on_hover_text(dir.display().to_string()).clicked() {
            add = true;
        }
    });
    let e = &mut st.config.emulation;
    if add && !e.emulator_dirs.iter().any(|x| x == dir) {
        e.emulator_dirs.push(dir.to_path_buf());
        return true;
    }
    if dismiss && !e.dismissed_emulator_dirs.iter().any(|x| x == dir) {
        e.dismissed_emulator_dirs.push(dir.to_path_buf());
        return true;
    }
    false
}

pub(super) fn footer_button(ui: &mut Ui, text: &str, tip: &str, pal: &Pal, clicked: impl FnOnce()) {
    row_separator(ui, pal);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        if ui.button(text).on_hover_text(tip).clicked() {
            clicked();
        }
    });
    ui.add_space(6.0);
}

/// The consoles an emulator runs (any of its profiles), by name.
fn group_consoles(e: &CatEmu) -> Vec<String> {
    let mut v: Vec<String> = e.profiles.iter().flat_map(|p| p.platforms.iter().cloned()).collect();
    v.sort_by_cached_key(|id| catalog::console_name(id).to_lowercase());
    v.dedup();
    v
}

/// The order of the emulator rows: those whose consoles have games first, then by name.
pub(super) fn group_order(c: &Catalog, facts: &LibFacts) -> Vec<usize> {
    let mut v: Vec<usize> = (0..c.groups.len()).collect();
    v.sort_by_cached_key(|&gi| {
        let e = &c.emus[c.groups[gi].main];
        let games = group_consoles(e).iter().any(|pf| facts.per_platform.contains_key(pf));
        (!games, e.emu.name.to_lowercase(), gi)
    });
    v
}

/// One emulator row: its name and kind, where it is, <Name> settings…, Start options, ⋯; the consoles
/// it runs with their games and BIOS; its other copies (collapsed); the panels opened from it.
fn emulator_group(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, c: &Catalog, gi: usize, pal: &Pal) -> bool {
    let g = c.groups[gi].clone();
    let e = &c.emus[g.main];
    let members: Vec<&str> =
        std::iter::once(g.main).chain(g.copies.iter().copied()).map(|i| c.emus[i].emu.id.as_str()).collect();
    let mut ch = emulator_row(ui, st, p, c, e, pal);
    // ---- the consoles it runs
    let consoles = group_consoles(e);
    let (with, without): (Vec<&String>, Vec<&String>) =
        consoles.iter().partition(|pf| p.facts.per_platform.contains_key(*pf));
    let used_here = |pf: &str| c.uses(pf).is_some_and(|(id, _)| members.contains(&id.as_str()));
    let mut toggle_opts: Option<String> = None;
    let mut toggle_bios: Option<String> = None;
    egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 14, top: 0, bottom: 8 }).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            if with.is_empty() {
                let runs =
                    catalog::join_limited(&consoles.iter().map(|x| catalog::short_name(x)).collect::<Vec<_>>(), 6);
                let text =
                    if runs.is_empty() { "No games yet".to_string() } else { format!("No games yet · runs {runs}") };
                ui.label(RichText::new(text).size(12.0).color(pal.dim));
                return;
            }
            for pf in &with {
                let n = p.facts.per_platform.get(*pf).copied().unwrap_or(0);
                let text = format!("{} · {}", catalog::short_name(pf), plural(n, "game", "games"));
                if !used_here(pf) {
                    let other = c.uses(pf).map(|(x, _)| c.name_by_id(&x)).unwrap_or_default();
                    ui.label(RichText::new(format!("{text} · with {other}")).size(12.0).color(pal.dim)).on_hover_text(
                        format!(
                            "{} games start with {other} (Consoles with a choice, above)",
                            catalog::console_name(pf)
                        ),
                    );
                    continue;
                }
                let open = p.console_opts.as_deref() == Some(pf.as_str());
                if ui
                    .add(
                        egui::Button::new(RichText::new(text).size(12.0).color(pal.text))
                            .corner_radius(12)
                            .selected(open),
                    )
                    .on_hover_text(format!("Switches and extra arguments for every {} game", catalog::console_name(pf)))
                    .clicked()
                {
                    toggle_opts = Some(pf.to_string());
                }
                let info = p.bios.data.as_ref().and_then(|d| d.by_platform.get(*pf));
                if let Some(i) = info {
                    let chip = catalog::bios_chip_for(&i.status).or_else(|| {
                        catalog::is_firmware_console(pf).then(|| ("Keys & firmware".to_string(), ChipTone::Dim))
                    });
                    if let Some((t, tn)) = chip
                        && chip_button(ui, &t, tn, p.bios_open.as_deref() == Some(pf.as_str()), pal).clicked()
                    {
                        toggle_bios = Some(pf.to_string());
                    }
                } else if p.bios.running() && catalog::wants_check(pf) {
                    chip_button(ui, "Checking BIOS…", ChipTone::Dim, false, pal);
                }
            }
            if !without.is_empty() {
                let names: Vec<String> = without.iter().map(|x| catalog::console_name(x)).collect();
                ui.label(RichText::new(format!("+{} more", without.len())).size(12.0).color(pal.dim))
                    .on_hover_text(format!("Also runs (no games yet): {}", names.join(", ")));
            }
        });
    });
    if let Some(pf) = toggle_opts {
        p.console_opts = if p.console_opts.as_deref() == Some(pf.as_str()) { None } else { Some(pf) };
    }
    if let Some(pf) = toggle_bios {
        p.bios_open = if p.bios_open.as_deref() == Some(pf.as_str()) { None } else { Some(pf) };
    }
    // ---- its other copies
    if !g.copies.is_empty() {
        let id = e.emu.id.clone();
        let open = p.copies_open.contains(&id);
        let n = g.copies.len();
        let text = format!("{}  {} other {}", if open { "▾" } else { "▸" }, n, if n == 1 { "copy" } else { "copies" });
        egui::Frame::new().inner_margin(egui::Margin { left: 8, right: 14, top: 0, bottom: 6 }).show(ui, |ui| {
            let r = ui.add(egui::Button::new(RichText::new(text).size(12.5).color(pal.dim)).frame(false));
            if open && p.scroll_to.as_deref() == Some("copies") {
                r.scroll_to_me(Some(egui::Align::TOP));
            }
            if r.on_hover_text(format!(
                "{} installed again (an older AppImage, a backup). Lodestar uses the one above.",
                e.emu.name
            ))
            .clicked()
            {
                if open {
                    p.copies_open.remove(&id);
                } else {
                    p.copies_open.insert(id.clone());
                }
            }
        });
        if open {
            for &i in &g.copies {
                egui::Frame::new().inner_margin(egui::Margin { left: 12, right: 0, top: 0, bottom: 4 }).show(
                    ui,
                    |ui| {
                        ui.set_width(ui.available_width());
                        ch |= copy_row(ui, st, c, &c.emus[i], &with, pal);
                    },
                );
            }
        }
    }
    // ---- panels opened from it
    for pf in with.iter().filter(|pf| used_here(pf)) {
        if p.console_opts.as_deref() == Some(pf.as_str()) {
            indented(ui, |ui| ch |= super::console_options(ui, st, p, c, pf, pal));
        }
        if p.bios_open.as_deref() == Some(pf.as_str()) {
            indented(ui, |ui| ch |= bios_ui::bios_panel(ui, st, p, c, pf, pal));
        }
    }
    ch
}

/// One of an emulator's other copies: where it is, Use this copy, Remove.
fn copy_row(ui: &mut Ui, st: &mut AppState, c: &Catalog, e: &CatEmu, consoles: &[&String], pal: &Pal) -> bool {
    let home = st.paths.home.clone();
    // "DuckStation.backup/DuckStation.AppImage" (what tells the copies apart), its folder below.
    let (title, dir) = match e.emu.program_path() {
        Some(path) => {
            let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let parent = path.parent().unwrap_or(Path::new("/"));
            let pname = parent.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let title = if pname.is_empty() { file } else { format!("{pname}/{file}") };
            (title, folder_tail(parent.parent().unwrap_or(parent), &home))
        }
        None => (e.emu.flatpak_id().unwrap_or_default().to_string(), String::new()),
    };
    let mut sub: Vec<(String, Color32)> = vec![];
    if !dir.is_empty() {
        sub.push((format!("In {dir}"), pal.dim));
    }
    if e.emu.blocked {
        let sep = if sub.is_empty() { "" } else { " · " };
        sub.push((format!("{sep}not allowed to run yet"), tone(ChipTone::Warn, pal)));
    }
    let mut action = None;
    row_tagged(ui, pal, &title, Some(e.emu.kind_label()), &sub, |ui| {
        if ui.small_button("Remove").on_hover_text("Lodestar stops using this copy (it stays on the disk)").clicked() {
            action = Some("remove");
        }
        if !consoles.is_empty()
            && ui
                .small_button("Use this copy")
                .on_hover_text(format!(
                    "Games of {} start with this copy",
                    consoles.iter().map(|x| catalog::short_name(x)).collect::<Vec<_>>().join(", ")
                ))
                .clicked()
        {
            action = Some("use");
        }
    });
    let cfg = &mut st.config.emulation;
    match action {
        Some("remove") => {
            remove_emulator(cfg, &e.emu);
            true
        }
        Some("use") => {
            for pf in consoles {
                let auto = c.auto.get(pf.as_str()).map(|(x, _)| x.as_str());
                if auto == Some(e.emu.id.as_str()) {
                    cfg.platform_emulator.remove(pf.as_str());
                } else if let Some(pr) = e.profiles.iter().find(|x| x.platforms.iter().any(|q| q == *pf)) {
                    cfg.platform_emulator.insert(pf.to_string(), catalog::choice_value(&e.emu.id, &pr.id));
                }
            }
            true
        }
        _ => false,
    }
}

fn emulator_row(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, c: &Catalog, e: &CatEmu, pal: &Pal) -> bool {
    let mut ch = false;
    let id = e.emu.id.clone();
    let _ = c;
    let mut sub: Vec<(String, Color32)> = vec![];
    let kind = e.emu.kind_label();
    let (how, blocked) = match e.emu.how.split_once(" — not allowed to run yet") {
        Some((how, _)) => (how, true),
        None => (e.emu.how.as_str(), false),
    };
    let how = where_line(&e.emu, how, kind, &st.paths.home);
    if !how.is_empty() {
        sub.push((how, pal.dim));
    }
    if blocked {
        let sep = if sub.is_empty() { "" } else { " · " };
        sub.push((format!("{sep}not allowed to run yet"), tone(ChipTone::Warn, pal)));
    }
    let mut action: Option<&'static str> = None;
    row_tagged(ui, pal, &e.emu.name, Some(kind), &sub, |ui| {
        let more = icon_button(ui, Icon::More, false, 26.0, "More");
        egui::Popup::menu(&more).show(|ui| {
            ui.set_min_width(190.0);
            ui.menu_button("Change program", |ui| {
                if ui.button("A program or AppImage…").clicked() {
                    action = Some("file");
                }
                if ui.button("An installed Flatpak…").clicked() {
                    action = Some("flatpak");
                }
            });
            if ui.button("Profiles…").clicked() {
                action = Some("profiles");
            }
            if ui.button("Custom arguments…").clicked() {
                action = Some("args");
            }
            ui.separator();
            if ui.button("Remove").on_hover_text("Lodestar stops using it (the emulator stays installed)").clicked() {
                action = Some("remove");
            }
        });
        let opts_open = matches!(&p.emu_panel, Some((x, EmuPanel::Options)) if *x == id);
        if ui
            // "Start options", not "Options": the button beside it ("<Emulator> settings…")
            // opens the emulator's own settings, and two synonyms side by side didn't say
            // which one is Lodestar's.
            .add(egui::Button::new("Start options").selected(opts_open))
            .on_hover_text(format!(
                "How Lodestar starts games with {}: fullscreen, closing it when the game ends …",
                e.emu.name
            ))
            .clicked()
        {
            action = Some("options");
        }
        if e.emu.blocked {
            if ui
                .button("Allow it to run")
                .on_hover_text("This AppImage was downloaded but isn't marked as a program yet. Lodestar marks it (like chmod u+x) so it can start.")
                .clicked()
            {
                action = Some("allow");
            }
        } else if ui.button(settings_label(&e.emu.name)).on_hover_text(settings_tip(&e.emu.name)).clicked() {
            action = Some("open");
        }
    });
    match action {
        Some("open") => open_emulator(st, &e.emu),
        Some("allow") => allow_to_run(st, &e.emu),
        Some("file") => {
            p.pick = Some((
                PickFor::ChangeProgram(id.clone()),
                Pick::start(ui.ctx(), PickKind::File, &format!("Choose the program for {}", e.emu.name), None),
            ));
        }
        Some("flatpak") => open_flatpak_chooser(st, p, FlatpakFor::ChangeProgram(id.clone())),
        Some("profiles") => p.emu_panel = toggle(&p.emu_panel, &id, EmuPanel::Profiles),
        Some("options") => p.emu_panel = toggle(&p.emu_panel, &id, EmuPanel::Options),
        Some("args") => {
            p.emu_panel = toggle(&p.emu_panel, &id, EmuPanel::Args);
            p.args_text = Some((id.clone(), catalog::entry_for(&e.emu, &st.config.emulation).args));
        }
        Some("remove") => {
            remove_emulator(&mut st.config.emulation, &e.emu);
            ch = true;
        }
        _ => {}
    }
    match &p.emu_panel {
        Some((x, EmuPanel::Profiles)) if *x == id => indented(ui, |ui| ch |= profiles_panel(ui, st, p, e, pal)),
        Some((x, EmuPanel::Args)) if *x == id => indented(ui, |ui| ch |= args_panel(ui, st, p, e, pal)),
        Some((x, EmuPanel::Options)) if *x == id => indented(ui, |ui| ch |= options_panel(ui, st, p, e, pal)),
        _ => {}
    }
    ch
}

/// The line under an emulator's name: where it is ("In …/Emulators/DuckStation", the last two
/// folders: the rest is the same for every row) and whether it was added or changed by hand. The
/// tag beside the name says the kind, so it isn't repeated ("Flatpak" twice).
pub(super) fn where_line(e: &Emulator, how: &str, kind: &str, home: &Path) -> String {
    let how = if kind == "portable" { how.replace(" · portable", "") } else { how.to_string() };
    let by_hand = match how.as_str() {
        h if h.starts_with("Added by you") => Some("added by you"),
        h if h.starts_with("Changed by you") => Some("changed by you"),
        _ => None,
    };
    let dir = e.program_path().and_then(Path::parent);
    match (dir, by_hand) {
        (Some(d), Some(b)) => format!("In {} · {b}", folder_tail(d, home)),
        (Some(d), None) if how.contains(" in /") || how.contains(" in ~") => format!("In {}", folder_tail(d, home)),
        (None, Some(b)) => {
            let mut c = b.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
        _ => how_after_kind(&how, kind),
    }
}

/// A folder in few words: "~/Applications", "/opt/pcsx2", "…/Emulators/DuckStation" (deeper
/// folders: only the last two).
pub(super) fn folder_tail(d: &Path, home: &Path) -> String {
    let (prefix, rest) = match d.strip_prefix(home) {
        Ok(r) if !home.as_os_str().is_empty() => ("~/", r),
        _ => ("/", d.strip_prefix("/").unwrap_or(d)),
    };
    let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    match parts.len() {
        0 => prefix.trim_end_matches('/').to_string(),
        1..=3 => format!("{prefix}{}", parts.join("/")),
        n => format!("…/{}", parts[n - 2..].join("/")),
    }
}

/// How an emulator was found, without the kind its tag shows: "Flatpak" → "", "AppImage in
/// ~/Apps" → "In ~/Apps", "Added by you" → "", "Program in PATH (/usr/bin/x)" stays.
pub(super) fn how_after_kind(how: &str, kind: &str) -> String {
    let how = how.trim();
    if how.eq_ignore_ascii_case(kind) {
        return String::new();
    }
    match how.get(..kind.len()).filter(|h| h.eq_ignore_ascii_case(kind)).map(|_| how[kind.len()..].trim_start()) {
        Some(rest) if rest.starts_with("in ") => {
            let mut c = rest.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
        Some(rest) if rest.starts_with('(') => rest.trim_matches(|c| c == '(' || c == ')').to_string(),
        _ => how.to_string(),
    }
}

pub(super) fn toggle(cur: &Option<(String, EmuPanel)>, id: &str, what: EmuPanel) -> Option<(String, EmuPanel)> {
    match cur {
        Some((x, w)) if x == id && *w == what => None,
        _ => Some((id.to_string(), what)),
    }
}

/// "Allow it to run": mark the AppImage executable (the user clicked), then look again.
pub(in crate::ui::desktop) fn allow_to_run(st: &mut AppState, e: &Emulator) {
    let Some(path) = e.program_path().map(Path::to_path_buf) else { return };
    if st.no_launch {
        st.notify(format!("Skipped (screenshot run): allow {} to run", e.name));
        return;
    }
    match crate::emulation::detect::allow_to_run(&path) {
        Ok(()) => {
            st.notify(format!("{} can run now", e.name));
            crate::emulation::registry::mark_allowed(&e.id);
            st.refresh_emulation(true);
        }
        Err(err) => st.notify_error(format!("Couldn't allow {} to run: {err}", e.name)),
    }
}

/// Remove an emulator: a user entry is deleted (with its profiles and console choices); one
/// that was found by itself is hidden — also when the user had changed it (the override goes
/// and the found one stays away).
pub(super) fn remove_emulator(cfg: &mut crate::config::EmulationConfig, e: &Emulator) {
    cfg.emulators.remove(&e.id);
    let prefix = format!("{}/", e.id);
    cfg.profiles.retain(|k, _| !k.starts_with(&prefix));
    cfg.platform_emulator.retain(|_, v| catalog::split_choice(v).0 != e.id);
    if (e.auto || !e.user) && !cfg.hidden_emulators.contains(&e.id) {
        cfg.hidden_emulators.push(e.id.clone());
    }
}

/// Emulators the user removed: one quiet, folded line ("3 removed by you"). Opening it lists
/// them with "Use again": removed looks removed, with the way back one click away (a row of
/// restore chips would be clutter).
fn hidden_line(ui: &mut Ui, st: &mut AppState, pal: &Pal) -> bool {
    let hidden = st.config.emulation.hidden_emulators.clone();
    if hidden.is_empty() {
        return false;
    }
    let mut restore = None;
    row_separator(ui, pal);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        let title = format!("{} removed by you", hidden.len());
        egui::CollapsingHeader::new(RichText::new(title).size(12.0).color(pal.dim))
            .id_salt("ls_emu_removed")
            .default_open(false)
            .show(ui, |ui| {
                for h in &hidden {
                    let name = hidden_name(h);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&name).size(12.5).color(pal.text));
                        if ui
                            .small_button("Use again")
                            .on_hover_text(format!("Lodestar finds and uses {name} again"))
                            .clicked()
                        {
                            restore = Some(h.clone());
                        }
                    });
                }
            });
    });
    ui.add_space(2.0);
    if let Some(h) = restore {
        st.config.emulation.hidden_emulators.retain(|x| *x != h);
        return true;
    }
    false
}

/// A removed emulator's name from its id: "duckstation-appimage-2" → "DuckStation (AppImage)".
pub(super) fn hidden_name(id: &str) -> String {
    let mut cut = id.len();
    loop {
        if let Some(d) = defs::by_id(&id[..cut]) {
            let rest = id[cut..].trim_start_matches('-');
            let kind = match rest.split('-').next().unwrap_or("") {
                "flatpak" => Some("Flatpak"),
                "appimage" => Some("AppImage"),
                "program" => Some("program"),
                "steam" => Some("Steam"),
                _ => None,
            };
            // Detection numbers further copies of the same kind ("-2", "-3"): say so, or a
            // removed copy reads exactly like the one still in use above it.
            let copy =
                rest.rsplit('-').next().and_then(|n| n.parse::<u32>().ok()).filter(|&n| n >= 2).map(|n| match n {
                    2 => "second copy".to_string(),
                    3 => "third copy".to_string(),
                    4 => "fourth copy".to_string(),
                    n => format!("copy {n}"),
                });
            let parts: Vec<String> = kind.map(str::to_string).into_iter().chain(copy).collect();
            return if parts.is_empty() { d.name.to_string() } else { format!("{} ({})", d.name, parts.join(", ")) };
        }
        match id[..cut].rfind('-') {
            Some(i) => cut = i,
            None => return id.to_string(),
        }
    }
}
