//! The Edit Game Details tabs: General, Advanced, Media (new game), Links and Installation.

use super::*;

// ------------------------------------------------------------------------------- General

pub(super) fn general_tab(ui: &mut Ui, st: &AppState, d: &mut EditDlg, pal: &Pal) {
    let wide = ui.available_width() >= 700.0;
    let left = |ui: &mut Ui, d: &mut EditDlg| {
        form_row(ui, "Name", pal, |ui, w| {
            let r = ui.add(te(&mut d.name.text, "Game name", w - RESET_W - ui.spacing().item_spacing.x));
            if d.focus_name {
                d.focus_name = false;
                r.request_focus();
            }
            if r.changed() {
                d.name.touched();
            }
            if reset_button(ui, &d.name) {
                d.name.reset();
            }
        });
        form_row(ui, "Sort name", pal, |ui, w| {
            ui.add(te(&mut d.sort_name, "Automatic (without “The”, “A” …)", w - RESET_W - ui.spacing().item_spacing.x));
        });
        field_row(ui, "Platform", &mut d.platform, "Linux, Windows …", pal);
        field_row(ui, "Genres", &mut d.genres, "Comma-separated", pal);
        field_row(ui, "Developers", &mut d.developers, "Comma-separated", pal);
        field_row(ui, "Publishers", &mut d.publishers, "Comma-separated", pal);
        field_row(ui, "Features", &mut d.features, "Single-player, Co-op …", pal);
        form_row(ui, "Tags", pal, |ui, w| {
            ui.add(te(&mut d.tags, "Comma-separated", w - RESET_W - ui.spacing().item_spacing.x));
        });
    };
    let right = |ui: &mut Ui, d: &mut EditDlg| {
        form_row(ui, "Release date", pal, |ui, w| {
            let bad = {
                let t = d.release_date.text.trim();
                !t.is_empty() && parse_date(t).is_none()
            };
            let r = ui.add(te(&mut d.release_date.text, "YYYY-MM-DD", 104.0).text_color_opt(bad.then_some(pal.danger)));
            if r.changed() {
                d.release_date.touched();
            }
            mark_updated(ui, &r, &d.release_date, pal);
            ui.label(RichText::new("Year").size(13.0).color(pal.dim));
            let yw = (w - 104.0 - 40.0 - RESET_W - 3.0 * ui.spacing().item_spacing.x).max(48.0);
            small_field(ui, &mut d.release_year, "", yw, pal);
            if reset_button(ui, &d.release_year) {
                d.release_year.reset();
            }
        });
        field_row(ui, "Series", &mut d.series, "", pal);
        field_row(ui, "Age rating", &mut d.age_rating, "PEGI 16, ESRB M …", pal);
        field_row(ui, "Region", &mut d.region, "", pal);
        form_row(ui, "Completion", pal, |ui, w| {
            egui::ComboBox::from_id_salt("ls_edit_completion")
                .width(w - RESET_W - ui.spacing().item_spacing.x)
                .selected_text(d.completion.map_or("Not set", |c| c.label()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut d.completion, None, "Not set");
                    for c in CompletionStatus::ALL {
                        ui.selectable_value(&mut d.completion, Some(c), c.label());
                    }
                });
        });
        form_row(ui, "Controller", pal, |ui, w| {
            let text = match d.controller {
                None => d.controller_auto.clone(),
                Some(c) => c.label().to_string(),
            };
            egui::ComboBox::from_id_salt("ls_edit_controller")
                .width(w - RESET_W - ui.spacing().item_spacing.x)
                .selected_text(text)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut d.controller, None, d.controller_auto.as_str());
                    for c in [ControllerSupport::Full, ControllerSupport::Partial, ControllerSupport::None] {
                        ui.selectable_value(&mut d.controller, Some(c), c.label());
                    }
                });
        });
        form_row(ui, "Scores", pal, |ui, w| {
            let fw = ((w - 3.0 * ui.spacing().item_spacing.x - RESET_W) / 3.0).max(40.0);
            ui.spacing_mut().item_spacing.x = 6.0;
            small_field(ui, &mut d.user_score, "User", fw, pal).on_hover_text("Your score, 0 to 100");
            small_field(ui, &mut d.critic_score, "Critic", fw, pal).on_hover_text("Critics' score, 0 to 100");
            small_field(ui, &mut d.community_score, "Community", fw, pal).on_hover_text("Players' score, 0 to 100");
        });
    };
    if wide {
        ui.columns(2, |cols| {
            cols[0].spacing_mut().item_spacing.y = 6.0;
            cols[1].spacing_mut().item_spacing.y = 6.0;
            left(&mut cols[0], d);
            right(&mut cols[1], d);
        });
    } else {
        left(ui, d);
        right(ui, d);
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Description").size(13.0).color(pal.dim));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if reset_button(ui, &d.description) {
                d.description.reset();
            }
        });
    });
    let r = ui.add(
        egui::TextEdit::multiline(&mut d.description.text)
            .desired_rows(5)
            .desired_width(ui.available_width() - 4.0)
            .hint_text(if d.id.is_some() {
                "No description yet. Download metadata… looks one up."
            } else {
                "What the game is about"
            })
            .margin(egui::Margin::symmetric(8, 6)),
    );
    if r.changed() {
        d.description.touched();
    }
    mark_updated(ui, &r, &d.description, pal);
    let _ = st;
}

// ------------------------------------------------------------------------------- Advanced

pub(super) fn advanced_tab(ui: &mut Ui, d: &mut EditDlg, pal: &Pal) {
    let wide = ui.available_width() >= 700.0;
    let left = |ui: &mut Ui, d: &mut EditDlg| {
        section_heading(ui, "Play history", pal);
        date_row(ui, "Last played", &mut d.last_played, "Never", pal);
        form_row(ui, "Play time", pal, |ui, _| {
            let bad = parse_duration(&d.playtime).is_none();
            ui.add(te(&mut d.playtime, "0h", 120.0).text_color_opt(bad.then_some(pal.danger)));
            ui.label(RichText::new("e.g. 12h 30m").size(12.0).color(pal.dim));
        });
        form_row(ui, "Play count", pal, |ui, _| {
            let r = ui.add(te(&mut d.play_count, "0", 120.0));
            if r.changed() {
                d.play_count.retain(|c| c.is_ascii_digit());
            }
        });
        date_row(ui, "Added", &mut d.added, "YYYY-MM-DD", pal);
    };
    let right = |ui: &mut Ui, d: &mut EditDlg| {
        section_heading(ui, "Game", pal);
        field_row(ui, "Version", &mut d.version, "", pal);
        form_row(ui, "Manual", pal, |ui, w| {
            let bw = 30.0;
            let r =
                ui.add(te(&mut d.manual.text, "File or https:// link", w - 2.0 * (bw + ui.spacing().item_spacing.x)));
            if r.changed() {
                d.manual.touched();
            }
            let picking = d.picker.is_some();
            if ui
                .add_enabled_ui(!picking, |ui| ui.add_sized([bw, 26.0], egui::Button::new("…")))
                .inner
                .on_hover_text("Choose a file")
                .clicked()
            {
                d.picker = Some((PickFor::Manual, Pick::start(ui.ctx(), PickKind::File, "Choose the manual", None)));
            }
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.add_space(LABEL_W + ui.spacing().item_spacing.x);
            ui.vertical(|ui| {
                ui.checkbox(&mut d.favorite, "Favourite");
                ui.checkbox(&mut d.hidden, "Hidden");
                // HDR moved to the game options (Default / On / Off, inherited from Settings).
                ui.label(
                    RichText::new("HDR, Gamescope, MangoHud …: Launch options → Game options")
                        .size(11.5)
                        .color(pal.dim),
                );
            });
        });
    };
    if wide {
        ui.columns(2, |cols| {
            cols[0].spacing_mut().item_spacing.y = 6.0;
            cols[1].spacing_mut().item_spacing.y = 6.0;
            left(&mut cols[0], d);
            right(&mut cols[1], d);
        });
    } else {
        left(ui, d);
        right(ui, d);
    }
    ui.add_space(8.0);
    ui.label(RichText::new("Notes").size(13.0).color(pal.dim));
    ui.add(
        egui::TextEdit::multiline(&mut d.notes)
            .desired_rows(5)
            .desired_width(ui.available_width() - 4.0)
            .hint_text("Anything you want to remember about this game")
            .margin(egui::Margin::symmetric(8, 6)),
    );
}

// ------------------------------------------------------------------------------- Media (new game)

pub(super) fn new_media_tab(ui: &mut Ui, d: &mut EditDlg, pal: &Pal) {
    hint_text(ui, "Pick images now, or add the game first and search for artwork online from this tab.", pal);
    ui.add_space(6.0);
    for slot in [ArtSlot::Cover, ArtSlot::Hero, ArtSlot::Logo, ArtSlot::Icon] {
        let idx = d.new_art.iter().position(|(s, _)| *s == slot);
        let mut text = idx.map(|i| d.new_art[i].1.clone()).unwrap_or_default();
        let picking = d.picker.is_some();
        let mut pick = false;
        form_row(ui, slot.label(), pal, |ui, w| {
            let bw = 30.0;
            ui.add(te(&mut text, slot.hint(), w - bw - ui.spacing().item_spacing.x));
            pick = ui
                .add_enabled_ui(!picking, |ui| ui.add_sized([bw, 26.0], egui::Button::new("…")))
                .inner
                .on_hover_text("Choose an image")
                .clicked();
        });
        match idx {
            Some(i) => d.new_art[i].1 = text,
            None if !text.is_empty() => d.new_art.push((slot, text)),
            None => {}
        }
        if pick {
            d.picker = Some((
                PickFor::NewArt(slot),
                Pick::start(ui.ctx(), PickKind::Image, &format!("Choose {} image", slot.label().to_lowercase()), None),
            ));
        }
    }
}

// ------------------------------------------------------------------------------- Links

pub(super) fn links_tab(ui: &mut Ui, st: &mut AppState, d: &mut EditDlg, pal: &Pal) {
    if !d.auto_links.is_empty() {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Automatic").size(14.5).strong().color(pal.text));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut d.hide_auto_links, "Hide automatic links");
            });
        });
        let col = if d.hide_auto_links { with_alpha(pal.dim, 110) } else { pal.dim };
        for l in &d.auto_links {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 20.0), Sense::hover());
                paint_icon(ui.painter(), r, Icon::Link, col);
                let (nr, _) = ui.allocate_exact_size(vec2(LABEL_W - 20.0, 20.0), Sense::hover());
                ui.painter().text(
                    pos2(nr.left(), nr.center().y),
                    egui::Align2::LEFT_CENTER,
                    &l.name,
                    egui::FontId::proportional(13.0),
                    col,
                );
                ui.add(egui::Label::new(RichText::new(&l.url).size(12.0).color(col)).truncate());
            });
        }
        hint_text(ui, "Lodestar adds these for Steam games. They can't be edited, only hidden.", pal);
        ui.add_space(10.0);
    }
    section_heading(ui, "Your links", pal);
    if d.links.is_empty() {
        hint_text(ui, "None yet: add a wiki, a mod page, a guide …", pal);
    }
    let n = d.links.len();
    let mut op: Option<(usize, i32)> = None; // (index, -1 up / 1 down / 0 remove)
    let mut open = None;
    for (i, l) in d.links.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let w = ui.available_width();
            ui.add(te(&mut l.name, "Name", 150.0));
            let btns = 4.0 * (24.0 + 6.0);
            ui.add(te(&mut l.url, "https://…", (w - 150.0 - 6.0 - btns).max(120.0)));
            if icon_button(ui, Icon::Link, false, 24.0, "Open").clicked() && !l.url.trim().is_empty() {
                open = Some(l.url.trim().to_string());
            }
            ui.add_enabled_ui(i > 0, |ui| {
                if icon_button(ui, Icon::ArrowUp, false, 24.0, "Move up").clicked() {
                    op = Some((i, -1));
                }
            });
            ui.add_enabled_ui(i + 1 < n, |ui| {
                if icon_button(ui, Icon::ArrowDown, false, 24.0, "Move down").clicked() {
                    op = Some((i, 1));
                }
            });
            if icon_button(ui, Icon::Trash, false, 24.0, "Remove").clicked() {
                op = Some((i, 0));
            }
        });
    }
    match op {
        Some((i, 0)) => {
            d.links.remove(i);
        }
        Some((i, -1)) => d.links.swap(i, i - 1),
        Some((i, _)) => d.links.swap(i, i + 1),
        None => {}
    }
    if let Some(u) = open {
        st.open_url(&u);
    }
    ui.add_space(4.0);
    if ui.button("Add link").clicked() {
        d.links.push(GameLink { name: String::new(), url: String::new() });
    }
}

// ------------------------------------------------------------------------------- Installation

pub(super) fn install_tab(ui: &mut Ui, st: &mut AppState, d: &mut EditDlg, pal: &Pal) {
    // A cloud game has no folder or size: how it opens instead.
    if let Some(c) = &mut d.cloud {
        c.rows(ui, pal, |ui, label, add| {
            form_row(ui, label, pal, |ui, _| add(ui));
        });
        return;
    }
    form_row(ui, "Folder", pal, |ui, w| {
        let bw = 78.0;
        let r = ui.add(te(
            &mut d.install_dir.text,
            "Where the game is installed",
            w - 2.0 * (bw + ui.spacing().item_spacing.x) - RESET_W - ui.spacing().item_spacing.x,
        ));
        if r.changed() {
            d.install_dir.touched();
        }
        let picking = d.picker.is_some();
        if ui.add_enabled_ui(!picking, |ui| ui.add_sized([bw, 26.0], egui::Button::new("Choose…"))).inner.clicked() {
            let start = Some(PathBuf::from(d.install_dir.text.trim())).filter(|p| p.is_absolute());
            d.picker = Some((
                PickFor::InstallDir,
                Pick::start(ui.ctx(), PickKind::Folder, "Choose the install folder", start),
            ));
        }
        let dir = d.install_dir.text.trim().to_string();
        if ui.add_enabled_ui(!dir.is_empty(), |ui| ui.add_sized([bw, 26.0], egui::Button::new("Open"))).inner.clicked()
        {
            st.open_path(std::path::Path::new(&dir));
        }
        if reset_button(ui, &d.install_dir) {
            d.install_dir.reset();
        }
    });
    form_row(ui, "Installed", pal, |ui, _| {
        if ui.checkbox(&mut d.installed.val, "").changed() {
            d.installed.touched();
        }
        if d.installed.over {
            let tip = match d.installed.source {
                Some(s) => format!("Follow the source again ({})", if s { "installed" } else { "not installed" }),
                None => "Follow the source again (shown after saving)".into(),
            };
            if icon_button(ui, Icon::Reset, false, RESET_W, &tip).clicked() {
                d.installed.over = false;
                if let Some(s) = d.installed.source {
                    d.installed.val = s;
                }
            }
        }
    });
    form_row(ui, "Install size", pal, |ui, _| {
        let latest = d.id.and_then(|id| st.library.get(id)).and_then(|g| g.size_bytes).filter(|s| *s > 0);
        ui.label(RichText::new(latest.map_or("Unknown".to_string(), format_size)).size(13.0).color(pal.text));
        if let Some(id) = d.id {
            if st.size_calculating(id) {
                lazy_spinner(ui, 14.0, pal.accent);
                ui.label(RichText::new("Calculating…").size(12.0).color(pal.dim));
            } else if ui
                .add_enabled(!d.install_dir.text.trim().is_empty(), egui::Button::new("Calculate"))
                .on_hover_text("Add up the install folder's files (in the background)")
                .on_disabled_hover_text("Set the install folder first")
                .clicked()
            {
                st.calc_install_size(id);
            }
        }
    });
    if d.install_dir.over {
        hint_text(ui, "Save first to calculate the size of a changed folder.", pal);
    }
    if d.emu.emulated {
        // Emulated games: emulator / profile choice and the ROM files (edit_emulation.rs).
        ui.add_space(14.0);
        super::super::edit_emulation::section(ui, st, &mut d.emu, d.id, pal);
        ui.add_space(14.0);
        ui.label(RichText::new("RetroAchievements").size(14.5).strong().color(pal.text));
        ui.add_space(2.0);
        form_row(ui, "Game id", pal, |ui, w| {
            ui.add(te(&mut d.ra_game_id, "Automatic", w.min(160.0)));
            if super::parse_ra_id(&d.ra_game_id).is_err() {
                ui.label(RichText::new("A number, like 1234").size(12.0).color(pal.danger));
            }
        });
        hint_text(
            ui,
            "Leave empty to match automatically. Otherwise the number in the game's address on \
             retroachievements.org (…/game/1234).",
            pal,
        );
    }
}
