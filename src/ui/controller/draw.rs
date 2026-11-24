//! Drawing the controller screens: theme views, details, the empty / no-results / scanning
//! states, the search chip and the playing overlay.

use super::*;

/// How many of a list's first games have their item pictures loaded ahead.
const AHEAD_ITEMS: usize = 8;

impl ControllerUi {
    /// Load ahead the pictures of the screen A opens (the selected collection's game list, the
    /// selected game's page) and, on a game's page, of the games PageUp / PageDown open: they
    /// show right away instead of a few frames after the screen changes.
    pub(super) fn prefetch_next_screen(&mut self, st: &mut AppState, screen: Rect, ppp: f32, now: f64) {
        let theme = st.controller_theme.clone();
        let variant = st.config.controller.variant.clone();
        let themed_details = theme.file.details.is_some();
        match self.screen {
            Screen::Collections if self.home.group.is_none() => {
                let coll = self.colls.get(self.coll_sel);
                // The list's first games in its default order (a sorted list is sorted on
                // opening: then only a remembered game is known) …
                let ordered: Vec<&Game> = match self.colls_games.get(self.coll_sel) {
                    Some(base) if self.sort.is_none() => {
                        let at = |&i: &usize| st.library.games.get(i);
                        if self.descending {
                            base.iter().rev().take(AHEAD_ITEMS).filter_map(at).collect()
                        } else {
                            base.iter().take(AHEAD_ITEMS).filter_map(at).collect()
                        }
                    }
                    _ => vec![],
                };
                // … and the game it opens on: the one selected there last time, else the first.
                let remembered = coll.and_then(|c| self.remembered.get(&c.collection.key())).copied();
                let first = remembered.and_then(|id| st.library.get(id)).or_else(|| ordered.first().copied());
                // Opening further down the list shows other items: those load as usual.
                let items =
                    if remembered.is_none_or(|id| ordered.iter().any(|g| g.id == id)) { ordered } else { vec![] };
                self.renderer.prefetch_view(
                    &theme,
                    ViewKind::Games,
                    coll,
                    first,
                    &items,
                    variant.as_deref(),
                    &mut st.images,
                    screen,
                    ppp,
                    now,
                );
            }
            Screen::Games => {
                let Some(game) = self.sel_id().and_then(|id| st.library.get(id)) else { return };
                if themed_details {
                    let coll = self.cur.and_then(|i| self.colls.get(i));
                    self.renderer.prefetch_view(
                        &theme,
                        ViewKind::Details,
                        coll,
                        Some(game),
                        &[],
                        variant.as_deref(),
                        &mut st.images,
                        screen,
                        ppp,
                        now,
                    );
                } else {
                    details::prefetch(&mut st.images, game);
                }
            }
            // Themed pages: the renderer loads the neighbours' pictures itself.
            Screen::Details if !themed_details => {
                for i in [self.game_sel + 1, self.game_sel.wrapping_sub(1)] {
                    if let Some(g) = self.list_ids.get(i).and_then(|&id| st.library.get(id)) {
                        details::prefetch(&mut st.images, g);
                    }
                }
            }
            _ => {}
        }
    }

    pub(super) fn draw_view(
        &mut self,
        ui: &mut egui::Ui,
        st: &mut AppState,
        kind: ViewKind,
        pass: &ScreenPass,
    ) -> crate::theme::render::Output {
        let ScreenPass { screen, hints, opacity, interactive, .. } = *pass;
        let theme = st.controller_theme.clone();
        let variant = st.config.controller.variant.clone();
        let rail = self.rail_info(st, kind);
        let (coll_idx, items_sel) = match kind {
            ViewKind::Collections => (Some(self.coll_sel), self.coll_sel),
            _ => (self.cur, self.game_sel),
        };
        let coll = coll_idx.and_then(|i| self.colls.get(i));
        let Some(prepared) = self.renderer.prepare(&theme, kind, coll.map(|c| &c.collection), variant.as_deref())
        else {
            return Default::default();
        };
        // Apply the theme's display name for the collection, if it has one.
        let named: Option<CollectionItem> =
            prepared.display_name.as_ref().and_then(|n| coll.map(|c| CollectionItem { name: n.clone(), ..c.clone() }));
        let coll_ref = named.as_ref().or(coll);
        let game = match kind {
            ViewKind::Collections => None,
            _ => self.sel_id().and_then(|id| st.library.get(id)),
        };
        // Rail themes: the collections page shows one section (Platforms, Sources …).
        let group = self.home.group.filter(|_| kind == ViewKind::Collections);
        let group_sel = self.home.group_map.iter().position(|&i| i == self.coll_sel).unwrap_or(0);
        let items = match kind {
            ViewKind::Collections if group.is_some() => Items::Collections(&self.home.group_items),
            ViewKind::Collections => Items::Collections(&self.colls),
            _ => Items::Games { games: &st.library.games, indices: &self.list },
        };
        let items_sel = if group.is_some() { group_sel } else { items_sel };
        let list_id = match kind {
            ViewKind::Collections => hash_of((self.colls.len(), group)),
            _ => self.list_id,
        };
        let extra = SceneExtra {
            games: &st.library.games,
            rail: rail.as_ref().map(|i| self.rail_scene(i)),
            section: group.map(home::Group::label),
            ..Default::default()
        };
        let scene = Scene {
            kind,
            game,
            collection: coll_ref,
            items,
            selected: items_sel,
            list_id,
            hints,
            prompts: st.prompts(),
            opacity,
            interactive,
            extra,
        };
        let prepared: Arc<Prepared> = prepared;
        let mut out = self.renderer.draw(ui, screen, &theme, &prepared, &st.fonts, &mut st.images, &scene);
        if let (Some(j), Some(info)) = (out.rail_clicked, &rail) {
            if let Some(k) = info.keys.get(j).cloned() {
                let now = ui.input(|i| i.time);
                self.rail_activate(st, &k, now);
            }
            return Default::default();
        }
        if out.hovered.is_some() || out.clicked.is_some() {
            self.home.rail_focus = self.home.rail_focus && out.clicked.is_none();
        }
        if group.is_some() {
            // Card indices of the section page → indices into `colls`.
            let map = &self.home.group_map;
            out.clicked = out.clicked.and_then(|i| map.get(i).copied());
            out.hovered = out.hovered.and_then(|i| map.get(i).copied());
            if out.scroll_steps != 0 && !map.is_empty() {
                let n = map.len() as i64;
                let i = (group_sel as i64 + out.scroll_steps as i64).clamp(0, n - 1) as usize;
                self.coll_sel = map[i];
                out.scroll_steps = 0;
            }
        }
        out
    }

    pub(super) fn draw_details(
        &mut self,
        ui: &mut egui::Ui,
        st: &mut AppState,
        look: &Look,
        pass: &ScreenPass,
    ) -> bool {
        let ScreenPass { screen, hints, opacity, interactive, now, dt } = *pass;
        let Some(id) = self.details.as_ref().map(|d| d.game) else {
            return false;
        };
        if st.library.get(id).is_none_or(|g| g.hidden != self.cur_is_hidden()) {
            self.details = None;
            self.go(self.details_back(), now);
            return true;
        }
        if st.controller_theme.file.details.is_some() {
            // Themed details: the game is bound, Accept plays.
            crate::ui::achievements::publish_binding(st, id);
            let theme = st.controller_theme.clone();
            let variant = st.config.controller.variant.clone();
            let coll = self.cur.and_then(|i| self.colls.get(i));
            if let Some(prepared) =
                self.renderer.prepare(&theme, ViewKind::Details, coll.map(|c| &c.collection), variant.as_deref())
            {
                let game = st.library.get(id);
                let items = Items::Games { games: &st.library.games, indices: &self.list };
                let scene = Scene {
                    kind: ViewKind::Details,
                    game,
                    collection: coll,
                    items,
                    selected: self.game_sel,
                    list_id: self.list_id,
                    hints,
                    prompts: st.prompts(),
                    opacity,
                    interactive,
                    extra: Default::default(),
                };
                let out = self.renderer.draw(ui, screen, &theme, &prepared, &st.fonts, &mut st.images, &scene);
                return out.animating;
            }
        }
        let primary = Self::primary(st, id);
        let Some(game) = st.library.get(id).cloned() else {
            return false;
        };
        let ach = crate::ui::achievements::for_game(st, id);
        let Some(ds) = self.details.as_mut() else {
            return false;
        };
        let out = details::draw(ui, screen, look, &mut st.images, &game, primary, ds, hints, dt, opacity, ach.as_ref());
        if let (Some(b), true) = (out.clicked, interactive) {
            if let Some(d) = &mut self.details {
                d.focus = b;
            }
            self.details_button(st, id, b, now);
        }
        out.animating
    }

    pub(super) fn draw_empty(
        &mut self,
        ui: &mut egui::Ui,
        st: &AppState,
        screen: Rect,
        look: &Look,
        now: f64,
        hints: &[Hint],
    ) -> bool {
        let p = ui.painter();
        p.rect_filled(screen, 0.0, look.bg);
        rd::gradient_rect(p, screen, 0.0, [fade(look.accent, 0.10), fade(look.accent, 0.0)], false);
        let c = screen.center();
        // A simple star mark.
        rd::star(p, c - vec2(0.0, look.h(0.16)), look.h(0.06), fade(look.accent, 0.9));
        let (title, sub) = if st.scan.running {
            (
                "Looking for games…",
                "Checking Steam, Heroic, Lutris, Bottles, Faugus, your emulators and more.".to_string(),
            )
        } else {
            (
                "No games yet",
                format!(
                    "Lodestar finds games from Steam, Epic, GOG and Amazon (through Heroic), Lutris, Bottles, Faugus, \
                     your emulators and more.\nInstall a game, then press {} to scan again, or add games in desktop mode.",
                    look.prompts.name(Button::A)
                ),
            )
        };
        let g = p.layout_no_wrap(title.into(), look.title(0.05), look.text);
        p.galley(pos2(c.x - g.size().x / 2.0, c.y - look.h(0.06)), g, look.text);
        let style =
            crate::theme::TextStyle { align: crate::theme::Align::Center, line_spacing: 1.4, ..Default::default() };
        let sg = rd::layout(p, &sub, look.regular.clone(), look.h(0.022), look.dim, &style, Some(look.sw * 0.6), None);
        p.galley(pos2(c.x, c.y + look.h(0.02)), sg, look.dim);
        if st.scan.running {
            for i in 0..3 {
                let ph = ((now * 2.5) as f32 - i as f32 * 0.25).sin() * 0.5 + 0.5;
                p.circle_filled(
                    pos2(c.x + (i as f32 - 1.0) * look.h(0.03), c.y + look.h(0.16)),
                    look.h(0.008),
                    fade(look.accent, 0.3 + 0.7 * ph),
                );
            }
        }
        let hb = Rect::from_min_max(
            pos2(screen.left() + 0.04 * look.sw, screen.bottom() - 0.08 * look.sh),
            pos2(screen.right() - 0.045 * look.sw, screen.bottom() - 0.02 * look.sh),
        );
        let mut empty_hints = vec![];
        if !st.scan.running {
            empty_hints.push(Hint::new(Button::A, "Rescan"));
        }
        empty_hints.push(
            hints.iter().find(|h| h.button == Button::Start).cloned().unwrap_or(Hint::new(Button::Start, "Menu")),
        );
        look.paint_screen_hints(p, hb, &empty_hints, look.text);
        st.scan.running
    }

    pub(super) fn draw_no_results(&self, ui: &egui::Ui, screen: Rect, look: &Look) {
        let p = ui.painter();
        let msg = if self.search.is_empty() {
            "Nothing here".to_string()
        } else {
            format!("No matches for “{}”", self.search)
        };
        let g = rd::layout_line(p, &msg, look.bold(0.034), look.text, look.sw * 0.8);
        let r = Rect::from_center_size(screen.center(), g.size() + vec2(look.h(0.06), look.h(0.04)));
        p.rect_filled(r, rd::cr(look.h(0.015)), fade(look.surface, 0.92));
        p.galley(r.center() - g.size() / 2.0, g, look.text);
    }

    pub(super) fn draw_search_chip(&self, ui: &egui::Ui, screen: Rect, look: &Look) {
        let p = ui.painter();
        let g = rd::layout_line(
            p,
            &format!("Search: {}   ·   {} to clear", self.search, look.prompts.name(Button::B)),
            look.medium(0.02),
            look.accent_text,
            look.sw * 0.6,
        );
        let size = g.size() + vec2(look.h(0.04), look.h(0.018));
        let r = Rect::from_min_size(pos2(screen.center().x - size.x / 2.0, screen.top() + look.h(0.035)), size);
        p.rect_filled(r, rd::cr(size.y / 2.0), look.accent);
        p.galley(r.center() - g.size() / 2.0, g, look.accent_text);
    }

    pub(super) fn draw_scanning(ui: &egui::Ui, screen: Rect, look: &Look, now: f64) -> bool {
        let p = ui.painter();
        let g = p.layout_no_wrap("Scanning library".into(), look.medium(0.018), look.text);
        let h = look.h(0.04);
        let r = Rect::from_min_size(
            pos2(screen.left() + look.h(0.04), screen.bottom() - look.h(0.04) - h),
            vec2(g.size().x + h * 1.6, h),
        );
        p.rect_filled(r, rd::cr(h / 2.0), fade(look.surface, 0.92));
        let c = pos2(r.left() + h * 0.55, r.center().y);
        let a = (now * 5.0) as f32;
        for i in 0..8 {
            let ang = a + i as f32 * std::f32::consts::TAU / 8.0;
            p.circle_filled(c + vec2(ang.cos(), ang.sin()) * h * 0.22, h * 0.045, fade(look.accent, i as f32 / 8.0));
        }
        p.galley(pos2(r.left() + h * 1.05, r.center().y - g.size().y / 2.0), g, look.text);
        true
    }

    /// The Now playing screen (or Starting, while a launch waits): the game, since when, and a
    /// Quit / Cancel button that asks twice.
    pub(super) fn draw_playing(
        &mut self,
        ui: &egui::Ui,
        st: &mut AppState,
        screen: Rect,
        look: &Look,
        (id, since): (GameId, i64),
        now: f64,
    ) {
        let Some(game) = st.library.get(id).cloned() else {
            return;
        };
        let p = ui.painter();
        let y = playing_backdrop(p, &mut st.images, &game, screen, look);
        // Launched but not seen running yet: a store client may be starting, updating or waiting
        // for a sign-in.
        let status = crate::ui::launching::status(st, id);
        let y = playing_text(ui, &game, status.as_deref(), screen, look, y, since);
        let starting = status.is_some();
        let armed = self.stop_armed.is_some_and(|t| now - t < STOP_CONFIRM_SECS);
        let a = look.prompts.name(Button::A);
        let label = match (armed, starting) {
            (true, false) => format!("Press {a} again to quit"),
            (false, false) => "Quit game".into(),
            (true, true) => format!("Press {a} again to cancel"),
            (false, true) => "Cancel launch".into(),
        };
        let c = screen.center();
        let bg = p.layout_no_wrap(label, look.bold(0.024), Color32::WHITE);
        let bs = bg.size() + vec2(look.h(0.06), look.h(0.03));
        let br = Rect::from_min_size(pos2(c.x - bs.x / 2.0, y), bs);
        p.rect_filled(br, rd::cr(bs.y / 2.0), if armed { look.danger } else { fade(look.danger, 0.7) });
        p.galley(br.center() - bg.size() / 2.0, bg, Color32::WHITE);
        if let Some(t) = self.stop_armed.filter(|_| armed) {
            // Redraw when the confirmation window closes (button label goes back).
            let left = (STOP_CONFIRM_SECS - (now - t)).max(0.0) + 0.05;
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(left));
        }
        // Both name the menu button: a launch can wait on a sign-in for minutes, and the menu
        // and library are reachable meanwhile.
        let menu_button = settings::menu_button_name(st);
        let hint = p.layout_no_wrap(
            if starting {
                format!("The game starts as soon as its launcher is ready. {menu_button}: menu and library.")
            } else {
                format!("The game runs outside Lodestar; this screen returns when it exits. {menu_button}: menu and library.")
            },
            look.font(0.017),
            fade(look.dim, 0.8),
        );
        p.galley(pos2(c.x - hint.size().x / 2.0, br.bottom() + look.h(0.025)), hint, look.dim);
        let hints = match (armed, starting) {
            (true, false) => vec![Hint::new(Button::A, "Quit game"), Hint::new(Button::B, "Keep playing")],
            (false, false) => vec![Hint::new(Button::A, "Quit game…")],
            (true, true) => vec![Hint::new(Button::A, "Cancel launch"), Hint::new(Button::B, "Keep waiting")],
            (false, true) => vec![Hint::new(Button::A, "Cancel launch…")],
        };
        let hb = Rect::from_min_max(
            pos2(screen.left() + 0.04 * look.sw, screen.bottom() - 0.08 * look.sh),
            pos2(screen.right() - 0.045 * look.sw, screen.bottom() - 0.02 * look.sh),
        );
        look.paint_screen_hints(p, hb, &hints, look.text);
        if let Some((pos, true)) = ui.input(|i| i.pointer.hover_pos().map(|p| (p, i.pointer.primary_clicked())))
            && br.contains(pos)
        {
            self.playing_nav(st, Nav::Accept, now);
        }
    }
}

/// The dimmed, blurred background and the cover. Returns where the text starts.
fn playing_backdrop(
    p: &egui::Painter,
    images: &mut crate::images::ImageCache,
    game: &Game,
    screen: Rect,
    look: &Look,
) -> f32 {
    p.rect_filled(screen, 0.0, fade(look.bg, 0.93));
    if let Some(t) = game.art.hero.as_deref().and_then(|h| images.get(h, crate::images::ImgSize::Thumb)) {
        rd::paint_blurred(
            p,
            screen,
            &t,
            crate::theme::Fit::Cover,
            fade(Color32::WHITE, 0.25),
            look.h(0.03),
            game.edits.background_placement,
        );
    }
    let c = screen.center();
    let mut y = c.y - look.h(0.2);
    if let Some(t) = game.art.cover.as_deref().and_then(|h| images.get(h, crate::images::ImgSize::Medium)) {
        let h = look.h(0.26);
        let r = Rect::from_min_size(pos2(c.x - h / 3.0, y - h * 0.3), vec2(h * 2.0 / 3.0, h));
        rd::drop_shadow(p, r, look.h(0.012), 0.9);
        rd::paint_image(p, r, &t, crate::theme::Fit::Cover, Color32::WHITE, look.h(0.012), 0.0);
        y = r.bottom() + look.h(0.03);
    }
    y
}

/// NOW PLAYING / STARTING (with a spinner), the game's name, and since when it runs (or the
/// launch status). Returns where the button goes.
fn playing_text(
    ui: &egui::Ui,
    game: &Game,
    status: Option<&str>,
    screen: Rect,
    look: &Look,
    mut y: f32,
    since: i64,
) -> f32 {
    let p = ui.painter();
    let c = screen.center();
    let label = p.layout_no_wrap(
        if status.is_some() { "STARTING" } else { "NOW PLAYING" }.into(),
        look.bold(0.018),
        look.accent,
    );
    let sp = if status.is_some() { label.size().y * 1.2 } else { 0.0 };
    let lx = c.x - (label.size().x + sp * 1.4) / 2.0 + sp * 1.4;
    if sp > 0.0 {
        let r = Rect::from_center_size(pos2(lx - sp * 0.9, y + label.size().y / 2.0), vec2(sp, sp));
        crate::ui::widgets::paint_lazy_spinner(ui, r, look.accent);
    }
    p.galley(pos2(lx, y), label.clone(), look.accent);
    y += label.size().y + look.h(0.01);
    let g = rd::layout_line(p, game.display_name(), look.title(0.05), look.text, look.sw * 0.9);
    p.galley(pos2(c.x - g.size().x / 2.0, y), g.clone(), look.text);
    y += g.size().y + look.h(0.012);
    let mins = ((now_unix() - since).max(0) / 60) as u64;
    let (_, _, _, hh, mm, _) = crate::theme::render::bind::local_time(since);
    let info = match status {
        Some(s) => s.to_string(),
        None => format!(
            "Started {hh:02}:{mm:02}  ·  {}",
            if mins == 0 { "just now".to_string() } else { format_playtime(mins * 60) }
        ),
    };
    let ig = p.layout(info, look.medium(0.022), if status.is_some() { look.text } else { look.dim }, look.sw * 0.7);
    let ig = if ig.rows.len() > 1 {
        // Several rows: centre each one.
        let mut job = (*ig.job).clone();
        job.halign = egui::Align::Center;
        p.layout_job(job)
    } else {
        ig
    };
    let ix = if ig.rows.len() > 1 { c.x } else { c.x - ig.size().x / 2.0 };
    p.galley(pos2(ix, y), ig.clone(), look.dim);
    y += ig.size().y + look.h(0.04);
    // Refresh the minute counter once a minute only.
    let secs_left = 60 - (now_unix() - since).rem_euclid(60);
    ui.ctx().request_repaint_after(std::time::Duration::from_secs(secs_left.max(1) as u64));
    y
}
