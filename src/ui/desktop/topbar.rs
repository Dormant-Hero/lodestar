//! Thin top bar: app menu, view switcher, cover size, filters, sort, search, panel toggles.

use super::{Action, DesktopUi, settings::Tab};
use crate::app::{AppState, Mode};
use crate::config::{DesktopConfig, DesktopView, GroupBy, QuickPreset, SortBy};
use crate::ui::widgets::{Icon, icon_button, paint_icon, readable_on};
use egui::{Align, Color32, Layout, Response, RichText, Sense, Ui, vec2};

const BAR_H: f32 = 28.0;

impl DesktopUi {
    pub(super) fn top_bar(&mut self, ui: &mut Ui, st: &mut AppState) {
        ui.set_height(BAR_H);
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let fit = BarFit::for_width(ui.available_width());
            self.app_menu(ui, st);
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Q)) {
                st.quit();
            }
            ui.add_space(4.0);
            if fit.wordmark {
                ui.label(
                    RichText::new("LODESTAR")
                        .family(self.fonts.title.clone())
                        .size(13.0)
                        .strong()
                        .color(self.pal.text)
                        .extra_letter_spacing(1.5),
                );
                ui.add_space(10.0);
            }
            let view = self.view_switcher(ui, st, fit);

            // ---- right side, from the right edge
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let d = &mut st.config.desktop;
                if view != DesktopView::Details
                    && icon_button(ui, Icon::Panel, d.show_details_panel, BAR_H, "Show or hide the details panel")
                        .clicked()
                {
                    d.show_details_panel = !d.show_details_panel;
                    self.mark_config_dirty();
                }
                if icon_button(ui, Icon::Sidebar, d.show_sidebar, BAR_H, "Show or hide the sidebar").clicked() {
                    d.show_sidebar = !d.show_sidebar;
                    self.mark_config_dirty();
                }
                ui.add_space(4.0);
                self.sort_controls(ui, d, fit);
                // Group by (every view; shown anyway while grouping).
                if fit.group || d.group_by != GroupBy::None {
                    self.group_by_combo(ui, d);
                }
                if icon_button(ui, Icon::Dice, false, BAR_H, "Random game (Ctrl+R)").clicked() {
                    self.push(Action::RandomGame);
                }
                ui.add_space(4.0);
                self.quick_presets(ui, d, fit);
                ui.add_space(4.0);
                self.search_box(ui);
            });
        });
    }

    /// The ☰ app menu (Insert opens it with its Add game submenu).
    fn app_menu(&mut self, ui: &mut Ui, st: &mut AppState) {
        let menu_resp = icon_button(ui, Icon::Menu, false, BAR_H, "Menu");
        if self.debug_menu.as_deref() == Some("appmenu") {
            self.debug_menu = None;
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&menu_resp));
        }
        if std::mem::take(&mut self.open_add_menu) {
            // Insert: the app menu with its Add game submenu open.
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&menu_resp));
            self.add_submenu_open = true;
        }
        egui::Popup::menu(&menu_resp).show(|ui| {
            ui.set_min_width(230.0);
            let add = ui.menu_button("Add game", |ui| self.add_game_menu(ui));
            if std::mem::take(&mut self.add_submenu_open) {
                use egui::containers::menu::{MenuState, SubMenu};
                let sub = SubMenu::id_from_widget_id(add.response.id);
                MenuState::from_ui(ui, |s, _| s.open_item = Some(sub));
                ui.ctx().request_repaint();
            }
            let scan_label = if st.scan.running { "Scanning…" } else { "Rescan library" };
            if ui.add_enabled(!st.scan.running, egui::Button::new(scan_label)).clicked() {
                st.rescan();
            }
            if ui.button("Download artwork…").clicked() {
                let scope = if self.multi.len() > 1 {
                    super::artwork::BulkScope::Selection
                } else {
                    super::artwork::BulkScope::View
                };
                self.push(Action::BulkArt(scope));
            }
            if ui.button("Add-ons…").on_hover_text("Extra libraries, metadata sources and more").clicked() {
                self.push(Action::Addons(super::addons::Section::All));
            }
            if ui.button("Theme creator…").on_hover_text("Make your own controller-mode theme").clicked() {
                self.push(Action::ThemeCreator(None));
            }
            if ui.button("Statistics").on_hover_text("Play time, completion, sources and more").clicked() {
                self.push(Action::Statistics);
            }
            ui.separator();
            if ui.button("Settings…").clicked() {
                self.push(Action::Settings(Tab::General));
            }
            if ui.button("Switch to controller mode").clicked() {
                st.switch_mode(Mode::Controller);
            }
            ui.separator();
            if ui.button("Changelog…").clicked() {
                self.push(Action::WhatsNew);
            }
            if ui.button("About Lodestar…").clicked() {
                self.push(Action::About);
            }
            if ui.add(egui::Button::new("Quit").shortcut_text("Ctrl+Q")).clicked() {
                st.quit();
            }
        });
    }

    /// App menu → Add game ▸.
    fn add_game_menu(&mut self, ui: &mut Ui) {
        ui.set_min_width(190.0);
        if ui.button("Manually…").on_hover_text("Fill in the details yourself").clicked() {
            self.push(Action::AddManual);
        }
        if ui
            .button("Pick a program…")
            .on_hover_text("A Linux program, script, AppImage, Windows .exe or .desktop file")
            .clicked()
        {
            self.push(Action::AddProgram);
        }
        if ui
            .button("Scan for games…")
            .on_hover_text("Find games in a folder or installed ones that aren't in the library")
            .clicked()
        {
            self.push(Action::AddScan);
        }
        if ui
            .button("Emulated games…")
            .on_hover_text("Console games from a ROM folder, started with an emulator")
            .clicked()
        {
            self.push(Action::AddRoms);
        }
        if ui
            .button("Cloud game…")
            .on_hover_text("A game you stream from GeForce NOW, Xbox Cloud Gaming, Amazon Luna, Boosteroid …")
            .clicked()
        {
            self.push(Action::AddCloud);
        }
    }

    /// Details / Grid / List, and the cover size slider in the grid. Returns the view this
    /// frame started with.
    fn view_switcher(&mut self, ui: &mut Ui, st: &mut AppState, fit: BarFit) -> DesktopView {
        let cur = st.config.desktop.view;
        for (v, icon, label) in [
            (DesktopView::Details, Icon::Details, "Details"),
            (DesktopView::Grid, Icon::Grid, "Grid"),
            (DesktopView::List, Icon::List, "List"),
        ] {
            if seg_button(ui, icon, label, fit.view_labels, cur == v, &self.pal).clicked() && cur != v {
                st.config.desktop.view = v;
                self.scroll_to_sel = true;
                self.mark_config_dirty();
            }
        }
        if cur == DesktopView::Grid && fit.slider {
            ui.add_space(10.0);
            let mut w = st.config.desktop.cover_width;
            let r = ui
                .add(egui::Slider::new(&mut w, 100.0..=320.0).show_value(false).trailing_fill(true))
                .on_hover_text("Cover size (Ctrl + mouse wheel)");
            if r.changed() {
                st.config.desktop.cover_width = w;
                self.scroll_to_sel = true;
                self.mark_config_dirty();
            }
        }
        cur
    }

    /// The sort direction button and the Sort drop-down.
    fn sort_controls(&mut self, ui: &mut Ui, d: &mut DesktopConfig, fit: BarFit) {
        // "Normal" order is each field's natural one (A→Z, most recent / most played /
        // newest first), so talk about reversing rather than ascending/descending.
        // "Recently played" always sorts newest first: show that, and lock the controls.
        let (eff_sort, eff_rev, locked) = self.effective_sort(d);
        let locked_tip = if d.preset == QuickPreset::MostPlayed {
            "Most played is always sorted by play time, most first"
        } else {
            "Recently played is always sorted by last played, newest first"
        };
        let (icon, tip) = if eff_rev {
            (Icon::SortDesc, "Reversed order — click for the normal order")
        } else {
            (Icon::SortAsc, "Normal order — click to reverse")
        };
        let r = ui.add_enabled_ui(!locked, |ui| icon_button(ui, icon, false, BAR_H, if locked { "" } else { tip }));
        let r = r.inner.on_disabled_hover_text(locked_tip);
        if !locked && r.clicked() {
            d.sort_descending = !d.sort_descending;
            self.mark_config_dirty();
        }
        let before = d.sort;
        let sort_text =
            if fit.sort_prefix { format!("Sort: {}", eff_sort.label()) } else { eff_sort.label().to_string() };
        let combo = ui.add_enabled_ui(!locked, |ui| {
            egui::ComboBox::from_id_salt("ls_sort")
                .selected_text(sort_text)
                .width(if fit.sort_prefix { 170.0 } else { 140.0 })
                .height(600.0)
                .show_ui(ui, |ui| {
                    // egui caps a popup's first (sizing) frame at 400 px: tighter rows keep
                    // every entry visible without a hidden scroll.
                    ui.spacing_mut().item_spacing.y = 3.0;
                    for s in SortBy::ALL {
                        ui.selectable_value(&mut d.sort, s, s.label());
                    }
                })
        });
        let combo = combo.inner.response.on_disabled_hover_text(locked_tip);
        if self.debug_menu.as_deref() == Some("sortmenu") {
            self.debug_menu = None;
            egui::Popup::open_id(ui.ctx(), combo.id.with("popup"));
        }
        if d.sort != before {
            self.mark_config_dirty();
        }
    }

    fn group_by_combo(&mut self, ui: &mut Ui, d: &mut DesktopConfig) {
        let before = d.group_by;
        let r = egui::ComboBox::from_id_salt("ls_group")
            .selected_text(format!("Group: {}", d.group_by.label()))
            .width(160.0)
            .height(600.0)
            .show_ui(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 3.0;
                for g in GroupBy::ALL {
                    ui.selectable_value(&mut d.group_by, g, g.label());
                }
            })
            .response
            .on_hover_text("Group the games under headers (click a header to fold it)");
        if self.debug_menu.as_deref() == Some("groupmenu") {
            self.debug_menu = None;
            egui::Popup::open_id(ui.ctx(), r.id.with("popup"));
        }
        if d.group_by != before {
            self.scroll_to_sel = true;
            self.mark_config_dirty();
        }
    }

    /// The quick presets: chips in a wide bar, else a "Show:" drop-down (always shown while one
    /// is active, so it never hides itself).
    fn quick_presets(&mut self, ui: &mut Ui, d: &mut DesktopConfig, fit: BarFit) {
        let cur_preset = d.preset;
        let mut want = cur_preset;
        if fit.preset_chips {
            // Right to left: the last one first.
            for p in QuickPreset::ALL.into_iter().rev() {
                let mut on = cur_preset == p;
                if chip(ui, p.label(), &mut on, &self.pal).on_hover_text(p.tip()).clicked() {
                    // Clicking the active preset again goes back to All.
                    want = if cur_preset == p { QuickPreset::All } else { p };
                }
            }
        } else if fit.chips || cur_preset != QuickPreset::All {
            let r = egui::ComboBox::from_id_salt("ls_preset")
                .selected_text(format!("Show: {}", cur_preset.label()))
                .width(150.0)
                .show_ui(ui, |ui| {
                    for p in QuickPreset::ALL {
                        ui.selectable_value(&mut want, p, p.label()).on_hover_text(p.tip());
                    }
                })
                .response
                .on_hover_text("Quick filter");
            if self.debug_menu.as_deref() == Some("presetmenu") {
                self.debug_menu = None;
                egui::Popup::open_id(ui.ctx(), r.id.with("popup"));
            }
        }
        if want != cur_preset {
            d.preset = want;
            d.installed_only = want == QuickPreset::Installed;
            self.mark_config_dirty();
        }
    }

    fn search_box(&mut self, ui: &mut Ui) {
        let pal = self.pal;
        let width = (ui.available_width() - 12.0).clamp(110.0, 280.0);
        let fill = ui.visuals().text_edit_bg_color();
        let id = egui::Id::unique("ls_search");
        let focused = ui.memory(|m| m.has_focus(id));
        egui::Frame::new()
            .fill(fill)
            .corner_radius(5)
            .stroke(egui::Stroke::new(1.0, if focused { pal.accent } else { pal.border }))
            .inner_margin(egui::Margin { left: 6, right: 4, top: 2, bottom: 2 })
            .show(ui, |ui| {
                ui.set_width(width);
                // Explicitly left-to-right: the box sits inside the bar's right-to-left half, and
                // a plain `horizontal` would inherit that (icon on the right, × on the left).
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 20.0), Sense::hover());
                    paint_icon(ui.painter(), r, Icon::Search, pal.dim);
                    let clear_w = if self.filter.search.is_empty() { 0.0 } else { 22.0 };
                    let te = egui::TextEdit::singleline(&mut self.filter.search)
                        .id(id)
                        .hint_text(RichText::new("Search library  (Ctrl+F)").color(pal.dim))
                        .frame(egui::Frame::NONE)
                        .align(egui::Align2::LEFT_CENTER)
                        .desired_width(ui.available_width() - clear_w);
                    let resp = ui.add(te);
                    if self.focus_search {
                        self.focus_search = false;
                        resp.request_focus();
                    }
                    let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
                    if esc && (resp.lost_focus() || resp.has_focus()) {
                        self.filter.search.clear();
                        resp.surrender_focus();
                    }
                    // Down / Page Down hand the keyboard back to the results (query kept);
                    // Enter already does that (single-line edits drop focus on Enter).
                    if resp.has_focus()
                        && ui.input(|i| i.key_pressed(egui::Key::ArrowDown) || i.key_pressed(egui::Key::PageDown))
                    {
                        resp.surrender_focus();
                    }
                    if !self.filter.search.is_empty()
                        && icon_button(ui, Icon::Close, false, 18.0, "Clear search").clicked()
                    {
                        self.filter.search.clear();
                    }
                });
            });
    }
}

/// Which optional top-bar items fit at a given bar width (logical px), so nothing overlaps at
/// small windows or large UI scales. Pure; unit-tested.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BarFit {
    pub wordmark: bool,
    pub view_labels: bool,
    pub slider: bool,
    pub sort_prefix: bool,
    /// The quick presets as chips (else a small drop-down).
    pub preset_chips: bool,
    /// The quick presets at all (shown anyway while one is active).
    pub chips: bool,
    /// The Group by drop-down (shown anyway while grouping).
    pub group: bool,
}

impl BarFit {
    pub fn for_width(w: f32) -> BarFit {
        BarFit {
            // The presets are a compact "Show" drop-down unless the bar is really wide: the
            // wordmark and the view labels matter more (the sidebar has the same entries).
            wordmark: w >= 1460.0,
            view_labels: w >= 1360.0,
            sort_prefix: w >= 1360.0,
            preset_chips: w >= 1960.0,
            slider: w >= 1060.0,
            chips: w >= 760.0,
            group: w >= 700.0,
        }
    }
}

/// Segmented view-switcher button (icon + label, or icon only when space is short).
fn seg_button(ui: &mut Ui, icon: Icon, label: &str, show_label: bool, selected: bool, pal: &super::Pal) -> Response {
    let font = egui::FontId::proportional(13.0);
    let galley = ui.painter().layout_no_wrap(if show_label { label } else { "" }.to_owned(), font, pal.text);
    let size = if show_label { vec2(16.0 + 6.0 + galley.size().x + 20.0, BAR_H) } else { vec2(16.0 + 20.0, BAR_H) };
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let (bg, fg) = if selected {
            (pal.sel_bg, pal.text)
        } else if resp.hovered() {
            (pal.hover, pal.text)
        } else {
            (Color32::TRANSPARENT, pal.dim)
        };
        p.rect_filled(rect, 5.0, bg);
        let ir = egui::Rect::from_min_size(rect.min + vec2(10.0, (BAR_H - 16.0) / 2.0), vec2(16.0, 16.0));
        let icon_fg = if selected { readable_on(pal.selection, bg, pal.text) } else { fg };
        paint_icon(p, ir, icon, icon_fg);
        p.galley_with_override_text_color(
            egui::pos2(ir.right() + 6.0, rect.center().y - galley.size().y / 2.0),
            galley,
            fg,
        );
    }
    resp.on_hover_text(format!("{label} view"))
}

/// Toggle "chip": outlined when off, accent-filled when on.
fn chip(ui: &mut Ui, label: &str, on: &mut bool, pal: &super::Pal) -> Response {
    let font = egui::FontId::proportional(13.0);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, pal.text);
    let size = vec2(galley.size().x + 22.0, BAR_H - 2.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
    }
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let (bg, fg, stroke) = if *on {
            (pal.accent, pal.accent_text, egui::Stroke::NONE)
        } else if resp.hovered() {
            (pal.hover, pal.text, egui::Stroke::new(1.0, pal.border))
        } else {
            (Color32::TRANSPARENT, pal.dim, egui::Stroke::new(1.0, pal.border))
        };
        p.rect_filled(rect, rect.height() / 2.0, bg);
        if stroke.width > 0.0 {
            p.rect_stroke(rect, rect.height() / 2.0, stroke, egui::StrokeKind::Inside);
        }
        p.galley_with_override_text_color(rect.center() - galley.size() / 2.0, galley, fg);
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::BarFit;

    #[test]
    fn bar_items_drop_out_as_the_window_narrows() {
        let wide = BarFit::for_width(2000.0);
        assert!(wide.wordmark && wide.view_labels && wide.slider && wide.chips && wide.preset_chips);
        // The default window (1480 px): wordmark and view labels, presets as a drop-down.
        let default = BarFit::for_width(1480.0);
        assert!(!default.preset_chips && default.slider && default.view_labels && default.wordmark);
        // 1280 px at ui_scale 1.5 ≈ 853 logical px: compact, but the filters stay.
        let narrow = BarFit::for_width(853.0);
        assert!(!narrow.wordmark && !narrow.view_labels && !narrow.slider);
        assert!(narrow.chips);
        let tiny = BarFit::for_width(600.0);
        assert!(!tiny.chips && !tiny.sort_prefix);
        // Monotonic: a wider bar never shows fewer items.
        let mut prev = BarFit::for_width(0.0);
        for w in (0..2000).step_by(10) {
            let f = BarFit::for_width(w as f32);
            assert!(f.wordmark >= prev.wordmark && f.view_labels >= prev.view_labels);
            assert!(f.slider >= prev.slider && f.chips >= prev.chips);
            assert!(f.preset_chips >= prev.preset_chips && f.group >= prev.group);
            prev = f;
        }
    }
}
