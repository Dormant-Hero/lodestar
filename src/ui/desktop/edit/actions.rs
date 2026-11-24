//! The Edit Game Details Actions and Scripts tabs.

use super::*;

// ------------------------------------------------------------------------------- Actions

pub(super) fn kind_label(k: ActionKind) -> &'static str {
    match k {
        ActionKind::Command => "Program",
        ActionKind::Url => "Link / URI",
        ActionKind::File => "Open a file",
        ActionKind::Emulator => "Emulator",
    }
}

pub(super) fn tracking_label(t: TrackingMode) -> &'static str {
    match t {
        TrackingMode::Default => "Default",
        TrackingMode::Process => "Started process only",
        TrackingMode::Folder => "Any process in the install folder",
        TrackingMode::None => "Don't track",
    }
}

pub(super) fn tracking_tip(t: TrackingMode) -> &'static str {
    match t {
        TrackingMode::Default => "The started program and everything it starts",
        TrackingMode::Process => "Only the started program itself",
        TrackingMode::Folder => {
            "Any process running from the install folder: for launchers that start the real \
             game and quit"
        }
        TrackingMode::None => "Don't wait for it and don't count play time",
    }
}

pub(super) fn actions_tab(ui: &mut Ui, st: &mut AppState, d: &mut EditDlg, pal: &Pal) {
    play_summary(ui, d, pal);
    ui.add_space(6.0);
    let n = d.actions.len();
    let mut op: Option<(usize, i32)> = None;
    let mut pick: Option<(PickFor, PickKind)> = None;
    let cx = CardCtx { n, picking: d.picker.is_some(), emu: d.emu.clone(), id: d.id };
    for (i, a) in d.actions.iter_mut().enumerate() {
        let preview = d.previews.get(i).and_then(|p| p.as_ref()).map(|(_, p)| p);
        let out = action_card(ui, st, &cx, i, a, preview, pal);
        if let Some(o) = out.op {
            op = Some((i, o));
        }
        if out.pick.is_some() {
            pick = out.pick;
        }
        ui.add_space(2.0);
    }
    match op {
        Some((i, 0)) => {
            d.actions.remove(i);
        }
        Some((i, -1)) => d.actions.swap(i, i - 1),
        Some((i, _)) => d.actions.swap(i, i + 1),
        None => {}
    }
    if let Some((what, kind)) = pick {
        let title = if kind == PickKind::Folder { "Choose the working folder" } else { "Choose the program or file" };
        let start = Some(PathBuf::from(d.install_dir.text.trim())).filter(|p| p.is_absolute());
        d.picker = Some((what, Pick::start(ui.ctx(), kind, title, start)));
    }
    ui.add_space(4.0);
    let emulated = d.emu.emulated;
    if ui.button("Add action").clicked() {
        let first = d.actions.is_empty();
        d.actions.push(GameAction {
            name: if first && d.id.is_none() { "Play".into() } else { String::new() },
            // A new game's first action is what Play runs.
            play: first && d.id.is_none() && d.template.launch.is_none(),
            // Emulated games: the game's emulator (its choice now: automatic or its own).
            kind: if emulated { ActionKind::Emulator } else { ActionKind::Command },
            emulator: if emulated { d.emu.emulator.clone().filter(|s| !s.is_empty()) } else { None },
            profile: if emulated { d.emu.profile.clone().filter(|s| !s.is_empty()) } else { None },
            ..Default::default()
        });
    }
    ui.add_space(6.0);
    hint_text(ui, VARS_HELP, pal);
    if emulated {
        hint_text(ui, EMU_VARS_HELP, pal);
    }
}

/// The box on top: what Play does with these actions, and what they replace.
fn play_summary(ui: &mut Ui, d: &EditDlg, pal: &Pal) {
    let any_play = d.actions.iter().any(|a| a.play);
    let plays = d.actions.iter().filter(|a| a.play).count();
    egui::Frame::new()
        .fill(mix(pal.bg, pal.surface, 0.4))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let (title, col) = if plays > 1 {
                ("Play asks which of your Play actions to run (the first is the default)", pal.dim)
            } else if any_play {
                ("Play runs your Play action below", pal.dim)
            } else {
                ("Play starts the game the usual way", pal.text)
            };
            ui.label(RichText::new(title).size(13.0).strong().color(col));
            let how = match &d.source_launch {
                // With Play actions the store's own launch is no longer one of Play's choices:
                // say it's what they replace, or it reads as one of the options.
                Some(l) if any_play => Some(format!("Instead of: {l}")),
                Some(l) => Some(l.clone()),
                // Nothing to replace: no line under "Play runs your Play action below".
                None if any_play => None,
                None if d.id.is_none() || d.source == Source::Manual => {
                    Some("This game has no launch yet: add an action and tick “Play action”.".into())
                }
                None => Some(format!("{} doesn't say how to start it.", d.source.display())),
            };
            if let Some(how) = how {
                ui.add(egui::Label::new(RichText::new(how).size(12.0).monospace().color(pal.dim)).truncate());
            }
        });
}

/// What every action card needs from the dialog.
struct CardCtx {
    /// How many actions there are.
    n: usize,
    /// A file picker is open.
    picking: bool,
    emu: super::super::edit_emulation::EmuEdit,
    id: Option<GameId>,
}

/// What an action card's buttons asked for.
#[derive(Default)]
struct CardOut {
    /// Remove it (0) or move it up (−1) / down (+1).
    op: Option<i32>,
    /// A file / folder picker for one of its fields.
    pick: Option<(PickFor, PickKind)>,
}

/// One action: name, Play action, remove / move; type and tracking; program or URL, arguments,
/// working folder; what it will run.
fn action_card(
    ui: &mut Ui,
    st: &mut AppState,
    cx: &CardCtx,
    i: usize,
    a: &mut GameAction,
    preview: Option<&Result<String, String>>,
    pal: &Pal,
) -> CardOut {
    let mut out = CardOut::default();
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, if a.play { with_alpha(pal.accent, 150) } else { with_alpha(pal.border, 180) }))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                let w = ui.available_width();
                ui.add(te(&mut a.name, "Name (Play, Settings, Launcher …)", (w - 260.0).max(140.0)));
                ui.checkbox(&mut a.play, "Play action").on_hover_text(
                    "The Play button runs this one (instead of the source's launch). \
                     With several, Play asks which one.",
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    if icon_button(ui, Icon::Trash, false, 24.0, "Remove").clicked() {
                        out.op = Some(0);
                    }
                    ui.add_enabled_ui(i + 1 < cx.n, |ui| {
                        if icon_button(ui, Icon::ArrowDown, false, 24.0, "Move down").clicked() {
                            out.op = Some(1);
                        }
                    });
                    ui.add_enabled_ui(i > 0, |ui| {
                        if icon_button(ui, Icon::ArrowUp, false, 24.0, "Move up").clicked() {
                            out.op = Some(-1);
                        }
                    });
                });
            });
            kind_and_tracking(ui, i, a, cx.emu.emulated, pal);
            if a.kind == ActionKind::Emulator {
                super::super::edit_emulation::emulator_action(ui, st, &cx.emu, cx.id, i, a, pal);
            } else {
                path_row(ui, i, a, cx, pal, &mut out.pick);
            }
            if a.kind == ActionKind::Command {
                command_rows(ui, i, a, cx.picking, pal, &mut out.pick);
            }
            if let Some(p) = preview {
                form_row(ui, "Will run", pal, |ui, _| {
                    let (text, col) = match p {
                        Ok(t) => (t.as_str(), mix(pal.text, pal.dim, 0.35)),
                        Err(e) => (e.as_str(), pal.danger),
                    };
                    ui.add(egui::Label::new(RichText::new(text).size(12.0).monospace().color(col)).truncate())
                        .on_hover_text(text);
                });
            }
        });
    out
}

/// The Type and Tracking drop-downs.
fn kind_and_tracking(ui: &mut Ui, i: usize, a: &mut GameAction, emulated: bool, pal: &Pal) {
    form_row(ui, "Type", pal, |ui, _| {
        egui::ComboBox::from_id_salt(("ls_act_kind", i)).width(150.0).selected_text(kind_label(a.kind)).show_ui(
            ui,
            |ui| {
                // Emulated games: "Emulator" first (the game's emulator, ROM filled in).
                let kinds: &[ActionKind] = if emulated {
                    &[ActionKind::Emulator, ActionKind::Command, ActionKind::Url, ActionKind::File]
                } else {
                    &[ActionKind::Command, ActionKind::Url, ActionKind::File]
                };
                for k in kinds {
                    ui.selectable_value(&mut a.kind, *k, kind_label(*k)).on_hover_text(kind_tip(*k));
                }
            },
        );
        ui.label(RichText::new("Tracking").size(13.0).color(pal.dim));
        egui::ComboBox::from_id_salt(("ls_act_track", i))
            .width((ui.available_width() - 4.0).max(120.0))
            .selected_text(tracking_label(a.tracking))
            .show_ui(ui, |ui| {
                for t in [TrackingMode::Default, TrackingMode::Process, TrackingMode::Folder, TrackingMode::None] {
                    ui.selectable_value(&mut a.tracking, t, tracking_label(t)).on_hover_text(tracking_tip(t));
                }
            });
    });
}

/// The program, URL or file (with "…" to pick a file, except for URLs).
fn path_row(
    ui: &mut Ui,
    i: usize,
    a: &mut GameAction,
    cx: &CardCtx,
    pal: &Pal,
    pick: &mut Option<(PickFor, PickKind)>,
) {
    let path_label = match a.kind {
        ActionKind::Command | ActionKind::Emulator => "Program",
        ActionKind::Url => "URL",
        ActionKind::File => "File",
    };
    let path_hint = match a.kind {
        ActionKind::Command if cx.emu.emulated => "empty: the game's emulator",
        ActionKind::Command | ActionKind::Emulator => "{InstallDir}/game.x86_64  or  umu-run …",
        ActionKind::Url => "steam://…  heroic://…  https://…",
        ActionKind::File => "{InstallDir}/Readme.pdf",
    };
    form_row(ui, path_label, pal, |ui, w| {
        let bw = 30.0;
        let has_btn = a.kind != ActionKind::Url;
        let tw = if has_btn { w - bw - ui.spacing().item_spacing.x } else { w };
        ui.add(
            egui::TextEdit::singleline(&mut a.path)
                .font(egui::TextStyle::Monospace)
                .hint_text(RichText::new(path_hint).family(egui::FontFamily::Monospace))
                .desired_width(tw - 3.0)
                .margin(egui::Margin::symmetric(7, 4)),
        );
        if has_btn
            && ui
                .add_enabled_ui(!cx.picking, |ui| ui.add_sized([bw, 26.0], egui::Button::new("…")))
                .inner
                .on_hover_text("Choose a file")
                .clicked()
        {
            *pick = Some((PickFor::ActionPath(i), PickKind::File));
        }
    });
}

/// A program's arguments and working folder.
fn command_rows(
    ui: &mut Ui,
    i: usize,
    a: &mut GameAction,
    picking: bool,
    pal: &Pal,
    pick: &mut Option<(PickFor, PickKind)>,
) {
    form_row(ui, "Arguments", pal, |ui, w| {
        ui.add(
            egui::TextEdit::singleline(&mut a.args)
                .font(egui::TextStyle::Monospace)
                .hint_text(RichText::new("--fullscreen").family(egui::FontFamily::Monospace))
                .desired_width(w - 4.0)
                .margin(egui::Margin::symmetric(7, 4)),
        );
    });
    form_row(ui, "Working folder", pal, |ui, w| {
        let bw = 30.0;
        let cwd = a.cwd.get_or_insert_with(String::new);
        ui.add(
            egui::TextEdit::singleline(cwd)
                .font(egui::TextStyle::Monospace)
                .hint_text(RichText::new("default: the program's folder").family(egui::FontFamily::Monospace))
                .desired_width(w - bw - ui.spacing().item_spacing.x - 3.0)
                .margin(egui::Margin::symmetric(7, 4)),
        );
        if ui
            .add_enabled_ui(!picking, |ui| ui.add_sized([bw, 26.0], egui::Button::new("…")))
            .inner
            .on_hover_text("Choose a folder")
            .clicked()
        {
            *pick = Some((PickFor::ActionCwd(i), PickKind::Folder));
        }
    });
}

/// Emulated games: what actions can use besides `VARS_HELP`.
const EMU_VARS_HELP: &str = "For this emulated game also {Rom} (the game file), {RomDir} (its folder), {RomName} (its name \
     without extension), {Platform} (the console's id) and {Emulator} (the emulator's name). A Program action with no \
     program runs the game's emulator.";

fn kind_tip(k: ActionKind) -> &'static str {
    match k {
        ActionKind::Emulator => {
            "Starts this game with an emulator: pick it and its options, Lodestar adds the game file"
        }
        ActionKind::Command => "Runs a program with arguments",
        ActionKind::Url => "Opens a link (steam://, heroic://, https://…)",
        ActionKind::File => "Opens a file with its usual app",
    }
}

// ------------------------------------------------------------------------------- Scripts

/// A monospace script box with a label and a Test button. Returns (changed, test clicked).
pub(crate) fn script_box(ui: &mut Ui, label: &str, text: &mut String, pal: &Pal) -> (bool, bool) {
    let mut test = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(13.0).color(pal.text));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            test = ui
                .add_enabled(!text.trim().is_empty(), egui::Button::new("Test").small())
                .on_hover_text("Run it now (output and exit status show as a notification)")
                .clicked();
        });
    });
    let changed = ui
        .add(
            egui::TextEdit::multiline(text)
                .font(egui::TextStyle::Monospace)
                .desired_rows(3)
                .desired_width(ui.available_width() - 4.0)
                .margin(egui::Margin::symmetric(8, 6)),
        )
        .changed();
    ui.add_space(4.0);
    (changed, test)
}

pub(super) fn scripts_tab(ui: &mut Ui, st: &mut AppState, d: &mut EditDlg, pal: &Pal) {
    hint_text(ui, "Shell commands run with sh around this game, as you (never as root).", pal);
    ui.add_space(4.0);
    let mut test = None;
    let s = &mut d.scripts;
    for (label, text) in [
        ("Before starting", &mut s.before_start),
        ("After the game has started", &mut s.after_start),
        ("After the game exits", &mut s.after_exit),
    ] {
        if script_box(ui, label, text, pal).1 {
            test = Some(text.clone());
        }
    }
    let has_global = {
        let g = &st.config.scripts;
        !(g.before_start.trim().is_empty() && g.after_start.trim().is_empty() && g.after_exit.trim().is_empty())
    };
    ui.checkbox(&mut s.use_global, "Also run the global scripts").on_hover_text("Settings → General → Scripts");
    if !has_global {
        hint_text(ui, "No global scripts are set (Settings → General).", pal);
    }
    if let Some(t) = test {
        // With the game's LODESTAR_* variables and in its install folder, like a real launch.
        st.test_script_for(d.id, &t);
    }
}
