//! Settings dialog (General, Games, Libraries, Emulation, Appearance, Controller, Metadata,
//! Achievements). The
//! Emulation page lives in `emulation_ui.rs`. Edits `st.config` in place
//! and saves (debounced) through `st.save_config()`.

use super::dialogs::{Buttons, body_height, footer, hint_text, modal};
use super::{DesktopUi, Pal};
use crate::app::AppState;
use crate::config::StartMode;
use crate::model::Source;
use egui::{RichText, Ui, vec2};
use std::path::PathBuf;

mod achievements;
mod appearance;
mod controller;
mod games;
mod general;
mod metadata;
mod theme_options;

pub(crate) use achievements::AchPage;
pub(crate) use appearance::theme_choice_label;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Tab {
    #[default]
    General,
    Games,
    Library,
    Emulation,
    Appearance,
    Controller,
    Metadata,
    Achievements,
}

impl Tab {
    const ALL: [Tab; 8] = [
        Tab::General,
        Tab::Games,
        Tab::Library,
        Tab::Emulation,
        Tab::Appearance,
        Tab::Controller,
        Tab::Metadata,
        Tab::Achievements,
    ];
    fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Games => "Games",
            Tab::Library => "Libraries",
            Tab::Emulation => "Emulation",
            Tab::Appearance => "Appearance",
            Tab::Controller => "Controller",
            Tab::Metadata => "Metadata",
            Tab::Achievements => "Achievements",
        }
    }
}

pub(crate) struct SettingsDlg {
    pub tab: Tab,
    ui_scale: f32,
    /// Libraries: text in each path list's add field.
    pub(super) new_path: std::collections::HashMap<super::libraries::PathTarget, String>,
    pub(super) new_ignore: String,
    /// Libraries: a folder picker open for one of the path lists.
    pub(super) pick: Option<(super::libraries::PathTarget, super::add::Pick)>,
    /// Libraries: expanded rows.
    pub(super) expanded: Vec<Source>,
    /// Libraries → Hidden & excluded, when that page is open.
    pub(super) hidden_page: Option<super::hidden_ui::HiddenPage>,
    /// Libraries → Missing games (`missing_ui.rs`).
    pub(super) missing_page: Option<super::missing_ui::MissingPage>,
    /// "Manage add-ons…" / "Get more themes" was clicked: close Settings and open
    /// the Add-ons window on that page.
    pub(super) open_addons: Option<super::addons::Section>,
    show_key: bool,
    key_edit: Option<String>,
    pub(super) scan_changed: bool,
    /// Emulation page state (owned by `emulation_ui.rs`).
    pub(super) emu: super::emulation_ui::EmuPage,
    /// Achievements page state.
    pub(super) ach: achievements::AchPage,
}

impl SettingsDlg {
    pub fn new(tab: Tab) -> SettingsDlg {
        SettingsDlg {
            tab,
            ui_scale: 0.0,
            new_path: Default::default(),
            new_ignore: String::new(),
            pick: None,
            expanded: vec![],
            hidden_page: None,
            missing_page: None,
            open_addons: None,
            show_key: false,
            key_edit: None,
            scan_changed: false,
            emu: Default::default(),
            ach: Default::default(),
        }
    }
}

impl DesktopUi {
    /// Returns false when the dialog should close.
    pub(super) fn settings_dialog(&mut self, ctx: &egui::Context, st: &mut AppState, d: &mut SettingsDlg) -> bool {
        let pal = self.pal;
        if d.ui_scale == 0.0 {
            d.ui_scale = st.config.ui_scale;
        }
        let mut keep = true;
        let mut changed = false;
        if super::libraries::poll_pick(st, d) {
            changed = true;
            d.scan_changed = true;
        }
        let fam = self.fonts.bold.clone();
        let in_library: Vec<(Source, usize)> =
            self.counts.sources.iter().filter_map(|&(k, n)| Some((k.as_source()?, n))).collect();
        // Taller in a tall window (the Libraries list is long), never taller than it.
        let in_main = (ctx.content_rect().height() - 260.0).clamp(300.0, 640.0);
        let (_, close) = modal(ctx, "ls_settings", &pal, 760.0, |ui| {
            ui.label(RichText::new("Settings").family(fam.clone()).size(20.0).strong().color(pal.text));
            ui.add_space(10.0);
            let body_h = body_height(ui, in_main);
            ui.horizontal_top(|ui| {
                // tabs
                ui.allocate_ui_with_layout(
                    vec2(150.0, body_h),
                    egui::Layout::top_down_justified(egui::Align::Min),
                    |ui| {
                        ui.set_width(150.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for t in Tab::ALL {
                            let sel = d.tab == t;
                            let b = egui::Button::selectable(sel, RichText::new(t.label()).size(14.0))
                                .min_size(vec2(150.0, 30.0));
                            if ui.add(b).clicked() {
                                d.tab = t;
                            }
                        }
                    },
                );
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_width(ui.available_width());
                    egui::ScrollArea::vertical()
                        .id_salt("ls_settings_scroll")
                        .max_height(body_h)
                        .min_scrolled_height(body_h)
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            changed |= match d.tab {
                                Tab::General => general::page(ui, st, d, &pal),
                                Tab::Games => games::page(ui, st, &pal),
                                Tab::Library => {
                                    let (ch, _, addons) = super::libraries::libraries(ui, st, d, &pal, &in_library);
                                    if addons {
                                        d.open_addons = Some(super::addons::Section::All);
                                        keep = false;
                                    }
                                    ch
                                }
                                Tab::Emulation => super::emulation_ui::emulation(ui, st, d, &pal),
                                Tab::Appearance => appearance::page(ui, st, d, &pal, self),
                                Tab::Controller => controller::page(ui, st, d, &pal),
                                Tab::Metadata => metadata::page(ui, st, d, &pal),
                                Tab::Achievements => achievements::page(ui, st, d, &pal),
                            };
                        });
                });
            });
            let f = footer(ui, &pal, Buttons::primary("Done", true), |ui| {
                if d.scan_changed {
                    let label = if st.scan.running { "Scanning…" } else { "Rescan now" };
                    if ui.add_enabled(!st.scan.running, egui::Button::new(label).min_size(vec2(100.0, 30.0))).clicked()
                    {
                        st.save_config();
                        st.rescan();
                        d.scan_changed = false;
                    }
                    ui.label(RichText::new("Library settings changed — rescan to apply.").size(12.0).color(pal.dim));
                }
            });
            if f.primary {
                keep = false;
            }
        });
        if changed {
            self.mark_config_dirty();
        }
        if d.open_addons.is_some() {
            keep = false;
        }
        let open = keep && !close;
        if !open && self.config_dirty.is_some() {
            // Save right away when the dialog closes.
            self.config_dirty = None;
            st.save_config();
        }
        open
    }
}

fn heading(ui: &mut Ui, text: &str, pal: &Pal) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).size(15.0).strong().color(pal.text));
    ui.add_space(2.0);
}

pub(crate) fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~/")
        && let Some(h) = dirs::home_dir()
    {
        return h.join(rest);
    }
    PathBuf::from(s)
}
