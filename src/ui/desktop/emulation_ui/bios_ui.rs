//! A console's BIOS panel in Settings → Emulation: one plain sentence ("Ready in PCSX2 ✓",
//! "PS2 BIOS found in ~/Documents/bios files — not in PCSX2 yet"), a short help text and one
//! main button (the single thing to do, "Set up everything" when there are several, or
//! "Choose BIOS files…" when nothing was found). "Set up everything" asks once, listing what goes
//! where. Details shows every file, where it was found and the single actions.

use super::*;
use crate::emulation::bios;

/// A console's BIOS: the sentence, the help text, the main button, "Details" (every file and the
/// single actions) and the "Set up everything" confirmation.
pub(super) fn bios_panel(ui: &mut Ui, st: &mut AppState, p: &mut EmuPage, c: &Catalog, pf: &str, pal: &Pal) -> bool {
    let Some(info) = p.bios.data.as_ref().and_then(|d| d.by_platform.get(pf)).cloned() else {
        ui.horizontal(|ui| {
            lazy_spinner(ui, 14.0, pal.dim);
            ui.label(RichText::new("Checking the BIOS files…").size(12.5).color(pal.dim));
        });
        return false;
    };
    let home = st.paths.home.clone();
    let s = &info.status;
    let name = |id: &str| in_text(&c.name_by_id(id));
    let allowed = |id: &str| c.emu(id).is_some_and(|e| !e.emu.blocked);
    let sum = bios::summary(s, &info.plan, &home, &name, &allowed);
    let (icon, t) = match sum.mood {
        bios::Mood::Ready => (Icon::Check, ChipTone::Good),
        bios::Mood::Todo => (Icon::Warning, ChipTone::Warn),
        bios::Mood::Missing => (Icon::Close, ChipTone::Bad),
        bios::Mood::Info => (Icon::Close, ChipTone::Dim),
    };
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        paint_icon(ui.painter(), r.shrink(1.0), icon, tone(t, pal));
        // The icon already says "ready": no second tick in the words.
        let text = sum.text.replace(" ✓", "");
        ui.add(egui::Label::new(RichText::new(text).size(13.0).color(pal.text)).wrap());
    });
    if let Some(h) = &sum.help {
        hint_text(ui, h, pal);
    }
    ui.add_space(2.0);
    let busy = p.install.is_some();
    let details_open = p.bios_details.as_deref() == Some(pf);
    let picking = p.pick.is_some() || p.identify.is_some();
    ui.horizontal_wrapped(|ui| {
        if let Some(label) = &sum.button {
            if !info.plan.is_empty() {
                let btn = if sum.extra { egui::Button::new(label.as_str()) } else { primary_btn(label, pal) };
                if ui.add_enabled(!busy, btn).clicked() {
                    p.confirm = Some(Confirm { platform: pf.to_string(), emulator: String::new(), job: Job::Setup });
                }
            } else if ui
                .add_enabled(!picking, primary_btn(label, pal))
                .on_hover_text("Pick the files you have (or a .zip of them); Lodestar recognises them by their content")
                .clicked()
            {
                p.pick = Some((
                    PickFor::BiosFiles(pf.to_string()),
                    Pick::start(ui.ctx(), PickKind::Files, "Choose BIOS files", None),
                ));
            }
        }
        // Firmware consoles that aren't ready: the emulator's own installer is the manual way.
        if catalog::is_firmware_console(pf)
            && sum.mood != bios::Mood::Ready
            && let Some(e) = c.emulators_for(pf).into_iter().find(|e| !e.emu.blocked)
            && plain_button(ui, &super::settings_label(&in_text(&c.name_of(&e.emu))))
                .on_hover_text(format!(
                    "Opens {} itself, without a game, to install firmware from its menu",
                    e.emu.name
                ))
                .clicked()
        {
            open_emulator(st, &e.emu);
        }
        if plain_button(ui, if details_open { "Hide details" } else { "Details" })
            .on_hover_text("Every file, where it was found, and each action on its own")
            .clicked()
        {
            p.bios_details = if details_open { None } else { Some(pf.to_string()) };
        }
        if p.identify.as_ref().is_some_and(|(x, _, _)| x == pf)
            || p.install.as_ref().is_some_and(|(x, _, _, _)| x == pf)
        {
            lazy_spinner(ui, 14.0, pal.dim);
        }
    });
    if let Some((m, err)) = p.bios_msg.get(pf) {
        ui.label(RichText::new(m).size(12.0).color(if *err { pal.danger } else { pal.dim }));
    }
    if let Some(cf) = p.confirm.clone().filter(|cf| cf.platform == pf && cf.job == Job::Setup)
        && let Some(go) = setup_confirm(ui, c, &info, &home, pal)
    {
        p.confirm = None;
        if go {
            start_setup(st, p, c, &info);
        }
        let _ = cf;
    }
    if details_open {
        ui.add_space(4.0);
        details(ui, st, p, c, pf, pal, &info);
    }
    false
}

/// An emulator's list name for a sentence: "RPCS3 — Flatpak" → "RPCS3 (Flatpak)".
fn in_text(name: &str) -> String {
    match name.split_once(" — ") {
        Some((n, kind)) => format!("{n} ({kind})"),
        None => name.to_string(),
    }
}

/// A primary-looking button that can be disabled (`add_enabled`).
fn primary_btn(label: &str, pal: &Pal) -> egui::Button<'static> {
    egui::Button::new(RichText::new(label.to_string()).color(Color32::WHITE)).fill(pal.accent)
}

/// "Set up <console>?" listing every step. Some(true) = go, Some(false) = cancelled.
fn setup_confirm(ui: &mut Ui, c: &Catalog, info: &catalog::BiosInfo, home: &Path, pal: &Pal) -> Option<bool> {
    let mut out = None;
    let name = |id: &str| in_text(&c.name_by_id(id));
    confirm_frame(ui, pal, |ui| {
        let q = match info.plan.as_slice() {
            [one] => format!("{}?", bios::step_line(one, &name, home)),
            _ => format!("Set up {}?", catalog::console_name(&info.status.platform)),
        };
        ui.add(egui::Label::new(RichText::new(q).size(13.0).strong().color(pal.text)).wrap());
        if info.plan.len() > 1 {
            for s in &info.plan {
                hint_text(ui, &format!("• {}", bios::step_line(s, &name, home)), pal);
            }
        }
        let kept: Vec<String> = info.conflicts.keys().map(|e| name(e)).collect();
        if !kept.is_empty() {
            hint_text(
                ui,
                &format!("Files {} already has are kept (Details can replace them).", bios::join_and(&kept)),
                pal,
            );
        }
        if info.plan.iter().any(|s| matches!(s, bios::Step::Ps3Firmware { .. })) {
            hint_text(ui, "RPCS3 opens its own window while it installs the firmware; close it when it's done.", pal);
        }
        ui.add_space(4.0);
        let go = if info.plan.len() == 1 { "Go" } else { "Set up" };
        let go = match info.plan.as_slice() {
            [bios::Step::Copy { .. }] => "Copy",
            [bios::Step::SwitchFirmware { .. }] | [bios::Step::Ps3Firmware { .. }] => "Install",
            _ => go,
        };
        let f = button_row(ui, pal, Buttons::primary(go, true).cancel("Cancel"), |_| {});
        if f.primary || f.cancel {
            out = Some(f.primary);
        }
    });
    out
}

/// Run the confirmed plan: copies and Switch firmware on a helper thread, RPCS3's installer as
/// its own program.
fn start_setup(st: &mut AppState, p: &mut EmuPage, c: &Catalog, info: &catalog::BiosInfo) {
    let pf = info.status.platform.clone();
    if st.no_launch {
        p.bios_msg.insert(pf, ("Set up skipped (screenshot run).".into(), false));
        return;
    }
    let home = st.paths.home.clone();
    let mut ps3 = None;
    for s in &info.plan {
        if let bios::Step::Ps3Firmware { emulator, pup, .. } = s
            && let Some(e) = c.emu(emulator)
        {
            let spec = bios::ps3_install_spec(&e.emu, &home, pup);
            run_detached(st, spec, &e.emu.name, &format!("Installing the PS3 firmware in {}", e.emu.name));
            ps3 = Some(e.emu.name.clone());
        }
    }
    let rest: Vec<bios::Step> =
        info.plan.iter().filter(|s| !matches!(s, bios::Step::Ps3Firmware { .. })).cloned().collect();
    if rest.is_empty() {
        if let Some(n) = ps3 {
            p.bios_msg.insert(
                pf,
                (
                    format!(
                        "{n} is installing the firmware in its own window. When it's done, check again (Scan now)."
                    ),
                    false,
                ),
            );
        }
        return;
    }
    let (tx, rx) = crossbeam_channel::bounded(1);
    let ctx = st.ctx().clone();
    let status = info.status.clone();
    let spawned = std::thread::Builder::new().name("bios-setup".into()).spawn(move || {
        let r = bios::run_setup(&status, &rest).map_err(|e| format!("{e:#}"));
        let _ = tx.send(r);
        ctx.request_repaint();
    });
    if spawned.is_ok() {
        p.install = Some((pf, String::new(), rx, Job::Setup));
    }
}

/// Every file and its state, the firmware found, and the single actions (Choose BIOS files…,
/// Copy to <emulator>, Install firmware into <emulator>, Replace …).
fn details(
    ui: &mut Ui,
    st: &mut AppState,
    p: &mut EmuPage,
    c: &Catalog,
    pf: &str,
    pal: &Pal,
    info: &catalog::BiosInfo,
) {
    let home = st.paths.home.clone();
    let s = &info.status;
    if s.files.is_empty() {
        hint_text(ui, "No BIOS files are needed.", pal);
    }
    // Where each emulator reads them from.
    for t in s.targets.iter().filter(|t| c.emu(&t.emulator).is_some()) {
        let n = c.name_by_id(&t.emulator);
        let at = short_path(&t.dir, &home, 70);
        let line = if pf == "sony_playstation3" {
            format!("{n} keeps its firmware in {at} — {}", if t.ready { "installed" } else { "none yet" })
        } else {
            format!("{n} reads them from {at} — {}", if t.ready { "has what it needs" } else { "missing files" })
        };
        hint_text(ui, &line, pal);
    }
    let packaged = |f: &BiosFile| {
        (f.file == "firmware" && s.packages.iter().any(|k| k.kind == PackageKind::SwitchFirmware))
            || (f.file == "PS3UPDAT.PUP" && s.packages.iter().any(|k| k.kind == PackageKind::Ps3Firmware))
    };
    // Firmware that was found is shown with its package below (found where, installed where).
    for (f, state, at) in s.files.iter().filter(|(f, st, _)| !(*st == BiosState::Missing && packaged(f))) {
        ui.horizontal(|ui| {
            let (icon, t) = match state {
                BiosState::Ok => (Icon::Check, ChipTone::Good),
                BiosState::Missing => (Icon::Close, if f.required { ChipTone::Bad } else { ChipTone::Dim }),
                BiosState::Wrong => (Icon::Warning, ChipTone::Warn),
                BiosState::Optional | BiosState::NotNeeded => (Icon::Close, ChipTone::Dim),
            };
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            paint_icon(ui.painter(), r.shrink(1.0), icon, tone(t, pal));
            // PS2 dumps have no fixed name: show the file found.
            let label = match (f.file, at) {
                ("", Some(at)) => at.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                ("", None) => "PS2 BIOS".to_string(),
                ("firmware", _) => "Firmware".to_string(),
                (name, _) => name.to_string(),
            };
            ui.label(RichText::new(label).monospace().size(12.5).color(pal.text));
            let mut what = catalog::state_word(*state).to_string();
            if !f.required && *state != BiosState::Ok {
                what = "Optional".into();
            }
            if let Some(r) = f.region {
                what.push_str(&format!(" · {r}"));
            }
            ui.label(RichText::new(what).size(12.0).color(tone(t, pal)));
            if let Some(at) = at {
                let text = crate::emulation::bios::describe(at, &home);
                ui.add(egui::Label::new(RichText::new(text).size(11.5).color(pal.dim)).truncate())
                    .on_hover_text(at.display().to_string());
            } else if !f.note.is_empty() {
                ui.add(egui::Label::new(RichText::new(f.note).size(11.5).color(pal.dim)).truncate());
            }
        });
    }
    // Firmware found on the computer, and where it is installed.
    for k in &s.packages {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
            paint_icon(ui.painter(), r.shrink(1.0), Icon::Check, tone(ChipTone::Good, pal));
            ui.label(RichText::new(k.label()).size(12.5).color(pal.text));
            let mut where_ = format!("found in {}", crate::emulation::bios::describe(&k.path, &home));
            if k.kind == PackageKind::SwitchFirmware {
                where_.push_str(&format!(" · {} files", k.files));
            }
            ui.add(egui::Label::new(RichText::new(where_).size(11.5).color(pal.dim)).truncate())
                .on_hover_text(k.path.display().to_string());
        });
        for t in &k.targets {
            let name = c.name_by_id(&t.emulator);
            let text = match &t.installed {
                Some(v) if v == "installed" => format!("Installed in {name}"),
                Some(v) if v == "other" => format!("{name} has a different firmware"),
                Some(v) => format!("{} installed in {name}", v.trim_start_matches('0')),
                None => format!("Not installed in {name} yet"),
            };
            let tn = if t.installed.is_none() || k.replaces(t) { ChipTone::Warn } else { ChipTone::Good };
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                ui.label(RichText::new(text).size(12.0).color(tone(tn, pal)));
            });
        }
    }
    if !s.note.is_empty() {
        hint_text(ui, &s.note, pal);
    }
    ui.add_space(2.0);
    let busy = p.install.is_some();
    ui.horizontal_wrapped(|ui| {
        let picking = p.pick.is_some() || p.identify.is_some();
        if ui
            .add_enabled(
                !picking,
                egui::Button::new(if pf == "sony_playstation3" {
                    "Choose firmware file…"
                } else {
                    "Choose BIOS files…"
                }),
            )
            .on_hover_text(
                "Pick the BIOS files you have (or a .zip of them); Lodestar recognises them by their content",
            )
            .clicked()
        {
            p.pick = Some((
                PickFor::BiosFiles(pf.to_string()),
                Pick::start(ui.ctx(), PickKind::Files, "Choose BIOS files", None),
            ));
        }
        if p.identify.as_ref().is_some_and(|(x, _, _)| x == pf) {
            lazy_spinner(ui, 14.0, pal.dim);
        }
        for t in s.targets.iter().filter(|t| !t.missing.is_empty()) {
            let name = c.name_by_id(&t.emulator);
            let label =
                if pf == "nintendo_switch" { format!("Copy keys to {name}") } else { format!("Copy to {name}") };
            if ui
                .add_enabled(!busy, egui::Button::new(label))
                .on_hover_text(format!("Put the files {name} doesn't have yet where it looks for them"))
                .clicked()
            {
                p.confirm = Some(Confirm { platform: pf.to_string(), emulator: t.emulator.clone(), job: Job::Copy });
            }
        }
        for k in &s.packages {
            for t in k.targets.iter().filter(|t| t.installed.is_none() || k.replaces(t)) {
                let name = c.name_by_id(&t.emulator);
                let replace = t.installed.is_some();
                let (label, job, tip) = match k.kind {
                    PackageKind::SwitchFirmware if replace => (
                        format!("Replace firmware in {name}"),
                        Job::Firmware,
                        format!("Put {} into {name} instead of the firmware it has", k.label()),
                    ),
                    PackageKind::SwitchFirmware => (
                        format!("Install firmware into {name}"),
                        Job::Firmware,
                        format!("Copy the firmware files into {name}, like its own Tools → Install Firmware"),
                    ),
                    PackageKind::Ps3Firmware if replace => (
                        format!("Update firmware in {name}"),
                        Job::Ps3Firmware,
                        format!("{name} installs {} over the older one (its own window opens for a minute)", k.label()),
                    ),
                    PackageKind::Ps3Firmware => (
                        format!("Install into {name}"),
                        Job::Ps3Firmware,
                        format!("{name} installs it (its own window opens for a minute)"),
                    ),
                };
                if ui.add_enabled(!busy, egui::Button::new(label)).on_hover_text(tip).clicked() {
                    p.confirm = Some(Confirm { platform: pf.to_string(), emulator: t.emulator.clone(), job });
                }
            }
        }
    });
    if let Some(cf) = p.confirm.clone().filter(|cf| cf.platform == pf && cf.job != Job::Setup) {
        let name = c.name_by_id(&cf.emulator);
        match cf.job {
            Job::Copy => {
                if let Some(t) = s.targets.iter().find(|t| t.emulator == cf.emulator) {
                    let conflicts = info.conflicts.get(&t.emulator).cloned().unwrap_or_default();
                    let regional = s.files.iter().any(|(f, _, _)| f.region.is_some() && f.group.is_some());
                    if let Some(answer) = confirm_box(ui, &name, t, &conflicts, regional, &home, pal) {
                        p.confirm = None;
                        if let Some(ow) = answer {
                            start_install(st, p, c, info, &t.emulator, ow);
                        }
                    }
                }
            }
            Job::Setup => {}
            Job::Firmware | Job::Ps3Firmware => {
                let pkg = s.packages.iter().find(|k| {
                    k.kind
                        == if cf.job == Job::Firmware { PackageKind::SwitchFirmware } else { PackageKind::Ps3Firmware }
                });
                if let Some(k) = pkg
                    && let Some(t) = k.targets.iter().find(|t| t.emulator == cf.emulator)
                    && let Some(go) = firmware_confirm(ui, &name, k, t, &home, pal)
                {
                    p.confirm = None;
                    if go {
                        if cf.job == Job::Firmware {
                            start_firmware(st, p, info, &t.emulator, t.installed.is_some());
                        } else if let Some(e) = c.emu(&t.emulator) {
                            let spec = crate::emulation::bios::ps3_install_spec(&e.emu, &home, &k.path);
                            run_detached(
                                st,
                                spec,
                                &e.emu.name,
                                &format!("Installing the PS3 firmware in {}", e.emu.name),
                            );
                            p.bios_msg.insert(
                                pf.to_string(),
                                (
                                    format!(
                                        "{} is installing the firmware in its own window. When it's done, check again (Scan now)."
                                        , e.emu.name
                                    ),
                                    false,
                                ),
                            );
                        }
                    }
                }
            }
        }
    }
}

/// The "Copy to <emulator>" confirmation. Some(None) = cancelled, Some(Some(overwrite)) = go.
fn confirm_box(
    ui: &mut Ui,
    name: &str,
    t: &crate::emulation::bios::BiosTarget,
    conflicts: &[PathBuf],
    regional: bool,
    home: &Path,
    pal: &Pal,
) -> Option<Option<bool>> {
    let mut out = None;
    confirm_frame(ui, pal, |ui| {
        let files: Vec<String> =
            t.missing.iter().filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
        ui.label(
            RichText::new(format!(
                "Copy {} to {name}?",
                if files.len() == 1 { files[0].clone() } else { format!("{} files", files.len()) }
            ))
            .size(13.0)
            .strong()
            .color(pal.text),
        );
        if files.len() > 1 {
            hint_text(ui, &catalog::join_limited(&files, 6), pal);
        }
        if regional && files.len() > 1 {
            hint_text(ui, "One file per region, not every copy found.", pal);
        }
        hint_text(ui, &format!("Into {}", short_path(&t.dir, home, 70)), pal);
        let replace = !conflicts.is_empty();
        if replace {
            let names: Vec<String> =
                conflicts.iter().filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned())).collect();
            ui.label(
                RichText::new(format!(
                    "{name} already has a different {}. Replace {}?",
                    catalog::join_limited(&names, 3),
                    if names.len() == 1 { "it" } else { "them" }
                ))
                .size(12.0)
                .color(tone(ChipTone::Warn, pal)),
            );
        }
        ui.add_space(4.0);
        let f = button_row(
            ui,
            pal,
            Buttons::primary(if replace { "Replace" } else { "Copy" }, true).cancel("Cancel"),
            |ui| {
                if replace && conflicts.len() < t.missing.len() && plain_button(ui, "Copy only the new ones").clicked()
                {
                    out = Some(Some(false));
                }
            },
        );
        if f.primary {
            out = Some(Some(replace));
        }
        if f.cancel {
            out = Some(None);
        }
    });
    out
}

/// "Install firmware into <emulator>?" Some(true) = go, Some(false) = cancelled.
fn firmware_confirm(
    ui: &mut Ui,
    name: &str,
    k: &crate::emulation::bios::Package,
    t: &crate::emulation::bios::PackageTarget,
    home: &Path,
    pal: &Pal,
) -> Option<bool> {
    let mut out = None;
    confirm_frame(ui, pal, |ui| {
        let replace = t.installed.is_some();
        let q = if replace {
            format!("Replace the firmware in {name} with {}?", k.label())
        } else {
            format!("Install {} into {name}?", k.label())
        };
        ui.label(RichText::new(q).size(13.0).strong().color(pal.text));
        hint_text(ui, &format!("From {}", crate::emulation::bios::describe(&k.path, home)), pal);
        match k.kind {
            PackageKind::SwitchFirmware => {
                hint_text(ui, &format!("Copies its {} files into {}.", k.files, short_path(&t.dir, home, 70)), pal);
                if replace {
                    hint_text(
                        ui,
                        &format!("The firmware files {name} has now are removed once the new ones are in."),
                        pal,
                    );
                }
            }
            PackageKind::Ps3Firmware => {
                hint_text(
                    ui,
                    &format!("{name} opens its own window while it installs the firmware; close it when it's done."),
                    pal,
                );
            }
        }
        ui.add_space(4.0);
        let f = button_row(
            ui,
            pal,
            Buttons::primary(if replace { "Replace" } else { "Install" }, true).cancel("Cancel"),
            |_| {},
        );
        if f.primary || f.cancel {
            out = Some(f.primary);
        }
    });
    out
}

fn confirm_frame(ui: &mut Ui, pal: &Pal, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(mix(pal.surface, pal.bg, 0.4))
        .stroke(egui::Stroke::new(1.0, with_alpha(pal.border, 200)))
        .corner_radius(6)
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

fn start_install(
    st: &AppState,
    p: &mut EmuPage,
    c: &Catalog,
    info: &catalog::BiosInfo,
    emulator: &str,
    overwrite: bool,
) {
    if st.no_launch {
        p.bios_msg.insert(info.status.platform.clone(), ("Copy skipped (screenshot run).".into(), false));
        return;
    }
    let (tx, rx) = crossbeam_channel::bounded(1);
    let ctx = st.ctx().clone();
    let reg = c.reg.clone();
    let status = info.status.clone();
    let emu = emulator.to_string();
    let spawned = std::thread::Builder::new().name("bios-install".into()).spawn(move || {
        let r = crate::emulation::bios::install(&reg, &status, &emu, overwrite).map_err(|e| format!("{e:#}"));
        let _ = tx.send(r);
        ctx.request_repaint();
    });
    if spawned.is_ok() {
        p.install = Some((info.status.platform.clone(), emulator.to_string(), rx, Job::Copy));
    }
}

/// Copy Switch firmware into an emulator (a helper thread; the user confirmed).
fn start_firmware(st: &AppState, p: &mut EmuPage, info: &catalog::BiosInfo, emulator: &str, replace: bool) {
    if st.no_launch {
        p.bios_msg.insert(info.status.platform.clone(), ("Install skipped (screenshot run).".into(), false));
        return;
    }
    let (tx, rx) = crossbeam_channel::bounded(1);
    let ctx = st.ctx().clone();
    let status = info.status.clone();
    let emu = emulator.to_string();
    let spawned = std::thread::Builder::new().name("firmware-install".into()).spawn(move || {
        let r = crate::emulation::bios::install_firmware(&status, &emu, replace).map_err(|e| format!("{e:#}"));
        let _ = tx.send(r);
        ctx.request_repaint();
    });
    if spawned.is_ok() {
        p.install = Some((info.status.platform.clone(), emulator.to_string(), rx, Job::Firmware));
    }
}
