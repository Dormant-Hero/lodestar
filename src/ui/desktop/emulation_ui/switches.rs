//! The emulator switches as the pages show them (Settings → Emulation's emulator and console
//! options, Edit → Installation, an emulator action): a plain label with one line under it and
//! an On / Off switch (the emulator's own level) or Default / On / Off (a console, a game, an
//! action — Default says what it inherits). Extra arguments, the emulator's command-line page
//! and the less used switches sit behind a "More options" line, closed at first.

use super::super::dialogs::{hint_text, link_text};
use super::Pal;
use crate::app::AppState;
use crate::config::EmulationConfig;
use crate::emulation::detect::{Emulator, Profile};
use crate::emulation::options::{self, Level, Switch};
use crate::ui::widgets::toggle_switch;
use egui::{RichText, Ui};
use std::collections::BTreeMap;

/// Help for every "Extra arguments" box.
pub(crate) const EXTRA_HELP: &str = "Placed before the game file. {Rom}, {RomDir}, {RomName}, {Platform} and {Emulator} work; \
     the game file, -- and the emulator's folder access are added for you.";

/// "▸ More options" / "▾ More options": a line that opens what is under it. The state is kept
/// per `id` in egui's memory. Returns whether it is open.
pub(crate) fn more_options(ui: &mut Ui, id: egui::Id, pal: &Pal) -> bool {
    let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let text = if open { "▾  More options" } else { "▸  More options" };
    if ui
        .add(egui::Button::new(RichText::new(text).size(12.5).color(pal.dim)).frame(false))
        .on_hover_text("Extra arguments, the emulator's own command-line options and the rest of its switches")
        .clicked()
    {
        open = !open;
        ui.data_mut(|d| d.insert_temp(id, open));
    }
    open
}

/// Open "More options" from outside (screenshot hooks).
pub(crate) fn set_more_open(ctx: &egui::Context, id: egui::Id, open: bool) {
    ctx.data_mut(|d| d.insert_temp(id, open));
}

fn on_off(v: bool) -> &'static str {
    if v { "On" } else { "Off" }
}

fn label_block(ui: &mut Ui, sw: &Switch, pal: &Pal, w: f32) {
    ui.vertical(|ui| {
        ui.set_width(w);
        ui.label(RichText::new(&sw.label).size(13.0).color(pal.text));
        ui.label(RichText::new(&sw.help).size(11.5).color(pal.dim));
    });
}

/// Default / On / Off rows. `inherited(sw)` = what Default means. Only switches `show` keeps.
/// Writes `values` (absent = Default). True when something changed.
pub(crate) fn tri_rows(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug + Copy,
    sws: &[Switch],
    inherited: &dyn Fn(&Switch) -> bool,
    values: &mut BTreeMap<String, bool>,
    show: &dyn Fn(&Switch) -> bool,
    pal: &Pal,
) -> bool {
    let mut ch = false;
    for sw in sws.iter().filter(|s| show(s)) {
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 150.0).max(160.0);
            label_block(ui, sw, pal, w);
            let cur = values.get(&sw.id).copied();
            let def = format!("Default ({})", on_off(inherited(sw)));
            let text = match cur {
                None => def.clone(),
                Some(v) => on_off(v).to_string(),
            };
            egui::ComboBox::from_id_salt(("ls_emu_tri", salt, sw.id.as_str()))
                .width(130.0)
                .selected_text(text)
                .show_ui(ui, |ui| {
                    for (label, v) in [(def.as_str(), None), ("On", Some(true)), ("Off", Some(false))] {
                        if ui.selectable_label(cur == v, label).clicked() && cur != v {
                            match v {
                                None => values.remove(&sw.id),
                                Some(b) => values.insert(sw.id.clone(), b),
                            };
                            ch = true;
                        }
                    }
                });
        });
        ui.add_space(2.0);
    }
    ch
}

/// On / Off rows (an emulator's own level): only a value that differs from the default is
/// stored. True when something changed.
pub(crate) fn toggle_rows(
    ui: &mut Ui,
    sws: &[Switch],
    values: &mut BTreeMap<String, bool>,
    show: &dyn Fn(&Switch) -> bool,
    pal: &Pal,
) -> bool {
    let mut ch = false;
    for sw in sws.iter().filter(|s| show(s)) {
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 60.0).max(160.0);
            label_block(ui, sw, pal, w);
            let mut on = values.get(&sw.id).copied().unwrap_or(sw.default);
            if toggle_switch(ui, &mut on, 20.0, pal.accent).changed() {
                if on == sw.default {
                    values.remove(&sw.id);
                } else {
                    values.insert(sw.id.clone(), on);
                }
                ch = true;
            }
        });
        ui.add_space(2.0);
    }
    ch
}

/// "Command-line options for PCSX2 ↗" when its documentation is known.
pub(crate) fn docs_link(ui: &mut Ui, st: &mut AppState, emu: &Emulator, pal: &Pal) {
    let Some(url) = emu.definition().and_then(|d| d.docs_url) else { return };
    if link_text(ui, &format!("Command-line options for {}", emu.name), url, pal) {
        st.open_url(url);
    }
}

/// A single-line monospace "Extra arguments" box. True when changed.
pub(crate) fn extra_args_box(ui: &mut Ui, text: &mut String, hint: &str, pal: &Pal) -> bool {
    let ch = ui
        .add(
            egui::TextEdit::singleline(text)
                .font(egui::TextStyle::Monospace)
                .hint_text(RichText::new(hint).family(egui::FontFamily::Monospace))
                .desired_width(ui.available_width() - 4.0)
                .margin(egui::Margin::symmetric(7, 4)),
        )
        .changed();
    hint_text(ui, EXTRA_HELP, pal);
    ch
}

/// The levels below a game (its console's, the emulator's) from the settings being shown.
pub(crate) fn levels_below(
    cfg: &EmulationConfig,
    platform: Option<&str>,
    emu: &str,
) -> Vec<(BTreeMap<String, bool>, Vec<String>)> {
    let mut v = vec![];
    if let Some(p) = platform {
        let c = cfg.console_settings.get(p).cloned().unwrap_or_default();
        v.push((c.options, crate::launch::split_command(&c.extra_args).unwrap_or_default()));
    }
    let e = cfg.emulator_settings.get(emu).cloned().unwrap_or_default();
    v.push((e.options, crate::launch::split_command(&e.extra_args).unwrap_or_default()));
    v
}

/// What Default means for a switch given the levels below.
pub(crate) fn inherited_from(sw: &Switch, below: &[(BTreeMap<String, bool>, Vec<String>)]) -> bool {
    let lv: Vec<Level> = below.iter().map(|(o, w)| Level { options: o, words: w }).collect();
    options::inherited(sw, &lv)
}

/// The switches of an emulator + profile with the fullscreen default from the settings.
pub(crate) fn switches_of(emu: &Emulator, profile: &Profile, cfg: &EmulationConfig) -> Vec<Switch> {
    options::switches(emu, profile, cfg.fullscreen)
}

/// The profile an emulator uses for a console (its first that runs it, else its first).
pub(crate) fn profile_for<'a>(profiles: &'a [Profile], platform: Option<&str>) -> Option<&'a Profile> {
    platform.and_then(|pf| profiles.iter().find(|p| p.supports(pf))).or(profiles.first())
}

/// An "Extra arguments" box for a setting that is saved as a whole (Settings): typed into a
/// buffer, written to `value` when the box loses focus or Enter is pressed (not on every key, as
/// saving Settings looks for emulators again). True when `value` changed.
pub(crate) fn buffered_args(
    ui: &mut Ui,
    salt: impl std::hash::Hash + std::fmt::Debug,
    value: &mut String,
    hint: &str,
    pal: &Pal,
) -> bool {
    let id = egui::Id::NULL.with(("ls_emu_args_buf", salt));
    let mut buf: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| value.clone());
    let r = ui.add(
        egui::TextEdit::singleline(&mut buf)
            .font(egui::TextStyle::Monospace)
            .hint_text(RichText::new(hint).family(egui::FontFamily::Monospace))
            .desired_width(ui.available_width() - 4.0)
            .margin(egui::Margin::symmetric(7, 4)),
    );
    let commit = r.lost_focus() && buf.trim() != value.trim();
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
    let bad = crate::launch::split_command(&buf).is_err();
    if bad {
        ui.label(RichText::new("Check the quotes: this can't be split into words.").size(12.0).color(pal.danger));
    }
    hint_text(ui, EXTRA_HELP, pal);
    if commit && !bad {
        *value = buf.trim().to_string();
        return true;
    }
    false
}
