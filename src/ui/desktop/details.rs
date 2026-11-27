//! Game details: the right-hand panel of the Grid/List views and the large pane of the Details view.

use super::{Action, DesktopUi, FEATURES, Facet, FilterValue, Pal, SrcKey, StoreCaps, more_menu};
use crate::app::AppState;
use crate::art::ArtSlot;
use crate::images::{ImageCache, ImgSize};
use crate::model::{
    BackgroundPlacement, ControllerSupport, DeckRating, Game, LogoPlacement, Source, format_last_played,
    format_playtime,
};
use crate::ui::desktop::edit;
use crate::ui::widgets::{
    Icon, gamepad_solid, icon_button, mix, name_tint, paint_contain, paint_icon, play_triangle, vgradient, with_alpha,
    wrapped,
};
use egui::{Align, Color32, FontId, Rect, RichText, Sense, Ui, UiBuilder, pos2, vec2};

mod achievements;
mod arrange;
pub(crate) use achievements::show_all_id;
pub(crate) use arrange::{Arrange, Layout, save_layout};
use arrange::{HeroPic, HeroTitle};

/// What the big button does for a game right now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PlayState {
    Play,
    Running,
    /// Launched, not seen running yet (store client starting, login …).
    Starting,
    Install,
    /// No longer in the account's Steam library: opens the game in Steam (steam_owned.rs).
    OpenInSteam,
    NotInstalled,
}

pub(crate) fn play_state(g: &Game, running: Option<crate::model::GameId>, caps: Option<&StoreCaps>) -> PlayState {
    if running == Some(g.id) {
        PlayState::Running
    } else if g.installed {
        PlayState::Play
    } else if caps.is_some_and(|c| c.can_install && c.not_in_steam_library) {
        PlayState::OpenInSteam
    } else if caps.is_some_and(|c| c.can_install) {
        PlayState::Install
    } else {
        PlayState::NotInstalled
    }
}

/// `play_state`, telling a launch still on its way from a running game.
fn play_state_now(
    st: &AppState,
    g: &Game,
    running: Option<crate::model::GameId>,
    caps: Option<&StoreCaps>,
) -> PlayState {
    // With "Allow several" any of the running games shows Stop, not only the latest.
    let running = if crate::ui::launching::is_active(st, g.id) { Some(g.id) } else { running };
    match play_state(g, running, caps) {
        PlayState::Running if crate::ui::launching::starting(st, g.id) => PlayState::Starting,
        s => s,
    }
}

/// Under the Play row while `id` is starting: spinner + what the launch waits for.
fn launch_status_line(ui: &mut Ui, st: &AppState, id: crate::model::GameId, pal: &Pal) {
    let Some(text) = crate::ui::launching::status(st, id) else {
        return;
    };
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        crate::ui::widgets::lazy_spinner(ui, 14.0, pal.accent);
        ui.add(egui::Label::new(RichText::new(text).size(13.0).color(pal.text)).wrap());
    });
}

/// Human file size: "12.4 GB".
pub(crate) fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{bytes} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[u])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

impl DesktopUi {
    /// Right-hand panel (Grid / List views).
    pub(super) fn details_panel(&mut self, ui: &mut Ui, st: &mut AppState) {
        let pal = self.pal;
        // `sel_pos` is None when the selected game is not in the current view (e.g. a search
        // with no results): show nothing rather than a game that does not match.
        let shown = self.selected.filter(|_| self.sel_pos.is_some());
        let Some(gi) = shown.and_then(|id| st.library.index_of(id)) else {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("No game selected").color(pal.dim)));
            return;
        };
        let running = crate::ui::launching::running(st);
        let w = ui.available_width();
        let hero_h = (w * 0.58).clamp(200.0, 330.0);
        let (hero_rect, hero_resp) = ui.allocate_exact_size(vec2(w, hero_h), Sense::click());
        let id = st.library.games[gi].id;
        let arranging = self.arranging(id);
        if hero_resp.double_clicked() && !arranging {
            self.push(Action::Edit(id, edit::Tab::General));
        }
        let pic;
        {
            let g = &st.library.games[gi];
            pic = paint_hero(
                ui.painter(),
                &mut st.images,
                g,
                hero_rect,
                &pal,
                self.fills.surface,
                true,
                self.background_of(g),
                HeroLook::of(&st.desktop_theme.file.desktop),
            );
            let logo_box = Rect::from_center_size(
                pos2(hero_rect.center().x, hero_rect.bottom() - hero_h * 0.26),
                vec2(w * 0.78, hero_h * 0.38),
            );
            if !paint_logo(ui.painter(), &mut st.images, g, logo_box, vec2(0.5, 0.5)) {
                let font = FontId::new(
                    super::style::scaled_title(&st.desktop_theme.file.desktop, 24.0),
                    self.fonts.title.clone(),
                );
                title_text(
                    ui.painter(),
                    g.display_name(),
                    font,
                    hero_rect.shrink2(vec2(16.0, 12.0)),
                    &pal,
                    Align::Center,
                );
            }
        }
        // The panel's logo is laid out automatically: Arrange moves only the background here.
        self.arrange_hero(ui, st, id, hero_rect, pic, HeroTitle::Fixed);
        let over = (ui.rect_contains_pointer(hero_rect) || self.debug_pencil) && !arranging;
        if super::artwork::edit_art_button(ui, hero_rect, over, "panel_hero", "Edit artwork", &pal) {
            self.push(Action::EditArt(st.library.games[gi].id, ArtSlot::Hero));
        }
        {
            let g = &st.library.games[gi];
            if !g.art.is_empty()
                && super::artwork::view_art_button(ui, hero_rect, over, "panel_hero", "View artwork full size", &pal)
            {
                let slot = crate::ui::art_viewer::background_slot(&g.art);
                self.push(Action::ViewArt(g.id, slot));
            }
        }

        egui::Frame::new().inner_margin(egui::Margin { left: 16, right: 16, top: 4, bottom: 8 }).show(ui, |ui| {
            let g = &st.library.games[gi];
            let caps = self.caps_for(g.id).cloned();
            let state = play_state_now(st, g, running, caps.as_ref());
            self.play_row(ui, g, state, 42.0);
            launch_status_line(ui, st, g.id, &pal);
            super::romm_ui::progress_line(ui, st, g.id, &pal);
            super::missing_ui::page_hint(ui, g, &pal);
            super::missing_ui::steam_library_hint(ui, caps.as_ref(), &pal);
            ui.add_space(10.0);
            let mut acts = vec![];
            if feature_chips(ui, g, &pal, pal.surface, &mut acts) {
                ui.add_space(10.0);
            }
            let ach = crate::ui::achievements::for_game(st, id);
            let ach_look = achievements::Look {
                heading_pt: 15.0,
                text_pt: 13.5,
                bold: self.fonts.bold.clone(),
                recent: 3,
                icon: 34.0,
            };
            egui::ScrollArea::vertical().id_salt("ls_details_scroll").auto_shrink([false, false]).show(ui, |ui| {
                let g = &st.library.games[gi];
                stats_grid(ui, g, &pal, "ls_stats_panel", &mut acts);
                link_chips(ui, g, &pal, &mut acts);
                let lgen = st.library.generation;
                let desc = self.description(lgen, g).to_owned();
                if !desc.is_empty() {
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Description")
                            .family(self.fonts.bold.clone())
                            .size(15.0)
                            .strong()
                            .color(pal.text),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(desc).size(13.5).color(mix(pal.text, pal.dim, 0.25)).line_height(Some(19.0)),
                    );
                    if let Some(url) = wiki_credit(ui, g, &pal) {
                        acts.push(Action::OpenUrl(url));
                    }
                }
                let space = 18.0;
                acts.extend(achievements::section(ui, &mut st.images, id, ach.as_ref(), &pal, &ach_look, space));
            });
            self.actions.extend(acts);
        });
    }

    /// The Details view's large pane. The page scrolls as a whole (hero, Play row, description
    /// and the info card), like Playnite's: a scroll box of its own for the part below the
    /// cover cut the info card off at ordinary window sizes. Each game opens at the top.
    pub(super) fn details_view(&mut self, ui: &mut Ui, st: &mut AppState) {
        let Some(id) = self.selected.filter(|&id| st.library.index_of(id).is_some()) else {
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt(("ls_dv_page", id))
            .auto_shrink([false, false])
            .show(ui, |ui| self.details_page(ui, st));
    }

    fn details_page(&mut self, ui: &mut Ui, st: &mut AppState) {
        let Some(gi) = self.selected.and_then(|id| st.library.index_of(id)) else {
            return;
        };
        let id = st.library.games[gi].id;
        let full = ui.available_rect_before_wrap();
        let w = full.width();
        let hero_h = (full.height() * 0.44).clamp(220.0, 480.0);
        let (hero_rect, hero_resp) = ui.allocate_exact_size(vec2(w, hero_h), Sense::click());
        let arranging = self.arranging(id);
        if hero_resp.double_clicked() && !arranging {
            self.push(Action::Edit(id, edit::Tab::General));
        }
        let l = PageLayout::new(w, hero_h, hero_rect);
        let (pic, title) = self.paint_page_header(ui, st, gi, &l);
        self.arrange_hero(ui, st, id, hero_rect, pic, title);
        self.header_art_buttons(ui, st, gi, &l, arranging);

        // Play row + quick facts next to the cover, below the hero (both wrap when narrow).
        let row_rect = Rect::from_min_max(pos2(l.text_left, l.row_top), pos2(full.right() - l.margin, full.bottom()));
        let row_bottom = ui
            .scope_builder(UiBuilder::new().max_rect(row_rect).id_salt("ls_dv_row"), |ui| {
                self.page_play_row(ui, st, gi)
            })
            .response
            .rect
            .bottom();

        // Below: description (left) and stats (right), stacked when narrow. As tall as they
        // are: the page around them scrolls.
        let body = Rect::from_min_max(
            pos2(full.left() + l.margin, l.cover_rect.bottom().max(row_bottom) + 22.0),
            pos2(full.right() - l.margin, f32::INFINITY),
        );
        ui.scope_builder(UiBuilder::new().max_rect(body).id_salt("ls_dv_body"), |ui| {
            self.page_body(ui, st, gi);
            ui.add_space(l.margin);
        });
    }

    /// The hero with the logo (or the name) and the cover. Returns the background picture drawn
    /// and the logo, for Arrange mode.
    fn paint_page_header(
        &mut self,
        ui: &mut Ui,
        st: &mut AppState,
        gi: usize,
        l: &PageLayout,
    ) -> (Option<HeroPic>, HeroTitle) {
        let pal = self.pal;
        let PageLayout { hero_rect, narrow, cover_rect, .. } = *l;
        let g = &st.library.games[gi];
        let p = ui.painter();
        let pic = paint_hero(
            p,
            &mut st.images,
            g,
            hero_rect,
            &pal,
            self.fills.page,
            false,
            self.background_of(g),
            HeroLook::of(&st.desktop_theme.file.desktop),
        );
        let title = if st.config.desktop.show_page_logo {
            HeroTitle::Placed(self.paint_page_title(p, st, gi, l))
        } else {
            HeroTitle::Fixed
        };
        if !narrow {
            let g = &st.library.games[gi];
            let cr = st.desktop_theme.file.desktop.cover_radius();
            crate::ui::widgets::soft_shadow(p, cover_rect, cr, 18, 6, 140);
            super::grid::paint_game_cover(p, &mut st.images, g, cover_rect, cr, &pal, ImgSize::Medium, false);
        }
        (pic, title)
    }

    /// The logo above the hero's bottom edge, right of the cover, or where the user placed it
    /// (Arrange mode: the placement being arranged right now); the name when there is no logo.
    /// Nothing when the game's logo is hidden, except faintly while arranging. Returns the logo's
    /// rect.
    fn paint_page_title(&self, p: &egui::Painter, st: &mut AppState, gi: usize, l: &PageLayout) -> Option<Rect> {
        let PageLayout { w, hero_h, hero_rect, narrow, margin, text_left, row_top, .. } = *l;
        let g = &st.library.games[gi];
        let (placement, hidden) = match &self.arrange {
            Some(a) if a.id == g.id => (a.live.logo, a.live.logo_hidden),
            _ if g.edits.logo_hidden => return None,
            _ => (g.edits.logo_placement.map(LogoPlacement::clamped), false),
        };
        let mut p = p.clone();
        if hidden {
            // Faint while arranging, so it is clear what Show logo brings back.
            p.multiply_opacity(0.3);
        }
        let logo_box = Rect::from_min_max(
            pos2(text_left, hero_rect.top() + hero_h * 0.18),
            pos2((text_left + w.max(500.0) * 0.42).min(hero_rect.right() - margin), row_top - 12.0),
        );
        let drawn = match placement {
            Some(pl) => paint_logo_placed(&p, &mut st.images, g, hero_rect, pl),
            None if logo_box.height() > 30.0 => paint_logo_rect(&p, &mut st.images, g, logo_box, vec2(0.0, 1.0)),
            None => LogoDraw::Missing,
        };
        let logo_rect = match drawn {
            LogoDraw::Drawn(r) => Some(r),
            _ => None,
        };
        if drawn == LogoDraw::Missing {
            let font = FontId::new(
                super::style::scaled_title(&st.desktop_theme.file.desktop, if narrow { 28.0 } else { 34.0 }),
                self.fonts.title.clone(),
            );
            let r = Rect::from_min_max(
                pos2(text_left, hero_rect.top() + 12.0),
                pos2(hero_rect.right() - margin, logo_box.bottom()),
            );
            title_text(&p, g.display_name(), font, r, &self.pal, Align::Min);
        }
        logo_rect
    }

    /// Pencils and magnifiers: the cover's while over the cover, the background's elsewhere on
    /// the hero. A click on the cover itself opens the viewer too (only a real cover: the
    /// placeholder keeps the hero's double-click to edit).
    fn header_art_buttons(&mut self, ui: &mut Ui, st: &AppState, gi: usize, l: &PageLayout, arranging: bool) {
        let pal = self.pal;
        let PageLayout { hero_rect, narrow, cover_rect, .. } = *l;
        let g = &st.library.games[gi];
        let id = g.id;
        let has_art = !g.art.is_empty();
        let has_cover = g.art.cover.as_ref().is_some_and(|p| !p.as_os_str().is_empty());
        // Arranging: the whole header is for dragging (no pencils, magnifiers, cover clicks).
        let over_cover = !narrow && (ui.rect_contains_pointer(cover_rect) || self.debug_pencil) && !arranging;
        let over_hero = ((ui.rect_contains_pointer(hero_rect) && !over_cover) || self.debug_pencil) && !arranging;
        if !narrow && has_cover && !arranging {
            let r = ui
                .interact(cover_rect, egui::Id::NULL.with(("ls_cover_view", id)), Sense::click())
                .on_hover_cursor(egui::CursorIcon::ZoomIn);
            if r.clicked() {
                self.push(Action::ViewArt(id, ArtSlot::Cover));
            }
        }
        if has_art
            && super::artwork::view_art_button(ui, hero_rect, over_hero, "view_hero", "View artwork full size", &pal)
        {
            let slot = crate::ui::art_viewer::background_slot(&g.art);
            self.push(Action::ViewArt(id, slot));
        }
        if !narrow
            && has_cover
            && super::artwork::view_art_button(ui, cover_rect, over_cover, "view_cover", "View cover full size", &pal)
        {
            self.push(Action::ViewArt(id, ArtSlot::Cover));
        }
        if super::artwork::edit_art_button(ui, hero_rect, over_hero, "view_hero", "Edit artwork", &pal) {
            self.push(Action::EditArt(id, ArtSlot::Hero));
        }
        if !narrow && super::artwork::edit_art_button(ui, cover_rect, over_cover, "view_cover", "Edit cover", &pal) {
            self.push(Action::EditArt(id, ArtSlot::Cover));
        }
    }

    /// The Play row, notes about launching / downloading / a missing game, the feature chips
    /// and the quick facts.
    fn page_play_row(&mut self, ui: &mut Ui, st: &AppState, gi: usize) {
        let pal = self.pal;
        let g = &st.library.games[gi];
        let running = crate::ui::launching::running(st);
        let caps = self.caps_for(g.id).cloned();
        let state = play_state_now(st, g, running, caps.as_ref());
        self.play_row(ui, g, state, 44.0);
        launch_status_line(ui, st, g.id, &pal);
        super::romm_ui::progress_line(ui, st, g.id, &pal);
        super::missing_ui::page_hint(ui, g, &pal);
        super::missing_ui::steam_library_hint(ui, caps.as_ref(), &pal);
        if emulator_hint(ui, g, &pal) {
            self.push(Action::Settings(super::settings::Tab::Emulation));
        }
        ui.add_space(12.0);
        let mut acts = vec![];
        if feature_chips(ui, g, &pal, pal.bg, &mut acts) {
            ui.add_space(12.0);
        }
        self.actions.extend(acts);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(28.0, 8.0);
            quick_fact(ui, "Play time", &format_playtime(g.playtime_secs), &pal);
            quick_fact(ui, "Last played", &format_last_played(g.last_played), &pal);
            // Emulated games: the console says more than "Emulators".
            match crate::emulation::source_label(g) {
                Some(console) => quick_fact(ui, "Console", &console, &pal),
                None => quick_fact(ui, "Source", g.source_name(), &pal),
            }
            if let Some(y) = g.meta.release_year {
                quick_fact(ui, "Released", &y.to_string(), &pal);
            }
        });
    }

    /// The description and the info card: side by side, or stacked in a narrow pane.
    fn page_body(&mut self, ui: &mut Ui, st: &mut AppState, gi: usize) {
        let pal = self.pal;
        let (card, card_border, card_radius) = (self.fills.card, self.fills.card_border, self.fills.card_radius);
        let ach = crate::ui::achievements::for_game(st, st.library.games[gi].id);
        let g = &st.library.games[gi];
        let lgen = st.library.generation;
        let desc = self.description(lgen, g).to_owned();
        let bold = self.fonts.bold.clone();
        // The theme's text size (`font_scale`; Lodestar: 16 / 14 / 20 pt).
        let ds = &st.desktop_theme.file.desktop;
        let (h_pt, d_pt, d_lh) =
            (super::style::scaled(ds, 16.0), super::style::scaled(ds, 14.0), super::style::scaled(ds, 20.0));
        let wiki_open = std::cell::RefCell::new(None);
        let ach_look = achievements::Look {
            heading_pt: h_pt,
            text_pt: d_pt,
            bold: bold.clone(),
            recent: 5,
            icon: super::style::scaled(ds, 40.0),
        };
        let images = std::cell::RefCell::new(&mut st.images);
        let ach_open = std::cell::RefCell::new(None);
        let achievements = |ui: &mut Ui| {
            let mut images = images.borrow_mut();
            *ach_open.borrow_mut() = achievements::section(ui, &mut images, g.id, ach.as_ref(), &pal, &ach_look, 26.0);
        };
        let description = |ui: &mut Ui| {
            ui.label(RichText::new("Description").family(bold).size(h_pt).strong().color(pal.text));
            ui.add_space(6.0);
            if desc.is_empty() {
                ui.label(
                    RichText::new(
                        "No description yet. Edit game details (the pencil) → \
                             Download metadata… looks one up.",
                    )
                    .color(pal.dim),
                );
            } else {
                ui.label(RichText::new(desc).size(d_pt).color(mix(pal.text, pal.dim, 0.2)).line_height(Some(d_lh)));
                if let Some(url) = wiki_credit(ui, g, &pal) {
                    *wiki_open.borrow_mut() = Some(url);
                }
            }
        };
        let mut acts = vec![];
        let mut stats = |ui: &mut Ui, w: f32| {
            let frame = egui::Frame::new().fill(card).stroke(card_border).corner_radius(card_radius);
            frame.inner_margin(14).show(ui, |ui| {
                ui.set_width(w - 28.0);
                stats_grid(ui, g, &pal, "ls_stats_view", &mut acts);
                link_chips(ui, g, &pal, &mut acts);
            });
        };
        let avail = ui.available_width();
        if avail < 620.0 {
            description(ui);
            achievements(ui);
            ui.add_space(18.0);
            stats(ui, avail.min(420.0));
        } else {
            // A wider card in a wide pane: its values wrap less, so it's shorter.
            let stats_w = (avail * 0.28).clamp(300.0, 380.0).min(avail * 0.4);
            ui.horizontal_top(|ui| {
                let desc_w = ui.available_width() - stats_w - 32.0;
                ui.allocate_ui_with_layout(vec2(desc_w, 0.0), egui::Layout::top_down(Align::Min), |ui| {
                    ui.set_width(desc_w);
                    description(ui);
                    achievements(ui);
                });
                ui.add_space(24.0);
                ui.allocate_ui_with_layout(vec2(stats_w, 0.0), egui::Layout::top_down(Align::Min), |ui| {
                    stats(ui, stats_w)
                });
            });
        }
        if let Some(url) = wiki_open.take() {
            acts.push(Action::OpenUrl(url));
        }
        acts.extend(ach_open.take());
        self.actions.extend(acts);
    }

    /// The action row: big Play button (with a drop-down for the game's other actions) ·
    /// pencil (Edit Game Details) · ••• (More).
    fn play_row(&mut self, ui: &mut Ui, g: &Game, state: PlayState, h: f32) {
        // Wraps instead of clipping when the panel is very narrow (big UI scale, small window).
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let s = h - 6.0;
            self.play_button(ui, g, state, h);
            let pencil = icon_button(ui, Icon::Pencil, false, s, "Edit game details (F3)");
            if pencil.clicked() {
                self.push(Action::Edit(g.id, edit::Tab::General));
            }
            let arranging = self.arranging(g.id);
            if icon_button(ui, Icon::Arrange, arranging, s, "Arrange logo and background").clicked() {
                self.toggle_arrange(g);
            }
            self.more_button(ui, g, s, arranging);
        });
    }

    /// The Play / Stop / Install button, split with a ▾ for the other ways to start the game
    /// when there are any.
    fn play_button(&mut self, ui: &mut Ui, g: &Game, state: PlayState, h: f32) {
        let pal = self.pal;
        let (label, fill, fg, enabled) = match state {
            PlayState::Play => ("Play", pal.play, pal.play_text, true),
            PlayState::Running => ("Stop", pal.danger, Color32::WHITE, true),
            PlayState::Starting => ("Cancel", pal.surface_alt, pal.text, true),
            PlayState::Install if g.source == Source::Romm => ("Download", pal.accent, pal.accent_text, true),
            PlayState::Install => ("Install", pal.accent, pal.accent_text, true),
            PlayState::OpenInSteam => ("Open in Steam", pal.accent, pal.accent_text, true),
            PlayState::NotInstalled => ("Not installed", pal.surface_alt, pal.dim, false),
        };
        let plays = g.edits.actions.iter().filter(|a| a.play).count();
        let extra =
            state == PlayState::Play && (g.edits.actions.iter().any(|a| !a.play) || g.emu.is_some() || plays > 1);
        let arrow_w = if extra { 32.0 } else { 0.0 };
        let font = FontId::new(17.0, self.fonts.bold.clone());
        let gal = ui.painter().layout_no_wrap(label.to_owned(), font, fg);
        let s = h - 6.0;
        let others = 3.0 * (s + ui.spacing().item_spacing.x);
        let min_w = gal.size().x + 60.0 + arrow_w;
        let bw = (ui.available_width() - others).clamp(min_w, 210.0_f32.max(min_w));
        let (rect, _) = ui.allocate_exact_size(vec2(bw, h), Sense::hover());
        let main_rect = Rect::from_min_max(rect.min, pos2(rect.right() - arrow_w, rect.bottom()));
        let arrow_rect = Rect::from_min_max(pos2(main_rect.right(), rect.top()), rect.max);
        let resp = ui.interact(
            main_rect,
            ui.scope_id().with("ls_play_main"),
            if enabled { Sense::click() } else { Sense::hover() },
        );
        let arrow = extra.then(|| {
            ui.interact(arrow_rect, ui.scope_id().with("ls_play_arrow"), Sense::click())
                .on_hover_text("More ways to start it")
        });
        let p = ui.painter();
        let base = if enabled { fill } else { mix(fill, pal.bg, 0.3) };
        // One more than the theme's control radius (Lodestar: 6).
        let br = ui.visuals().widgets.inactive.corner_radius.nw.saturating_add(1);
        p.rect_filled(rect, br, base);
        // Hover / press shading on the part under the pointer.
        let shade = |r: &egui::Response, rr: Rect, cr: egui::CornerRadius| {
            let c = if r.is_pointer_button_down_on() {
                Some(Color32::from_black_alpha(30))
            } else if r.hovered() {
                Some(Color32::from_white_alpha(26))
            } else {
                None
            };
            if let Some(c) = c {
                p.rect_filled(rr, cr, c);
            }
        };
        if enabled {
            let left =
                if extra { egui::CornerRadius { nw: br, sw: br, ne: 0, se: 0 } } else { egui::CornerRadius::same(br) };
            shade(&resp, main_rect, left);
        }
        if let Some(a) = &arrow {
            shade(a, arrow_rect, egui::CornerRadius { nw: 0, sw: 0, ne: br, se: br });
            p.line_segment(
                [pos2(arrow_rect.left(), arrow_rect.top() + 8.0), pos2(arrow_rect.left(), arrow_rect.bottom() - 8.0)],
                egui::Stroke::new(1.0, with_alpha(fg, 90)),
            );
            paint_icon(p, Rect::from_center_size(arrow_rect.center(), vec2(14.0, 14.0)), Icon::ChevronDown, fg);
        }
        let ir = Rect::from_min_size(pos2(main_rect.left() + 22.0, main_rect.center().y - 8.0), vec2(16.0, 16.0));
        match state {
            PlayState::Play => play_triangle(p, ir, fg),
            PlayState::Running => paint_icon(p, ir, Icon::Stop, fg),
            PlayState::Starting => paint_icon(p, ir, Icon::Close, fg),
            PlayState::Install => paint_icon(p, ir, Icon::Download, fg),
            PlayState::OpenInSteam => paint_icon(p, ir, Icon::Link, fg),
            PlayState::NotInstalled => paint_icon(p, ir, Icon::Close, fg),
        }
        p.galley(pos2(ir.right() + 12.0, main_rect.center().y - gal.size().y / 2.0), gal, fg);
        if resp.clicked() {
            // "Choose what to play" opens under the button.
            if state == PlayState::Play {
                super::emulation_ui::choose::set_anchor(ui.ctx(), main_rect.left_bottom() + vec2(0.0, 4.0));
            }
            self.push(match state {
                PlayState::Running | PlayState::Starting => Action::Stop(g.id),
                // Open in Steam is Install too: `install_game` opens Steam for it.
                PlayState::Install | PlayState::OpenInSteam => Action::Install(g.id),
                _ => Action::Play(g.id),
            });
        }
        if let Some(tip) = play_tooltip(g, state) {
            resp.on_hover_text(tip);
        }
        if let Some(a) = &arrow {
            if self.debug_menu.as_deref() == Some("playmenu") {
                self.debug_menu = None;
                egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(a));
            }
            let mut acts = vec![];
            egui::Popup::menu(a).show(|ui| {
                ui.set_min_width(180.0);
                if !super::emulation_ui::choose::menu_entries(ui, g, true, &mut acts)
                    && ui.button(RichText::new("Play").strong()).clicked()
                {
                    acts.push(Action::Play(g.id));
                }
                super::emulation_ui::play_menu_entries(ui, g, true, &mut acts);
                super::action_entries(ui, g, true, &mut acts);
            });
            self.actions.extend(acts);
        }
    }

    /// ••• with the More menu (a dot on it when the game has custom launch options).
    fn more_button(&mut self, ui: &mut Ui, g: &Game, s: f32, arranging: bool) {
        let custom = g.launch_override.as_ref().is_some_and(|o| !o.is_empty());
        let more =
            icon_button(ui, Icon::More, false, s, if custom { "More (custom launch options are set)" } else { "More" });
        if custom {
            ui.painter().circle_filled(more.rect.right_top() + vec2(-6.0, 6.0), 3.5, self.pal.accent);
        }
        if self.debug_menu.as_deref() == Some("menu") {
            self.debug_menu = None;
            egui::Popup::open_id(ui.ctx(), egui::Popup::default_response_id(&more));
        }
        let mut acts = vec![];
        let caps = self.caps_for(g.id).cloned();
        let mut arrange = false;
        egui::Popup::menu(&more).show(|ui| {
            more_menu(ui, g, caps.as_ref(), &mut acts);
            arrange = arrange::arrange_menu_entry(ui, arranging);
        });
        if arrange {
            self.toggle_arrange(g);
        }
        self.actions.extend(acts);
    }

    /// Plain-text description for `g` (HTML stripped), cached for the selected game.
    fn description(&mut self, generation: u64, g: &Game) -> &str {
        let fresh = matches!(&self.desc_cache, Some((id, lgen, _)) if *id == g.id && *lgen == generation);
        if !fresh {
            let text = g.meta.description.as_deref().map(super::strip_html).unwrap_or_default();
            self.desc_cache = Some((g.id, generation, text));
        }
        self.desc_cache.as_ref().map(|c| c.2.as_str()).unwrap_or("")
    }
}

/// "From Wikipedia ↗ · text under CC BY-SA 4.0 ↗" under a description taken from Wikipedia
/// (the licence asks for the credit and the licence link). The URL clicked, if any.
fn wiki_credit(ui: &mut Ui, g: &Game, pal: &Pal) -> Option<String> {
    use crate::metadata::wikipedia::LICENCE_URL;
    let url = g.meta.wikipedia.as_deref()?;
    let mut open = None;
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if super::dialogs::link_text(ui, "From Wikipedia", url, pal) {
            open = Some(url.to_string());
        }
        ui.label(RichText::new("· text under").size(12.0).color(pal.dim));
        if super::dialogs::link_text(ui, "CC BY-SA 4.0", LICENCE_URL, pal) {
            open = Some(LICENCE_URL.to_string());
        }
    });
    open
}

/// Where the parts of the Details page's header go.
#[derive(Clone, Copy)]
struct PageLayout {
    w: f32,
    hero_h: f32,
    hero_rect: Rect,
    /// Narrow pane (small window, big UI scale): no cover, everything left-aligned and wrapping.
    narrow: bool,
    margin: f32,
    cover_rect: Rect,
    /// Where the logo / name and the Play row start.
    text_left: f32,
    row_top: f32,
}

impl PageLayout {
    fn new(w: f32, hero_h: f32, hero_rect: Rect) -> PageLayout {
        let narrow = w < 600.0;
        let margin = if narrow { 18.0 } else { 28.0 };
        let cover_w = if narrow { 0.0 } else { (w * 0.16).clamp(140.0, 220.0) };
        let cover_h = cover_w * 1.5;
        let cover_rect = Rect::from_min_size(
            pos2(hero_rect.left() + margin, hero_rect.bottom() - cover_h * 0.62),
            vec2(cover_w, cover_h),
        );
        let text_left = if narrow { hero_rect.left() + margin } else { cover_rect.right() + 24.0 };
        let row_top = if narrow { hero_rect.bottom() - 36.0 } else { cover_rect.top() + cover_h * 0.22 };
        PageLayout { w, hero_h, hero_rect, narrow, margin, cover_rect, text_left, row_top }
    }
}

/// The Play button's tooltip (none on Play itself).
fn play_tooltip(g: &Game, state: PlayState) -> Option<String> {
    Some(match state {
        PlayState::Running => "Ask the game to quit".into(),
        PlayState::Starting => "Stop waiting for it (a store client that opened stays open)".into(),
        PlayState::Install if g.source == Source::Romm => {
            "On your RomM server. Downloads it into its console's ROM folder; then it plays with your emulator.".into()
        }
        PlayState::Install => format!("Not installed. Opens {} to install it.", install_via(g.source)),
        PlayState::OpenInSteam => {
            "Opens the game in Steam, which shows Install when someone in your Steam Family shares it, \
             or how to get it again."
                .into()
        }
        PlayState::NotInstalled => match g.source {
            Source::Manual | Source::Desktop => "Set its program in Edit… → Actions.".to_string(),
            Source::Emulator => "ROM not found — is the drive connected? (Settings → Emulation)".to_string(),
            s => format!("Install it from {} first.", s.display()),
        },
        PlayState::Play => return None,
    })
}

/// The theme's say on the game picture at the top of game pages.
#[derive(Clone, Copy, Debug, PartialEq)]
struct HeroLook {
    /// Extra darkening (0..1).
    dim: Option<f32>,
    /// Where the fade into the page starts (fraction of the height from the top).
    fade_top: f32,
    /// Over a see-through panel: the bottom fraction that fades out.
    see_through_fade: f32,
}

impl HeroLook {
    fn of(d: &crate::theme::DesktopStyle) -> HeroLook {
        HeroLook {
            dim: d.details_background_dim.filter(|v| v.is_finite() && *v > 0.0).map(|v| v.min(1.0)),
            fade_top: d.details_background_fade.filter(|v| v.is_finite()).map_or(0.35, |f| 1.0 - f.clamp(0.0, 1.0)),
            see_through_fade: d.details_background_fade.filter(|v| v.is_finite()).map_or(0.55, |f| f.clamp(0.0, 1.0)),
        }
    }
}

/// Background image across `rect`, fading into `fade_to` at the bottom; cropped by `crop` (the
/// game's background placement) when set. Returns the picture drawn.
#[allow(clippy::too_many_arguments)]
fn paint_hero(
    p: &egui::Painter,
    images: &mut ImageCache,
    g: &Game,
    rect: Rect,
    pal: &Pal,
    fade_to: Color32,
    emblem: bool,
    crop: Option<BackgroundPlacement>,
    look: HeroLook,
) -> Option<HeroPic> {
    let p = p.with_clip_rect(rect.intersect(p.clip_rect()));
    let sources = [
        (&g.art.hero, ImgSize::Full, vec2(0.5, 0.3)),
        (&g.art.banner, ImgSize::Medium, vec2(0.5, 0.5)),
        (&g.art.cover, ImgSize::Medium, vec2(0.5, 0.2)),
    ];
    // Over an ambient background the panel is translucent: fade the art itself out instead of
    // painting it over with an (also translucent) colour, which would leave a hard edge.
    let see_through = fade_to.a() < 255;
    let mut drawn = None;
    for (path, size, focus) in sources {
        if let Some(path) = path
            && let Some(tex) = images.get(path, size)
        {
            let uv = crate::model::background_uv(crop, tex.size_vec2(), rect.size(), focus);
            if see_through {
                crate::ui::widgets::paint_uv_fading(&p, rect, &tex, uv, look.see_through_fade);
            } else {
                p.add(egui::epaint::RectShape::filled(rect, 0, Color32::WHITE).with_texture(tex.id(), uv));
            }
            drawn = Some(HeroPic { tex: tex.size_vec2(), align: focus });
            break;
        }
    }
    if drawn.is_none() {
        let tint = name_tint(g.display_name(), pal.surface_alt);
        vgradient(&p, rect, mix(tint, pal.bg, 0.1), mix(tint, fade_to, 0.6));
        // No artwork at all: show the game's icon as an emblem above the title.
        if emblem
            && let Some(icon) = &g.art.icon
            && let Some(tex) = images.get(icon, ImgSize::Thumb)
        {
            let s = (rect.height() * 0.34).min(rect.width() * 0.3);
            let bounds = Rect::from_center_size(pos2(rect.center().x, rect.top() + rect.height() * 0.36), vec2(s, s));
            let r = crate::ui::widgets::contain_rect(tex.size_vec2(), bounds);
            crate::ui::widgets::soft_shadow(&p, r.shrink(r.width() * 0.08), 8, 18, 4, 110);
            paint_contain(&p, bounds, &tex, Color32::WHITE, vec2(0.5, 0.5));
        }
    }
    // The theme's darkening of the picture (`details_background_dim`).
    if let (Some(d), Some(_)) = (look.dim, &drawn) {
        p.rect_filled(rect, 0.0, Color32::from_black_alpha((d * 255.0).round() as u8));
    }
    // Darken the top a little (legibility), then fade the bottom into the panel.
    vgradient(
        &p,
        Rect::from_min_max(rect.min, pos2(rect.right(), rect.top() + rect.height() * 0.35)),
        Color32::from_black_alpha(70),
        Color32::TRANSPARENT,
    );
    let fade = Rect::from_min_max(pos2(rect.left(), rect.top() + rect.height() * look.fade_top), rect.max);
    if see_through && drawn.is_some() {
        return drawn;
    }
    vgradient(&p, fade, Color32::TRANSPARENT, fade_to);
    let base = Rect::from_min_max(pos2(rect.left(), rect.bottom() - 2.0), rect.max);
    p.rect_filled(base, 0.0, fade_to);
    drawn
}

/// Logo image letterboxed in `bounds`; false if the game has none (or it is still loading).
fn paint_logo(p: &egui::Painter, images: &mut ImageCache, g: &Game, bounds: Rect, align: egui::Vec2) -> bool {
    let Some(path) = &g.art.logo else {
        return false;
    };
    match images.get(path, ImgSize::Medium) {
        Some(tex) => {
            // soft dark halo behind the logo for contrast on bright heroes
            let r = crate::ui::widgets::contain_rect_aligned(tex.size_vec2(), bounds, align);
            crate::ui::widgets::soft_shadow(p, r.shrink(r.height() * 0.2), 30, 40, 0, 60);
            paint_contain(p, bounds, &tex, Color32::WHITE, align);
            true
        }
        // While a logo is decoding, draw nothing rather than flashing the text title.
        None => !images.is_failed(path, ImgSize::Medium),
    }
}

/// What `paint_logo_rect` / `paint_logo_placed` did.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LogoDraw {
    Drawn(Rect),
    /// Still decoding: draw nothing (no flash of the text title).
    Loading,
    /// No logo (or it failed): draw the title instead.
    Missing,
}

/// The logo letterboxed in `bounds` (the automatic layout).
fn paint_logo_rect(p: &egui::Painter, images: &mut ImageCache, g: &Game, bounds: Rect, align: egui::Vec2) -> LogoDraw {
    let Some(path) = &g.art.logo else {
        return LogoDraw::Missing;
    };
    match images.get(path, ImgSize::Medium) {
        Some(tex) => {
            let r = crate::ui::widgets::contain_rect_aligned(tex.size_vec2(), bounds, align);
            crate::ui::widgets::soft_shadow(p, r.shrink(r.height() * 0.2), 30, 40, 0, 60);
            paint_contain(p, bounds, &tex, Color32::WHITE, align);
            LogoDraw::Drawn(r)
        }
        None if images.is_failed(path, ImgSize::Medium) => LogoDraw::Missing,
        None => LogoDraw::Loading,
    }
}

/// The logo where the user placed it on the header `hero`.
fn paint_logo_placed(p: &egui::Painter, images: &mut ImageCache, g: &Game, hero: Rect, pl: LogoPlacement) -> LogoDraw {
    let Some(path) = &g.art.logo else {
        return LogoDraw::Missing;
    };
    match images.get(path, ImgSize::Medium) {
        Some(tex) => {
            // Its height as placed, narrower than the header.
            let size = crate::model::fit_size(tex.size_vec2(), vec2(hero.width() * 0.95, pl.h * hero.height()));
            let c = pos2(hero.left() + pl.cx * hero.width(), hero.top() + pl.cy * hero.height());
            let r = Rect::from_center_size(c, size);
            crate::ui::widgets::soft_shadow(p, r.shrink(r.height() * 0.2), 30, 40, 0, 60);
            paint_contain(p, r, &tex, Color32::WHITE, vec2(0.5, 0.5));
            LogoDraw::Drawn(r)
        }
        None if images.is_failed(path, ImgSize::Medium) => LogoDraw::Missing,
        None => LogoDraw::Loading,
    }
}

impl DesktopUi {
    /// The background crop to draw for `g`: the one being arranged right now, else its saved one.
    fn background_of(&self, g: &Game) -> Option<BackgroundPlacement> {
        match &self.arrange {
            Some(a) if a.id == g.id => a.live.bg,
            _ => g.edits.background_placement,
        }
    }
}

/// Big title text with a drop shadow, anchored at the bottom of `rect`.
fn title_text(p: &egui::Painter, text: &str, font: FontId, rect: Rect, pal: &Pal, halign: Align) {
    let gal = wrapped(p, text, font.clone(), pal.text, rect.width(), 2, halign);
    let x = match halign {
        Align::Center => rect.center().x, // centre-aligned galleys are laid out around x = 0
        _ => rect.left(),
    };
    let pos = pos2(x, rect.bottom() - gal.size().y);
    let shadow = wrapped(p, text, font, Color32::from_black_alpha(170), rect.width(), 2, halign);
    p.galley(pos + vec2(0.0, 2.0), shadow, Color32::from_black_alpha(170));
    p.galley(pos, gal, pal.text);
}

/// Steam Deck rating colours: green-ish / amber-ish, toned for a dark or light background.
fn deck_tint(r: DeckRating, pal: &Pal) -> Color32 {
    let light = crate::ui::widgets::luminance(pal.bg) > 0.5;
    match r {
        DeckRating::Verified if light => Color32::from_rgb(46, 128, 52),
        DeckRating::Verified => mix(Color32::from_rgb(110, 205, 95), pal.text, 0.2),
        DeckRating::Playable if light => Color32::from_rgb(160, 108, 0),
        DeckRating::Playable => mix(Color32::from_rgb(235, 190, 70), pal.text, 0.2),
        DeckRating::Unsupported => pal.dim,
    }
}

/// The row of small feature chips under the Play row: controller support and the Steam Deck
/// rating. Draws nothing (returns false) when the game has neither. `base` is the (opaque)
/// colour behind the chips. Clicking a chip filters the library to it.
fn feature_chips(ui: &mut Ui, g: &Game, pal: &Pal, base: Color32, acts: &mut Vec<Action>) -> bool {
    let ctl = g.controller();
    let deck = g.meta.deck;
    let proton = g.meta.protondb.as_deref().and_then(ProtonTier::parse);
    if ctl.is_none() && deck.is_none() && proton.is_none() {
        return false;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        if let Some(c) = ctl {
            let tint = if c == ControllerSupport::None { pal.dim } else { mix(pal.text, pal.dim, 0.25) };
            let icon = ChipIcon::Gamepad(c == ControllerSupport::None, base);
            let r = chip(ui, icon, c.label(), tint);
            let from = match g.meta.controller {
                Some(s) if g.source == Source::Steam => {
                    format!("Steam says: {}", super::controller_word(s))
                }
                Some(s) => format!("The store says: {}", super::controller_word(s)),
                None => "Steam has no data for this game".into(),
            };
            let tip = if g.controller_override.is_some() {
                format!("Your setting ({from}).\nChange it in ••• → Controller support.")
            } else {
                format!("{from}.\nChange it in ••• → Controller support.")
            };
            let feature = match c {
                ControllerSupport::Full => Some(FEATURES[0]),
                ControllerSupport::Partial => Some(FEATURES[1]),
                ControllerSupport::None => None,
            };
            let r = r.on_hover_text(if feature.is_some() {
                format!("{tip}\nClick to show all games with it.")
            } else {
                tip
            });
            if let Some(f) = feature
                && r.clicked()
            {
                acts.push(filter_action(ui, Facet::Feature, f));
            }
        }
        if let Some(d) = deck {
            let (icon, label) = match d {
                DeckRating::Verified => (Icon::CheckCircle, "Deck Verified"),
                DeckRating::Playable => (Icon::InfoCircle, "Deck Playable"),
                DeckRating::Unsupported => (Icon::BlockCircle, "Deck Unsupported"),
            };
            let r = chip(ui, ChipIcon::Icon(icon), label, deck_tint(d, pal)).on_hover_text(format!(
                "{}: Valve's compatibility rating, a good hint for how well it runs on \
                     Linux.",
                d.label()
            ));
            let feature = match d {
                DeckRating::Verified => Some(FEATURES[2]),
                DeckRating::Playable => Some(FEATURES[3]),
                DeckRating::Unsupported => None,
            };
            if let Some(f) = feature
                && r.clicked()
            {
                acts.push(filter_action(ui, Facet::Feature, f));
            }
        }
        if let Some(t) = proton {
            let r = chip(ui, ChipIcon::Dot(t.colour(pal)), t.label(), t.tint(pal));
            let steam_app = (g.source == Source::Steam).then(|| g.source_id.parse::<u32>().ok()).flatten();
            let tip = format!(
                "{}\n{}",
                t.explain(),
                if steam_app.is_some() { "Click to open its ProtonDB page." } else { "Rating from ProtonDB." }
            );
            let r = r.on_hover_text(tip);
            if let Some(app) = steam_app
                && r.clicked()
            {
                acts.push(Action::OpenUrl(format!("https://www.protondb.com/app/{app}")));
            }
        }
    });
    true
}

#[derive(Clone, Copy)]
enum ChipIcon {
    /// Gamepad glyph; crossed out when true ("No controller support"). The colour is the
    /// opaque background behind the chip (for the glyph's cut-outs).
    Gamepad(bool, Color32),
    Icon(Icon),
    /// A filled dot of this colour (ProtonDB medal).
    Dot(Color32),
}

/// One small, quiet chip: tinted outline + wash, icon and label in `tint`.
fn chip(ui: &mut Ui, icon: ChipIcon, label: &str, tint: Color32) -> egui::Response {
    let font = FontId::proportional(12.0);
    let gal = ui.painter().layout_no_wrap(label.to_owned(), font, tint);
    let h = 24.0;
    let icon_w = 15.0;
    let size = vec2(9.0 + icon_w + 6.0 + gal.size().x + 11.0, h);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, h / 2.0, with_alpha(tint, if resp.hovered() { 40 } else { 20 }));
        p.rect_stroke(rect, h / 2.0, egui::Stroke::new(1.0, with_alpha(tint, 48)), egui::StrokeKind::Inside);
        let ir = Rect::from_center_size(pos2(rect.left() + 9.0 + icon_w / 2.0, rect.center().y), vec2(icon_w, icon_w));
        match icon {
            ChipIcon::Gamepad(crossed, base) => {
                // The wash is ~8% tint over the background: cut the glyph's holes in that.
                let behind = mix(base, tint, 20.0 / 255.0);
                gamepad_solid(p, ir.expand(1.0), tint, behind);
                if crossed {
                    let a = ir.left_top() + vec2(1.0, 1.0);
                    let b = ir.right_bottom() - vec2(1.0, 1.0);
                    p.line_segment([a, b], egui::Stroke::new(3.2, behind));
                    p.line_segment([a, b], egui::Stroke::new(1.4, tint));
                }
            }
            ChipIcon::Icon(i) => paint_icon(p, ir.shrink(0.5), i, tint),
            ChipIcon::Dot(c) => {
                p.circle_filled(ir.center(), 5.0, c);
                p.circle_stroke(ir.center(), 5.0, egui::Stroke::new(1.0, with_alpha(Color32::BLACK, 50)));
            }
        }
        p.galley(pos2(ir.right() + 6.0, rect.center().y - gal.size().y / 2.0), gal, tint);
    }
    resp
}

/// ProtonDB's rating tiers (Metadata.protondb).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProtonTier {
    Platinum,
    Gold,
    Silver,
    Bronze,
    Borked,
    Native,
}

impl ProtonTier {
    /// "platinum", "Gold" …; None for "pending" / unknown values.
    pub fn parse(s: &str) -> Option<ProtonTier> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "platinum" => ProtonTier::Platinum,
            "gold" => ProtonTier::Gold,
            "silver" => ProtonTier::Silver,
            "bronze" => ProtonTier::Bronze,
            "borked" => ProtonTier::Borked,
            "native" => ProtonTier::Native,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            ProtonTier::Platinum => "ProtonDB Platinum",
            ProtonTier::Gold => "ProtonDB Gold",
            ProtonTier::Silver => "ProtonDB Silver",
            ProtonTier::Bronze => "ProtonDB Bronze",
            ProtonTier::Borked => "ProtonDB Borked",
            ProtonTier::Native => "Native Linux",
        }
    }

    fn explain(self) -> &'static str {
        match self {
            ProtonTier::Platinum => "Platinum: runs perfectly out of the box with Proton.",
            ProtonTier::Gold => "Gold: runs perfectly after a few tweaks.",
            ProtonTier::Silver => "Silver: runs with minor issues, generally playable.",
            ProtonTier::Bronze => "Bronze: runs, but often crashes or has issues.",
            ProtonTier::Borked => "Borked: doesn't start or isn't playable with Proton.",
            ProtonTier::Native => "Has a native Linux version.",
        }
    }

    /// The medal colour (ProtonDB's own hues, a little muted).
    pub fn colour(self, pal: &Pal) -> Color32 {
        let light = crate::ui::widgets::luminance(pal.bg) > 0.5;
        let c = match self {
            ProtonTier::Platinum => Color32::from_rgb(180, 199, 220),
            ProtonTier::Gold => Color32::from_rgb(214, 182, 72),
            ProtonTier::Silver => Color32::from_rgb(170, 170, 176),
            ProtonTier::Bronze => Color32::from_rgb(196, 128, 72),
            ProtonTier::Borked => Color32::from_rgb(214, 84, 76),
            ProtonTier::Native => Color32::from_rgb(92, 190, 150),
        };
        if light { mix(c, Color32::BLACK, 0.18) } else { c }
    }

    /// Text / outline colour: the medal colour toned towards the text so it stays quiet.
    fn tint(self, pal: &Pal) -> Color32 {
        let light = crate::ui::widgets::luminance(pal.bg) > 0.5;
        let c = self.colour(pal);
        if light { mix(c, Color32::BLACK, 0.3) } else { mix(c, pal.text, 0.25) }
    }
}

/// Who installs a game ("Steam", "Heroic" …) for the Install button's tooltip.
fn install_via(s: Source) -> &'static str {
    match s {
        Source::Epic | Source::Gog | Source::Amazon | Source::Heroic => "Heroic",
        Source::Addon => "its add-on",
        Source::Itch => "the itch app",
        s => s.display(),
    }
}

fn quick_fact(ui: &mut Ui, label: &str, value: &str, pal: &Pal) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(label.to_uppercase()).size(10.5).color(pal.dim).extra_letter_spacing(0.8));
        ui.label(RichText::new(value).size(14.5).color(pal.text));
    });
}

/// Filter by a clicked value (Ctrl / Shift: add to the current filter).
fn filter_action(ui: &Ui, f: Facet, v: &str) -> Action {
    let add = ui.input(|i| i.modifiers.command || i.modifiers.shift);
    Action::FilterBy(FilterValue::Facet(f, v.to_string()), add)
}

/// One stats value: plain text, or values that filter the library when clicked.
enum Val {
    Text(String),
    Links(Vec<(String, FilterValue)>),
    /// A value with its source as a dim suffix ("· IGDB", "· yours") and in full on hover;
    /// clicking opens the source's page when there is one.
    Sourced {
        text: String,
        source: &'static str,
        from: String,
        url: Option<String>,
    },
}

fn facet_links(f: Facet, g: &Game) -> Vec<(String, FilterValue)> {
    let mut v = vec![];
    f.for_each(g, |s| {
        if !s.is_empty() && !v.iter().any(|(x, _): &(String, FilterValue)| x == s) {
            v.push((s.to_string(), FilterValue::Facet(f, s.to_string())));
        }
    });
    v
}

/// Under Play of an emulated game Lodestar can't start yet (no emulator for its console, the
/// chosen one gone or not allowed to run): the reason on one line and "Set up emulation…",
/// as Edit → Installation has, instead of a Play that only fails when pressed. True when
/// "Set up emulation…" was clicked.
fn emulator_hint(ui: &mut Ui, g: &Game, pal: &Pal) -> bool {
    if g.emu.is_none() || g.missing.is_some() {
        return false;
    }
    let Some(why) = crate::emulation::registry::missing_reason(g) else { return false };
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let (rect, _) = ui.allocate_exact_size(vec2(13.0, 13.0), Sense::hover());
        paint_icon(ui.painter(), rect, Icon::InfoCircle, pal.dim);
        // The reason gets what the link leaves; a long one ends in "…" with all of it on hover.
        let w = (ui.available_width() - 150.0).max(80.0);
        ui.scope(|ui| {
            ui.set_max_width(w);
            ui.add(egui::Label::new(RichText::new(&why).size(12.5).color(pal.dim)).truncate()).on_hover_text(&why);
        });
        ui.add(egui::Button::new(RichText::new("Set up emulation…").size(12.5).color(pal.accent)).frame(false))
            .on_hover_text("Settings → Emulation: find or add an emulator")
            .clicked()
    })
    .inner
}

/// Label/value rows for whatever the game has. Genres, developers, the platform … are links:
/// clicking one filters the library to it.
fn stats_grid(ui: &mut Ui, g: &Game, pal: &Pal, id: &str, acts: &mut Vec<Action>) {
    let mut rows: Vec<(&str, Val)> = vec![
        ("Play time", Val::Text(format_playtime(g.playtime_secs))),
        ("Last played", Val::Text(format_last_played(g.last_played))),
    ];
    // Lodestar counts its own launches: "0" next to hours played through Steam says nothing.
    if g.play_count > 0 {
        rows.push(("Play count", Val::Text(g.play_count.to_string())));
    }
    if let Some(c) = g.completion() {
        rows.push((
            "Status",
            Val::Links(vec![(c.label().into(), FilterValue::Facet(Facet::Completion, c.label().into()))]),
        ));
    }
    if let Some(s) = g.edits.user_score {
        rows.push(("Your score", Val::Text(format!("{s} / 100"))));
    }
    rows.push(("Source", Val::Links(vec![(source_label(g), FilterValue::Source(SrcKey::of(g)))])));
    let facet_rows = [
        ("Platform", Facet::Platform),
        ("Developer", Facet::Developer),
        ("Publisher", Facet::Publisher),
        ("Released", Facet::Year),
        ("Genres", Facet::Genre),
    ];
    for (label, f) in facet_rows {
        let mut links = facet_links(f, g);
        if links.is_empty() {
            continue;
        }
        let label = if f == Facet::Platform {
            // The same name and label as the Console fact above the description: "Console
            // PlayStation 2" there and "Platform Sony PlayStation 2" here read as two things.
            for (text, _) in &mut links {
                *text = super::facets::platform_display(text);
            }
            if g.emu.is_some() { "Console" } else { label }
        } else {
            label
        };
        rows.push((label, Val::Links(links)));
    }
    // The user's features, else the modes IGDB lists (edits are applied to meta).
    if !g.meta.features.is_empty() {
        rows.push((
            "Features",
            Val::Links(
                g.meta.features.iter().map(|s| (s.clone(), FilterValue::Facet(Facet::Feature, s.clone()))).collect(),
            ),
        ));
    }
    if !g.meta.themes.is_empty() {
        rows.push(("Themes", Val::Text(g.meta.themes.join(", "))));
    }
    for (label, v, own, what) in [
        ("Critic score", g.meta.critic_score, g.edits.critic_score.is_some(), "the average of critics' reviews"),
        (
            "Community score",
            g.meta.community_score,
            g.edits.community_score.is_some(),
            "the average of IGDB users' ratings",
        ),
    ] {
        if let Some(s) = v {
            let (source, from, url) = if own {
                ("yours", "Set by you (Edit game details)".to_string(), None)
            } else {
                ("IGDB", format!("From IGDB: {what}"), g.meta.igdb_url.clone())
            };
            rows.push((label, Val::Sourced { text: format!("{s} / 100"), source, from, url }));
        }
    }
    if !g.tags.is_empty() {
        rows.push(("Tags", Val::Links(g.tags.iter().map(|t| (t.clone(), FilterValue::Tag(t.clone()))).collect())));
    }
    for (label, v) in [
        ("Series", &g.meta.series),
        ("Age rating", &g.edits.age_rating),
        ("Region", &g.edits.region),
        ("Version", &g.edits.version),
    ] {
        if let Some(v) = v.as_deref().filter(|v| !v.trim().is_empty()) {
            rows.push((label, Val::Text(v.to_string())));
        }
    }
    rows.push(("Installed", Val::Text(if g.installed { "Yes" } else { "No" }.into())));
    if let Some(s) = g.size_bytes.filter(|s| *s > 0) {
        rows.push(("Install size", Val::Text(format_size(s))));
    }
    // Label / value rows laid out one by one (not egui::Grid): a value that wraps onto several
    // lines makes its own row taller instead of running into the next one.
    // The label column fits the longest label shown, so none wraps ("Community score").
    let label_w = rows
        .iter()
        .map(|(k, _)| {
            ui.fonts_mut(|f| f.layout_no_wrap(k.to_string(), egui::FontId::proportional(13.0), pal.dim).size().x)
        })
        .fold(80.0_f32, f32::max)
        .ceil();
    let _ = id;
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 7.0;
        for (k, v) in rows {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 14.0;
                ui.allocate_ui_with_layout(vec2(label_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_min_width(label_w);
                    ui.label(RichText::new(k).size(13.0).color(pal.dim));
                });
                let value_w = ui.available_width().max(60.0);
                ui.allocate_ui_with_layout(vec2(value_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                    ui.set_max_width(value_w);
                    match v {
                        Val::Text(t) => {
                            ui.add(
                                egui::Label::new(RichText::new(t).size(13.0).color(pal.text))
                                    .wrap_mode(egui::TextWrapMode::Wrap),
                            );
                        }
                        Val::Sourced { text, source, from, url } => {
                            let hint = match &url {
                                Some(u) => format!("{from}\nClick to open its page ({u})"),
                                None => from,
                            };
                            // Where the score comes from is on the page, not only on hover.
                            let mut job = egui::text::LayoutJob::default();
                            let fmt = |color| egui::TextFormat {
                                font_id: FontId::proportional(13.0),
                                color,
                                ..Default::default()
                            };
                            job.append(&text, 0.0, fmt(pal.text));
                            job.append(&format!("  · {source}"), 0.0, fmt(pal.dim));
                            let mut r = ui.add(egui::Label::new(job).sense(Sense::click()));
                            if url.is_some() && r.hovered() {
                                let y = r.rect.bottom() - 1.0;
                                ui.painter().hline(
                                    r.rect.x_range(),
                                    y,
                                    egui::Stroke::new(1.0, with_alpha(pal.text, 160)),
                                );
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            r = r.on_hover_text(hint);
                            if let (true, Some(u)) = (r.clicked(), url) {
                                acts.push(Action::OpenUrl(u));
                            }
                        }
                        Val::Links(links) => {
                            // Text height, not a button's (set before the row is laid out): the
                            // values line up with their label.
                            ui.spacing_mut().interact_size.y = 0.0;
                            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min).with_main_wrap(true), |ui| {
                                ui.spacing_mut().item_spacing = vec2(0.0, 2.0);
                                let n = links.len();
                                for (i, (text, fv)) in links.into_iter().enumerate() {
                                    if value_link(ui, &text, pal).clicked() {
                                        let add = ui.input(|i| i.modifiers.command || i.modifiers.shift);
                                        acts.push(Action::FilterBy(fv, add));
                                    }
                                    if i + 1 < n {
                                        ui.label(RichText::new(", ").size(13.0).color(pal.dim));
                                    }
                                }
                            });
                        }
                    }
                });
            });
        }
    });
}

/// A detail value that filters the library when clicked (underlined on hover).
fn value_link(ui: &mut Ui, text: &str, pal: &Pal) -> egui::Response {
    // One value never breaks inside itself: whole values move to the next line, so the row is
    // measured right (a value split mid-label ran into the next row).
    let r = ui.add(
        egui::Label::new(RichText::new(text).size(13.0).color(pal.text))
            .wrap_mode(egui::TextWrapMode::Truncate)
            .sense(Sense::click()),
    );
    if r.hovered() {
        let y = r.rect.bottom() - 1.0;
        ui.painter().hline(r.rect.x_range(), y, egui::Stroke::new(1.0, with_alpha(pal.text, 160)));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    r.on_hover_text("Show the games with this (Ctrl+click: add to the filter)")
}

/// Links as one compact row of small chips (at most six, then "+N" with the rest).
fn link_chips(ui: &mut Ui, g: &Game, pal: &Pal, acts: &mut Vec<Action>) {
    let links = g.links();
    if links.is_empty() {
        return;
    }
    const MAX: usize = 6;
    ui.add_space(12.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        let tint = mix(pal.text, pal.dim, 0.35);
        for l in links.iter().take(MAX) {
            if chip(ui, ChipIcon::Icon(Icon::Link), &l.name, tint).on_hover_text(&l.url).clicked() {
                acts.push(Action::OpenUrl(l.url.clone()));
            }
        }
        if links.len() > MAX {
            let rest = &links[MAX..];
            let more = small_chip(ui, &format!("+{}", rest.len()), tint);
            egui::Popup::menu(&more).show(|ui| {
                for l in rest {
                    if ui.button(&l.name).on_hover_text(&l.url).clicked() {
                        acts.push(Action::OpenUrl(l.url.clone()));
                    }
                }
            });
        }
    });
}

/// A chip with just a short label ("+3").
fn small_chip(ui: &mut Ui, label: &str, tint: Color32) -> egui::Response {
    let gal = ui.painter().layout_no_wrap(label.to_owned(), FontId::proportional(12.0), tint);
    let h = 24.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(gal.size().x + 20.0, h), Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, h / 2.0, with_alpha(tint, if resp.hovered() { 40 } else { 20 }));
    p.rect_stroke(rect, h / 2.0, egui::Stroke::new(1.0, with_alpha(tint, 48)), egui::StrokeKind::Inside);
    p.galley(rect.center() - gal.size() / 2.0, gal, tint);
    resp
}

fn source_label(g: &Game) -> String {
    match g.source {
        Source::Steam => format!("Steam  ·  {}", g.source_id),
        _ => g.source_name().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(50 * 1024 * 1024 * 1024), "50.0 GB");
        assert_eq!(format_size(150 * 1024 * 1024 * 1024), "150 GB");
    }

    const NAME: &str = "Hollow Pines";

    /// The Details view at `width` × 800 with the game `NAME` (in the DB, so edits save) selected.
    struct Page {
        st: AppState,
        ui: DesktopUi,
        width: f32,
        id: crate::model::GameId,
        dir: tempfile::TempDir,
    }

    impl Page {
        fn new(width: f32) -> Page {
            let dir = tempfile::tempdir().unwrap();
            let mut st = AppState::for_test(dir.path());
            st.db.merge_scan(&[crate::model::ScannedGame::new(Source::Steam, "1", NAME)], &[Source::Steam]).unwrap();
            st.library = crate::library::Library::new(st.db.load_all().unwrap());
            st.config.desktop.view = crate::config::DesktopView::Details;
            let id = st.library.games[0].id;
            let mut page = Page { st, ui: DesktopUi::new(), width, id, dir };
            // The first frame restores the last session's selection (none here).
            page.frame(vec![]);
            page.ui.selected = Some(id);
            page
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> egui::FullOutput {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(self.width, 800.0))),
                events,
                ..Default::default()
            };
            let ctx = self.st.ctx().clone();
            ctx.run_ui(input, |eui| {
                // As the app does every frame: pictures decoded in the background show up.
                self.st.images.begin_frame();
                self.ui.show(eui, &mut self.st)
            })
        }

        /// The game's name in title-sized text (the hero's; the games list's rows are far
        /// smaller), once the page has settled: the strongest alpha of each.
        fn titles(&mut self) -> Vec<u8> {
            fn visit(s: &egui::Shape, found: &mut Vec<u8>) {
                match s {
                    egui::Shape::Vec(v) => v.iter().for_each(|s| visit(s, found)),
                    egui::Shape::Text(t)
                        if t.galley.text() == NAME
                            && t.galley.job.sections.iter().any(|s| s.format.font_id.size > 20.0) =>
                    {
                        let rows = t.galley.rows.iter().flat_map(|r| &r.row.visuals.mesh.vertices);
                        found.push(rows.map(|v| v.color.a()).max().unwrap_or(0));
                    }
                    _ => {}
                }
            }
            self.frame(vec![]);
            let out = self.frame(vec![]);
            let mut found = vec![];
            out.shapes.iter().for_each(|c| visit(&c.shape, &mut found));
            found
        }

        /// Where the text `label` (a button's) is drawn, once the page has settled.
        fn find(&mut self, label: &str) -> Option<egui::Pos2> {
            fn visit(s: &egui::Shape, label: &str) -> Option<egui::Pos2> {
                match s {
                    egui::Shape::Vec(v) => v.iter().find_map(|s| visit(s, label)),
                    egui::Shape::Text(t) if t.galley.text() == label => Some(t.pos + t.galley.rect.center().to_vec2()),
                    _ => None,
                }
            }
            self.frame(vec![]);
            self.frame(vec![]).shapes.iter().find_map(|c| visit(&c.shape, label))
        }

        /// Click the button labelled `label`.
        fn press(&mut self, label: &str) {
            let pos = self.find(label).unwrap_or_else(|| panic!("no {label} button"));
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            self.frame(vec![egui::Event::PointerMoved(pos)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
        }

        fn edits(&self) -> &crate::model::UserEdits {
            &self.st.library.get(self.id).unwrap().edits
        }

        /// Give the game a logo picture; returns its path.
        fn give_logo(&mut self) -> std::path::PathBuf {
            let logo = self.dir.path().join("logo.png");
            image::RgbaImage::from_pixel(300, 100, image::Rgba([240, 240, 240, 255])).save(&logo).unwrap();
            let art = crate::model::Art { logo: Some(logo.clone()), ..Default::default() };
            self.st.db.set_user_art(self.id, &art).unwrap();
            self.st.library = crate::library::Library::new(self.st.db.load_all().unwrap());
            logo
        }

        /// Frames until the text `label` shows (pictures decode in the background).
        fn wait_for(&mut self, label: &str) {
            let start = std::time::Instant::now();
            while self.find(label).is_none() {
                assert!(start.elapsed().as_secs() < 20, "{label} never showed");
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }

        fn arrange(&mut self) {
            self.ui.arrange = Some(Arrange::new(self.st.library.get(self.id).unwrap()));
        }

        /// Arrange's frame around the logo is there, once the page has settled.
        fn logo_frame(&mut self) -> bool {
            self.frame(vec![]);
            self.frame(vec![]);
            self.st.ctx().read_response(egui::Id::NULL.with(("ls_arrange_logo", self.id))).is_some()
        }
    }

    /// Settings → Appearance → "Show the logo or name over the background": off, the hero
    /// shows no name (wide and narrow pages).
    #[test]
    fn page_logo_setting_hides_the_name() {
        for width in [1400.0, 760.0] {
            let mut page = Page::new(width);
            assert!(!page.titles().is_empty(), "{width}");
            page.st.config.desktop.show_page_logo = false;
            assert!(page.titles().is_empty(), "{width}");
        }
    }

    /// Arrange → Hide name: saved at once, faint while arranging, gone after Done; Show name
    /// brings it back.
    #[test]
    fn arrange_hides_one_games_name() {
        for width in [1400.0, 760.0] {
            let mut page = Page::new(width);
            page.arrange();
            page.press("Hide name");
            assert!(page.edits().logo_hidden, "{width}");
            let faint = page.titles();
            assert!(!faint.is_empty() && faint.iter().all(|&a| a < 100), "{width}: {faint:?}");
            page.press("Show name");
            assert!(!page.edits().logo_hidden, "{width}");
            assert!(page.titles().iter().any(|&a| a > 200), "{width}");
            page.press("Hide name");
            page.press("Done");
            assert!(page.ui.arrange.is_none(), "{width}");
            assert!(page.titles().is_empty(), "{width}");
            assert!(page.edits().logo_hidden, "{width}");
        }
    }

    /// With the setting off, Arrange has nothing to hide or reset, and keeps the game's own
    /// choice for when the setting is turned on again.
    #[test]
    fn arrange_keeps_the_games_choice_while_the_setting_is_off() {
        let mut page = Page::new(1400.0);
        let e = crate::model::UserEdits { logo_hidden: true, ..page.edits().clone() };
        page.st.set_edits(page.id, e);
        page.st.config.desktop.show_page_logo = false;
        page.arrange();
        assert!(page.titles().is_empty());
        assert!(page.find("Show name").is_none() && page.find("Reset logo").is_none());
        page.press("Done");
        assert!(page.ui.arrange.is_none());
        assert!(page.edits().logo_hidden);
        page.st.config.desktop.show_page_logo = true;
        assert!(page.titles().is_empty());
    }

    /// A hidden logo keeps no frame or handle in Arrange, and Reset logo shows it again.
    #[test]
    fn reset_logo_brings_a_hidden_logo_back() {
        let mut page = Page::new(1400.0);
        page.give_logo();
        page.arrange();
        page.wait_for("Hide logo");
        assert!(page.logo_frame());
        page.press("Hide logo");
        assert!(page.edits().logo_hidden);
        assert!(!page.logo_frame());
        page.press("Reset logo");
        assert!(!page.edits().logo_hidden && page.edits().logo_placement.is_none());
        assert!(page.logo_frame() && page.find("Hide logo").is_some());
    }

    /// A hidden logo starts loading only when Arrange starts: the bar still goes to the corner
    /// away from it once it shows.
    #[test]
    fn arrange_bar_keeps_clear_of_a_hidden_logo() {
        let mut page = Page::new(1400.0);
        page.give_logo();
        let high = crate::model::LogoPlacement { cx: 0.7, cy: 0.2, h: 0.3 };
        let e = crate::model::UserEdits { logo_hidden: true, logo_placement: Some(high), ..page.edits().clone() };
        page.st.set_edits(page.id, e);
        assert!(page.titles().is_empty());
        page.arrange();
        page.wait_for("Show logo");
        // The hero is about 320 px tall under the 40 px top bar: the bar sits at its bottom.
        assert!(page.find("Done").unwrap().y > 200.0);
    }
}

#[cfg(test)]
mod hero_look_tests {
    use super::HeroLook;
    use crate::theme::DesktopStyle;

    #[test]
    fn hero_look_defaults_and_theme_values() {
        let d = HeroLook::of(&DesktopStyle::default());
        assert_eq!(d, HeroLook { dim: None, fade_top: 0.35, see_through_fade: 0.55 });
        let t = HeroLook::of(&DesktopStyle {
            details_background_dim: Some(0.6),
            details_background_fade: Some(0.8),
            ..Default::default()
        });
        assert_eq!(t.dim, Some(0.6));
        assert!((t.fade_top - 0.2).abs() < 1e-5 && (t.see_through_fade - 0.8).abs() < 1e-5);
    }
}
