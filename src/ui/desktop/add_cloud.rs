//! Add game → Cloud game…: pick the service (GeForce NOW, Xbox Cloud Gaming, Amazon Luna,
//! Boosteroid, other), the game's name (GeForce NOW: suggested from NVIDIA's public game list)
//! and optionally the game's own page (pasting one fills in the name and the service), and how it
//! opens: Automatic (the service's app when installed, else a browser window), a chosen app or
//! browser, or a tab in the default browser. Full screen and the command under "More options".
//!
//! Detection (apps / browsers) and the GeForce NOW list run on helper threads.

use super::Pal;
use super::add::background;
use super::dialogs::{Buttons, footer, hint_text, modal, title};
use crate::app::AppState;
use crate::cloud::{self, Entry, Found, Open, Service};
use crate::model::GameId;
use crate::ui::widgets::{lazy_spinner, mix, with_alpha};
use crossbeam_channel::Receiver;
use egui::{Color32, RichText, Sense, Ui, vec2};
use std::sync::Arc;

pub(crate) struct CloudDlg {
    service: Service,
    name: String,
    /// The name came from the link (a new link replaces it).
    name_auto: bool,
    link: String,
    /// The link the name / service were last taken from.
    link_seen: String,
    open: Open,
    fullscreen: bool,
    found: Option<Arc<Found>>,
    detecting: Option<Receiver<Arc<Found>>>,
    gfn_titles: Arc<Vec<String>>,
    gfn_loading: Option<Receiver<Result<Vec<String>, String>>>,
    gfn_started: bool,
    sugg_for: Option<String>,
    sugg: Vec<String>,
    more: bool,
    allow_dup: bool,
    error: Option<String>,
}

pub(crate) enum CloudOutcome {
    Keep,
    Close,
    Added(GameId),
    Show(GameId),
}

impl CloudDlg {
    pub fn open(ctx: &egui::Context, link: Option<String>) -> CloudDlg {
        let detecting = Some(background(ctx, "cloud-detect", cloud::refresh));
        let mut d = CloudDlg {
            service: Service::GeForceNow,
            name: String::new(),
            name_auto: false,
            link: link.unwrap_or_default(),
            link_seen: String::new(),
            open: Open::Auto,
            fullscreen: true,
            found: None,
            detecting,
            gfn_titles: Arc::new(vec![]),
            gfn_loading: None,
            gfn_started: false,
            sugg_for: None,
            sugg: vec![],
            more: false,
            allow_dup: false,
            error: None,
        };
        d.take_link();
        d
    }

    /// Testing hook: "More options" shown open.
    pub fn with_more(mut self) -> CloudDlg {
        self.more = true;
        self
    }

    /// Testing hook: pick this service.
    pub fn with_service(mut self, key: &str) -> CloudDlg {
        if let Some(s) = Service::from_key(key) {
            self.set_service(s);
        }
        self
    }

    /// Testing hook: this name typed.
    pub fn with_name(mut self, name: &str) -> CloudDlg {
        self.name = name.to_string();
        self.name_auto = false;
        self
    }

    fn set_service(&mut self, s: Service) {
        if self.service != s {
            self.service = s;
            self.allow_dup = false;
            self.sugg_for = None;
            if self.open == Open::App {
                self.open = Open::Auto;
            }
        }
    }

    /// A new link: its service and the game's name, when the link carries them.
    fn take_link(&mut self) {
        let link = self.link.trim().to_string();
        if link == self.link_seen {
            return;
        }
        self.link_seen = link.clone();
        if let Some(s) = Service::for_url(&link) {
            self.set_service(s);
        } else if cloud::is_web(&link) && self.service != Service::Other && !link_fits(self.service, &link) {
            self.set_service(Service::Other);
        }
        if (self.name.trim().is_empty() || self.name_auto)
            && let Some(n) = cloud::name_from_url(&link)
        {
            self.name = n;
            self.name_auto = true;
        }
    }

    fn entry(&self) -> Entry {
        Entry { service: self.service, open: self.open.clone(), fullscreen: self.fullscreen }
    }

    /// The page the game opens, when it is usable.
    fn url(&self) -> Option<String> {
        let l = self.link.trim();
        if l.is_empty() { self.service.home().map(str::to_string) } else { cloud::is_web(l).then(|| l.to_string()) }
    }

    fn poll(&mut self, st: &AppState) {
        if let Some(rx) = &self.detecting
            && let Ok(f) = rx.try_recv()
        {
            self.found = Some(f);
            self.detecting = None;
        }
        if self.service == Service::GeForceNow && !self.gfn_started {
            self.gfn_started = true;
            let cache = st.paths.cache_dir.clone();
            let offline = st.no_launch;
            self.gfn_loading = Some(background(st.ctx(), "cloud-gfn-list", move || {
                let net = crate::art::net::UreqNet::new();
                cloud::gfn::load(&cache, &net, cloud::gfn::LIST_URL, offline)
            }));
        }
        if let Some(rx) = &self.gfn_loading
            && let Ok(r) = rx.try_recv()
        {
            match r {
                Ok(v) => self.gfn_titles = Arc::new(v),
                Err(e) => log::info!("cloud: {e}"),
            }
            self.gfn_loading = None;
            self.sugg_for = None;
        }
        if self.sugg_for.as_deref() != Some(self.name.as_str()) {
            self.sugg = if self.service == Service::GeForceNow && !self.name_auto {
                cloud::gfn::suggest(&self.name, &self.gfn_titles, 5)
            } else {
                vec![]
            };
            // Nothing to suggest when the name is already one of them.
            if self.sugg.first().is_some_and(|s| s.eq_ignore_ascii_case(self.name.trim())) {
                self.sugg.clear();
            }
            self.sugg_for = Some(self.name.clone());
        }
    }
}

/// Is `link` a plausible page of `s` (or a store page any service may link to)?
fn link_fits(s: Service, link: &str) -> bool {
    match Service::for_url(link) {
        Some(x) => x == s,
        // Store pages (Steam, Epic …) name the game; fine for any service.
        None => {
            let l = link.to_ascii_lowercase();
            ["steampowered.com", "epicgames.com", "gog.com", "ubisoft.com", "ea.com"].iter().any(|d| l.contains(d))
        }
    }
}

fn color(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// The service picker: one tile per service.
fn service_tiles(ui: &mut Ui, st: &mut AppState, d: &mut CloudDlg, pal: &Pal) {
    let found = d.found.clone();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let w = ((ui.available_width() - 8.0 * 4.0) / 5.0).floor();
        for s in Service::ALL {
            let (r, resp) = ui.allocate_exact_size(vec2(w, 78.0), Sense::click());
            let sel = d.service == s;
            let p = ui.painter();
            let fill = if sel {
                mix(pal.surface_alt, pal.accent, 0.18)
            } else if resp.hovered() {
                pal.hover
            } else {
                pal.surface_alt
            };
            p.rect_filled(r, 8.0, fill);
            if sel {
                p.rect_stroke(r, 8.0, egui::Stroke::new(1.5, pal.accent), egui::StrokeKind::Inside);
            }
            let icon_r = egui::Rect::from_center_size(egui::pos2(r.center().x, r.top() + 27.0), vec2(36.0, 36.0));
            let icon = found.as_ref().and_then(|f| f.apps_for(s).find_map(|a| a.icon.clone()));
            let tex = icon.and_then(|i| st.images.get(&i, crate::images::ImgSize::Thumb));
            if let Some(tex) = tex {
                crate::ui::widgets::paint_contain(p, icon_r, &tex, Color32::WHITE, vec2(0.5, 0.5));
            } else {
                p.rect_filled(icon_r, 8.0, color(s.color()));
                let badge = s.badge();
                let size = if badge.len() > 3 {
                    10.0
                } else if badge.len() > 1 {
                    12.0
                } else {
                    18.0
                };
                p.text(
                    icon_r.center(),
                    egui::Align2::CENTER_CENTER,
                    badge,
                    egui::FontId::proportional(size),
                    Color32::WHITE,
                );
            }
            p.text(
                egui::pos2(r.center().x, r.bottom() - 14.0),
                egui::Align2::CENTER_CENTER,
                s.label(),
                egui::FontId::proportional(12.5),
                if sel { pal.text } else { mix(pal.text, pal.dim, 0.35) },
            );
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                d.set_service(s);
            }
        }
    });
}

fn label(ui: &mut Ui, text: &str, pal: &Pal) {
    ui.label(RichText::new(text).size(13.0).color(pal.dim));
}

/// `flatpak run x --app=…` as one line.
fn spec_line(spec: &crate::model::LaunchSpec) -> String {
    use crate::model::LaunchSpec;
    match spec {
        LaunchSpec::Command { program, args, .. } => std::iter::once(program.clone())
            .chain(args.iter().map(|a| crate::launch::shell_quote(a)))
            .collect::<Vec<_>>()
            .join(" "),
        LaunchSpec::Url(u) => format!("xdg-open {}", crate::launch::shell_quote(u)),
        LaunchSpec::Steam { appid } => format!("steam://rungameid/{appid}"),
    }
}

pub(crate) fn cloud_dialog(
    ctx: &egui::Context,
    st: &mut AppState,
    d: &mut CloudDlg,
    pal: &Pal,
    fam: &egui::FontFamily,
) -> CloudOutcome {
    d.poll(st);
    let mut out = CloudOutcome::Keep;
    let (_, close) = modal(ctx, "ls_cloud_dlg", pal, 580.0, |ui| {
        title(ui, "Add a cloud game", Some("A game you stream from a cloud gaming service."), pal, fam);
        service_tiles(ui, st, d, pal);
        ui.add_space(12.0);
        let svc = d.service;
        egui::Grid::new("ls_cloud_form").num_columns(2).spacing([12.0, 8.0]).min_col_width(84.0).show(ui, |ui| {
            label(ui, "Name", pal);
            ui.vertical(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut d.name)
                        .hint_text("The game's name")
                        .font(egui::FontId::proportional(15.0))
                        .desired_width(f32::INFINITY)
                        .margin(egui::Margin::symmetric(8, 5)),
                );
                if r.changed() {
                    d.name_auto = false;
                }
                if !d.sugg.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
                        for s in d.sugg.clone() {
                            if ui
                                .add(egui::Button::new(RichText::new(&s).size(12.5)).corner_radius(10))
                                .on_hover_text("Use this name (from GeForce NOW's game list)")
                                .clicked()
                            {
                                d.name = s;
                                d.name_auto = false;
                                d.sugg.clear();
                                d.sugg_for = Some(d.name.clone());
                            }
                        }
                    });
                } else if svc == Service::GeForceNow
                    && !d.gfn_titles.is_empty()
                    && cloud::gfn::contains(&d.name, &d.gfn_titles)
                {
                    ui.label(RichText::new("✓ In GeForce NOW's game list").size(12.0).color(pal.dim));
                }
            });
            ui.end_row();

            label(ui, if svc == Service::Other { "Web address" } else { "Game link" }, pal);
            ui.vertical(|ui| {
                let hint = match svc {
                    Service::Other => "https://… (the page you play the game on)".to_string(),
                    s => format!("Optional: paste the game's page from {}", s.name()),
                };
                ui.add(
                    egui::TextEdit::singleline(&mut d.link)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY)
                        .margin(egui::Margin::symmetric(8, 4)),
                );
                let l = d.link.trim();
                if !l.is_empty() && !cloud::is_web(l) {
                    ui.label(RichText::new("A link starts with https://").size(12.0).color(pal.danger));
                }
            });
            ui.end_row();

            label(ui, "Opens with", pal);
            ui.vertical(|ui| match &d.found {
                None => {
                    ui.horizontal(|ui| {
                        lazy_spinner(ui, 14.0, pal.accent);
                        ui.label(RichText::new("Looking for apps and browsers…").size(12.5).color(pal.dim));
                    });
                }
                Some(found) => {
                    let found = found.clone();
                    let url = d.url();
                    let fullscreen = d.fullscreen;
                    opens_with(ui, "ls_cloud_open", svc, &mut d.open, fullscreen, url.as_deref(), &found, pal);
                }
            });
            ui.end_row();
        });

        // ---- More options
        ui.add_space(4.0);
        let text = if d.more { "▾  More options" } else { "▸  More options" };
        if ui
            .add(egui::Button::new(RichText::new(text).size(12.5).color(pal.dim)).frame(false))
            .on_hover_text("Full screen and the command Lodestar runs")
            .clicked()
        {
            d.more = !d.more;
        }
        if d.more {
            ui.indent("ls_cloud_more", |ui| {
                ui.checkbox(&mut d.fullscreen, "Full screen").on_hover_text(
                    "Browser windows (and apps that can) start full screen. \
                         Chrome-based browsers: F11 leaves it. Firefox: close the window with Alt+F4.",
                );
                if let (Some(found), Some(u)) = (&d.found, d.url()) {
                    will_run(ui, &d.entry(), &u, found, pal);
                }
                hint_text(
                    ui,
                    "Lodestar counts play time while the app or browser window it opened stays open. \
                     You can change how it opens later in Edit → Installation.",
                    pal,
                );
            });
        }

        // ---- already in the library
        let dups = st.cloud_duplicates(svc, &d.name);
        if let Some(&dup) = dups.first().filter(|_| !d.allow_dup) {
            ui.add_space(8.0);
            egui::Frame::new()
                .fill(with_alpha(pal.fav, 22))
                .stroke(egui::Stroke::new(1.0, with_alpha(pal.fav, 90)))
                .corner_radius(6)
                .inner_margin(egui::Margin::symmetric(12, 9))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(format!("{} on {} is already in your library", d.name.trim(), svc.name()))
                            .strong()
                            .color(pal.text),
                    );
                    ui.horizontal(|ui| {
                        if ui.button("Show it").clicked() {
                            out = CloudOutcome::Show(dup);
                        }
                        if ui.button("Add anyway").clicked() {
                            d.allow_dup = true;
                        }
                    });
                });
        }

        let blocked = !dups.is_empty() && !d.allow_dup;
        let ok = !d.name.trim().is_empty() && d.url().is_some() && !blocked;
        let buttons = Buttons::primary("Add", ok).cancel("Cancel").disabled_hint(if blocked {
            "Choose what to do about the copy above"
        } else if svc == Service::Other {
            "Fill in the name and the web address"
        } else {
            "Fill in the name"
        });
        let f = footer(ui, pal, buttons, |ui| {
            if let Some(e) = &d.error {
                ui.label(RichText::new(e).size(12.5).color(pal.danger));
            }
        });
        if f.primary {
            let link = Some(d.link.trim().to_string()).filter(|l| !l.is_empty());
            match st.add_cloud_game(cloud::NewCloudGame { name: d.name.trim().to_string(), entry: d.entry(), link }) {
                Some(id) => out = CloudOutcome::Added(id),
                None => d.error = Some("Couldn't add the game".into()),
            }
        }
        if f.cancel {
            out = CloudOutcome::Close;
        }
    });
    d.take_link();
    if close && matches!(out, CloudOutcome::Keep) {
        out = CloudOutcome::Close;
    }
    out
}

/// "GeForce NOW app" / "Chrome window" / "browser tab" for the Automatic entry.
fn how_name(h: &cloud::How, found: &Found) -> String {
    match h {
        cloud::How::App { name, .. } => name.to_string(),
        cloud::How::Window { name, .. } => format!("{name} window"),
        cloud::How::Tab => match found.default_name() {
            Some(n) => format!("{n} tab"),
            None => "browser tab".into(),
        },
    }
}

/// The "Opens with" picker for a cloud game on `svc` opening `url` (Automatic names what it would
/// use right now), then what that does and any warning. Add a cloud game and Edit → Installation.
#[allow(clippy::too_many_arguments)]
pub(super) fn opens_with(
    ui: &mut Ui,
    id: &str,
    svc: Service,
    open: &mut Open,
    fullscreen: bool,
    url: Option<&str>,
    found: &Found,
    pal: &Pal,
) {
    let choices = cloud::choices(svc, found);
    let has_link = url.is_some_and(|u| cloud::is_game_link(svc, u));
    let entry = |o: Open| Entry { service: svc, open: o, fullscreen };
    let auto_name = url.map(|u| how_name(&cloud::plan(&entry(Open::Auto), u, has_link, found).how, found));
    let text_of = |o: &Open, l: &str| match (o, &auto_name) {
        (Open::Auto, Some(n)) => format!("Automatic ({n})"),
        _ => l.to_string(),
    };
    let current =
        choices.iter().find(|(o, _)| o == open).map(|(o, l)| text_of(o, l)).unwrap_or_else(|| "Automatic".into());
    egui::ComboBox::from_id_salt(id)
        .selected_text(current)
        .width(300.0)
        .show_ui(ui, |ui| {
            for (o, l) in &choices {
                ui.selectable_value(open, o.clone(), text_of(o, l));
            }
        })
        .response
        .on_hover_text("Automatic uses the service's app when it is installed, else a browser window");
    if let Some(u) = url {
        let p = cloud::plan(&entry(open.clone()), u, has_link, found);
        hint_text(ui, &cloud::describe(svc, &p, has_link), pal);
        if let Some(w) = cloud::warning(svc, &p, found) {
            ui.label(RichText::new(w).size(12.0).color(pal.fav));
        }
    }
}

/// "Will run: <command>" for `entry` opening `url`.
pub(super) fn will_run(ui: &mut Ui, entry: &Entry, url: &str, found: &Found, pal: &Pal) {
    let p = cloud::plan(entry, url, cloud::is_game_link(entry.service, url), found);
    ui.add(
        egui::Label::new(
            RichText::new(format!("Will run: {}", spec_line(&p.spec))).size(12.0).monospace().color(pal.dim),
        )
        .wrap(),
    );
}

/// A cloud game's settings in Edit → Installation ("Opens with", Full screen), saved with the
/// dialog (`AppState::set_cloud_entry`).
pub(crate) struct CloudEdit {
    pub entry: Entry,
    pub initial: Entry,
    /// The page it opens.
    pub url: Option<String>,
    found: Option<Arc<Found>>,
}

impl CloudEdit {
    pub fn of(g: &crate::model::Game) -> Option<CloudEdit> {
        let entry = Entry::of(g)?;
        Some(CloudEdit { url: cloud::game_url(g, &entry), initial: entry.clone(), entry, found: None })
    }

    pub fn changed(&self) -> bool {
        self.entry != self.initial
    }

    /// Edit → Installation for a cloud game: the service, Opens with, Full screen, Will run.
    pub fn rows(&mut self, ui: &mut Ui, pal: &Pal, row: impl Fn(&mut Ui, &str, &mut dyn FnMut(&mut Ui))) {
        // Apps and browsers installed now (a few dozen file checks, remembered for a minute).
        let found = self.found.get_or_insert_with(cloud::published).clone();
        let svc = self.entry.service;
        row(ui, "Service", &mut |ui| {
            ui.label(RichText::new(svc.name()).color(pal.text));
        });
        let url = self.url.clone();
        let fullscreen = self.entry.fullscreen;
        row(ui, "Opens with", &mut |ui| {
            ui.vertical(|ui| {
                opens_with(ui, "ls_edit_cloud_open", svc, &mut self.entry.open, fullscreen, url.as_deref(), &found, pal)
            });
        });
        // A tab in the browser that is already open can't start full screen.
        let tab = url.as_deref().is_some_and(|u| {
            let has_link = cloud::is_game_link(svc, u);
            matches!(cloud::plan(&self.entry, u, has_link, &found).how, cloud::How::Tab)
        });
        row(ui, "Full screen", &mut |ui| {
            ui.add_enabled(!tab, egui::Checkbox::without_text(&mut self.entry.fullscreen))
                .on_hover_text(
                    "Browser windows (and apps that can) start full screen. Chrome-based browsers: F11 leaves it. \
                     Firefox: close the window with Alt+F4.",
                )
                .on_disabled_hover_text("A browser tab opens in the browser window you already have: no full screen.");
        });
        if let Some(u) = &url {
            row(ui, "", &mut |ui| {
                ui.vertical(|ui| will_run(ui, &self.entry, u, &found, pal));
            });
        }
    }
}
