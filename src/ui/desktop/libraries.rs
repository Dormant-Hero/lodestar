//! Settings → Libraries: one row per library integration (Steam, the stores Heroic manages, the
//! Windows launchers found in Wine prefixes, game managers, Minecraft, Moonlight, desktop
//! entries, add-ons) with the last scan's result, an on / off switch and, where it applies,
//! manual paths. Detection is automatic; the paths only add to what is found.

use super::Pal;
use super::add::{Pick, PickKind};
use super::dialogs::hint_text;
use super::settings::{SettingsDlg, expand_tilde};
use crate::app::AppState;
use crate::model::Source;
use crate::ui::widgets::{Icon, lazy_spinner, mix, paint_icon, toggle_switch, with_alpha};
use egui::{Align, Layout, RichText, Sense, Ui, pos2, vec2};
use std::path::PathBuf;

/// Manual paths a row can add to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PathTarget {
    SteamRoots,
    DesktopDirs,
    PrefixDirs,
}

/// Extra settings shown when a row is expanded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Extra {
    None,
    /// Extra .desktop folders and the ignore list.
    Desktop,
    /// "Manage add-ons…".
    Addons,
    /// "Set up emulation…" (Settings → Emulation).
    Emulation,
    /// RomM server: address, token, Test connection (romm_ui.rs).
    Romm,
    /// Extra Steam libraries and the optional Steam Web API key.
    Steam,
}

struct Row {
    source: Source,
    name: &'static str,
    /// Where it reads from ("Heroic, Legendary").
    via: &'static str,
    extra: Extra,
}

struct Group {
    title: &'static str,
    hint: Option<&'static str>,
    rows: &'static [Row],
    /// A path list for the whole group (Wine prefixes for the Windows launchers).
    footer: Option<PathTarget>,
}

const fn row(source: Source, name: &'static str, via: &'static str, extra: Extra) -> Row {
    Row { source, name, via, extra }
}

const GROUPS: &[Group] = &[
    Group {
        title: "Stores",
        hint: None,
        rows: &[
            row(Source::Steam, "Steam", "Steam client libraries, owned games from its local data", Extra::Steam),
            row(Source::Epic, "Epic Games", "Heroic, Legendary", Extra::None),
            row(Source::Gog, "GOG", "Heroic", Extra::None),
            row(Source::Amazon, "Amazon Games", "Heroic, nile", Extra::None),
            row(Source::Heroic, "Heroic sideloaded", "Apps and games added to Heroic by hand", Extra::None),
            row(Source::Itch, "itch.io", "itch app", Extra::None),
        ],
        footer: None,
    },
    Group {
        title: "Windows launchers",
        hint: Some(
            "Installed games of these launchers, found in Wine prefixes (Lutris, Bottles, Faugus, \
             Heroic, Steam shortcuts, ~/Games). They start through the launcher in their prefix.",
        ),
        rows: &[
            row(Source::BattleNet, "Battle.net", "Wine prefixes", Extra::None),
            row(Source::Ea, "EA app", "Wine prefixes", Extra::None),
            row(Source::Ubisoft, "Ubisoft Connect", "Wine prefixes", Extra::None),
            row(Source::Rockstar, "Rockstar Games Launcher", "Wine prefixes", Extra::None),
            row(Source::Riot, "Riot Client", "Wine prefixes", Extra::None),
            row(Source::HoYoPlay, "HoYoPlay", "Wine prefixes", Extra::None),
        ],
        footer: Some(PathTarget::PrefixDirs),
    },
    Group {
        title: "Game managers",
        hint: None,
        rows: &[
            row(Source::Lutris, "Lutris", "Lutris library", Extra::None),
            row(Source::Bottles, "Bottles", "Programs in your bottles", Extra::None),
            row(Source::Faugus, "Faugus Launcher", "Faugus games", Extra::None),
        ],
        footer: None,
    },
    Group {
        title: "Emulation",
        hint: None,
        rows: &[
            row(Source::Emulator, "Emulators", "ROM folders set up in Settings → Emulation", Extra::Emulation),
            row(Source::Romm, "RomM server", "Games on your own RomM server", Extra::Romm),
        ],
        footer: None,
    },
    Group {
        title: "More",
        hint: None,
        rows: &[
            row(Source::Minecraft, "Minecraft", "Prism Launcher, MultiMC, ATLauncher instances", Extra::None),
            row(Source::Moonlight, "Moonlight", "Apps of paired Sunshine / Apollo hosts", Extra::None),
            row(
                Source::Desktop,
                "Desktop entries",
                "Installed apps' .desktop shortcuts that look like games",
                Extra::Desktop,
            ),
            row(Source::Addon, "Add-ons", "Games from external library add-ons", Extra::Addons),
        ],
        footer: None,
    },
];

/// The built-in library integrations, for the Add-ons window: (source, name, where it reads from).
pub(crate) fn builtin_libraries() -> impl Iterator<Item = (Source, &'static str, &'static str)> {
    GROUPS.iter().flat_map(|g| g.rows.iter()).filter(|r| r.source != Source::Addon).map(|r| (r.source, r.name, r.via))
}

/// Every source that has a row (used by tests: no store is left out).
#[cfg(test)]
pub(crate) fn listed_sources() -> Vec<Source> {
    GROUPS.iter().flat_map(|g| g.rows.iter().map(|r| r.source)).collect()
}

/// What a row's status says.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Status {
    Off,
    Scanning,
    Found(usize),
    NotFound,
    Error(String),
    /// No scan this session: what the library holds.
    InLibrary(usize),
    NotScanned,
}

pub(crate) fn status_for(
    s: Source,
    enabled: bool,
    scanning: bool,
    last: Option<&[(Source, usize, Option<String>)]>,
    in_library: &[(Source, usize)],
) -> Status {
    if !enabled {
        return Status::Off;
    }
    if scanning {
        return Status::Scanning;
    }
    if let Some(last) = last {
        return match last.iter().find(|(src, _, _)| *src == s) {
            Some((_, _, Some(e))) => Status::Error(e.clone()),
            Some((_, 0, None)) => Status::NotFound,
            Some((_, n, None)) => Status::Found(*n),
            // Enabled since the last scan.
            None => Status::NotScanned,
        };
    }
    match in_library.iter().find(|(src, _)| *src == s) {
        Some(&(_, n)) if n > 0 => Status::InLibrary(n),
        _ => Status::NotScanned,
    }
}

fn games(n: usize) -> String {
    if n == 1 { "1 game".into() } else { format!("{n} games") }
}

/// Draws the tab. Returns (config changed, scan settings changed, open the add-ons window).
pub(crate) fn libraries(
    ui: &mut Ui,
    st: &mut AppState,
    d: &mut SettingsDlg,
    pal: &Pal,
    in_library: &[(Source, usize)],
) -> (bool, bool, bool) {
    // Room for the scroll bar on the right (it floats over the content).
    egui::Frame::new()
        .inner_margin(egui::Margin { left: 0, right: 14, top: 0, bottom: 0 })
        .show(ui, |ui| {
            if let Some(page) = &mut d.hidden_page {
                if super::hidden_ui::hidden_page(ui, st, page, pal) {
                    d.hidden_page = None;
                }
                return (false, false, false);
            }
            if let Some(page) = &mut d.missing_page {
                if super::missing_ui::missing_page(ui, st, page, pal) {
                    d.missing_page = None;
                }
                return (false, false, false);
            }
            libraries_inner(ui, st, d, pal, in_library)
        })
        .inner
}

/// What a part of the Libraries page changed or asked for.
#[derive(Clone, Copy, Default)]
struct Changes {
    /// A setting changed.
    config: bool,
    /// A setting that needs a rescan changed.
    scan: bool,
    /// "Manage add-ons…" was clicked.
    open_addons: bool,
}

impl std::ops::BitOrAssign for Changes {
    fn bitor_assign(&mut self, o: Changes) {
        self.config |= o.config;
        self.scan |= o.scan;
        self.open_addons |= o.open_addons;
    }
}

fn libraries_inner(
    ui: &mut Ui,
    st: &mut AppState,
    d: &mut SettingsDlg,
    pal: &Pal,
    in_library: &[(Source, usize)],
) -> (bool, bool, bool) {
    let mut c = Changes::default();
    scan_header(ui, st, d, pal);
    ui.add_space(6.0);
    c.config |= ui.checkbox(&mut st.config.scan.on_startup, "Scan for games on startup").changed();
    c.scan |= ui
        .checkbox(&mut st.config.scan.include_uninstalled, "Include owned games that are not installed")
        .on_hover_text(
            "Steam, Epic, GOG, Amazon and itch.io know what you own. Those games show dimmed, \
             with an Install button that hands over to the store's app.",
        )
        .changed();
    let last: Option<Vec<(Source, usize, Option<String>)>> = st.scan.last.as_ref().map(|l| l.per_source.clone());
    for group in GROUPS {
        c |= library_group(ui, st, d, group, last.as_deref(), in_library, pal);
    }

    ui.add_space(14.0);
    ui.label(RichText::new("HIDDEN & EXCLUDED").size(11.0).strong().color(pal.dim).extra_letter_spacing(0.8));
    ui.add_space(4.0);
    if super::hidden_ui::summary_card(ui, st, pal) {
        d.hidden_page = Some(Default::default());
    }
    ui.add_space(4.0);
    c.config |= ui.checkbox(&mut st.config.desktop.show_hidden, "Show hidden games in the library").changed();
    ui.add_space(14.0);
    ui.label(RichText::new("MISSING GAMES").size(11.0).strong().color(pal.dim).extra_letter_spacing(0.8));
    ui.add_space(4.0);
    if super::missing_ui::summary(ui, st, pal) {
        d.missing_page = Some(Default::default());
    }
    ui.add_space(6.0);
    if c.scan {
        d.scan_changed = true;
    }
    (c.config || c.scan, c.scan, c.open_addons)
}

/// "Libraries", what the last scan found, and Rescan now.
fn scan_header(ui: &mut Ui, st: &mut AppState, d: &mut SettingsDlg, pal: &Pal) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new("Libraries").size(15.0).strong().color(pal.text));
            let line = match &st.scan.last {
                _ if st.scan.running && st.scan.addons_pending => "Waiting for library add-ons…".to_string(),
                _ if st.scan.running => "Scanning your libraries…".to_string(),
                Some(l) => format!(
                    "Last scan: {} in {:.1} s{}",
                    games(l.total),
                    l.elapsed.as_secs_f32(),
                    if l.added > 0 { format!(", {} new", l.added) } else { String::new() }
                ),
                None => "Found automatically. Switch off what you don't use.".to_string(),
            };
            ui.label(RichText::new(line).size(12.0).color(pal.dim));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let label = if st.scan.running { "Scanning…" } else { "Rescan now" };
            if ui.add_enabled(!st.scan.running, egui::Button::new(label).min_size(vec2(104.0, 30.0))).clicked() {
                st.save_config();
                st.rescan();
                d.scan_changed = false;
            }
            if st.scan.running {
                lazy_spinner(ui, 16.0, pal.dim);
            }
        });
    });
}

/// One group of libraries (stores, launchers …): a card with a row per library, expandable
/// for its folders / options, and the group's own folders at the bottom.
fn library_group(
    ui: &mut Ui,
    st: &mut AppState,
    d: &mut SettingsDlg,
    group: &Group,
    last: Option<&[(Source, usize, Option<String>)]>,
    in_library: &[(Source, usize)],
    pal: &Pal,
) -> Changes {
    let mut c = Changes::default();
    ui.add_space(14.0);
    ui.label(RichText::new(group.title.to_uppercase()).size(11.0).strong().color(pal.dim).extra_letter_spacing(0.8));
    if let Some(h) = group.hint {
        hint_text(ui, h, pal);
    }
    ui.add_space(4.0);
    egui::Frame::new()
        .fill(mix(pal.surface, pal.surface_alt, 0.55))
        .stroke(egui::Stroke::new(1.0, with_alpha(pal.border, 160)))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(0, 2))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            for (i, r) in group.rows.iter().enumerate() {
                if i > 0 {
                    row_separator(ui, pal);
                }
                let enabled = st.config.source_enabled(r.source);
                let status = status_for(r.source, enabled, st.scan.scanning(r.source), last, in_library);
                let expandable = r.extra != Extra::None;
                let expanded = expandable && d.expanded.contains(&r.source);
                let out = source_row(ui, r, enabled, &status, expandable, expanded, pal);
                if let Some(on) = out.toggled {
                    let key = r.source.key().to_string();
                    st.config.scan.disabled_sources.retain(|k| *k != key);
                    if !on {
                        st.config.scan.disabled_sources.push(key);
                    }
                    c.scan = true;
                }
                if out.expand_clicked {
                    if expanded {
                        d.expanded.retain(|s| *s != r.source);
                    } else {
                        d.expanded.push(r.source);
                    }
                }
                if expanded {
                    egui::Frame::new()
                        .inner_margin(egui::Margin { left: 40, right: 14, top: 0, bottom: 10 })
                        .show(ui, |ui| c |= expanded_row(ui, st, d, r.extra, pal));
                }
            }
            if let Some(t) = group.footer {
                row_separator(ui, pal);
                egui::Frame::new().inner_margin(egui::Margin { left: 14, right: 14, top: 8, bottom: 10 }).show(
                    ui,
                    |ui| {
                        ui.spacing_mut().item_spacing.y = 4.0;
                        c.scan |= paths_block(ui, st, d, t, pal);
                    },
                );
            }
        });
    c
}

/// What an expanded library row shows: its folders, words to ignore, or a way on.
fn expanded_row(ui: &mut Ui, st: &mut AppState, d: &mut SettingsDlg, extra: Extra, pal: &Pal) -> Changes {
    let mut c = Changes::default();
    ui.spacing_mut().item_spacing.y = 4.0;
    match extra {
        Extra::Steam => {
            c.scan |= paths_block(ui, st, d, PathTarget::SteamRoots, pal);
            ui.add_space(8.0);
            c.config |= steam_key_block(ui, st, pal);
        }
        Extra::Desktop => {
            c.scan |= paths_block(ui, st, d, PathTarget::DesktopDirs, pal);
            ui.add_space(6.0);
            c.scan |= ignore_words(ui, st, d, pal);
        }
        Extra::Addons => {
            hint_text(
                ui,
                "Library add-ons are small programs that list games \
                 from somewhere Lodestar doesn't know.",
                pal,
            );
            ui.add_space(4.0);
            if ui.button("Manage add-ons…").clicked() {
                c.open_addons = true;
            }
        }
        Extra::Emulation => {
            hint_text(
                ui,
                "Console games from ROM folders, started with the \
                 emulators installed here.",
                pal,
            );
            ui.add_space(4.0);
            if ui.button("Set up emulation…").clicked() {
                d.tab = super::settings::Tab::Emulation;
            }
        }
        Extra::Romm => {
            c.config |= super::romm_ui::settings_block(ui, st, pal);
        }
        Extra::None => {}
    }
    c
}

const STEAM_KEY_URL: &str = "https://steamcommunity.com/dev/apikey";

/// The optional Steam Web API key (the same one as in Settings → Achievements).
fn steam_key_block(ui: &mut Ui, st: &mut AppState, pal: &Pal) -> bool {
    ui.label(RichText::new("Steam Web API key (optional)").size(13.0).strong().color(pal.text));
    hint_text(
        ui,
        "Lets Lodestar list every Steam game you own, including ones you never started on this PC. \
         Achievements use the same key.",
        pal,
    );
    if super::dialogs::link_text(ui, "Get a key on steamcommunity.com", STEAM_KEY_URL, pal) {
        st.open_url(STEAM_KEY_URL);
    }
    let w = (ui.available_width() - 200.0).clamp(160.0, 340.0);
    let ch = super::sources_ui::value_field(
        ui,
        "Web API key",
        &mut st.config.achievements.steam_web_api_key,
        "optional",
        Some(&mut false),
        w,
    );
    let status = if st.steam_names.running {
        Some("Asking Steam for your games…".to_string())
    } else {
        st.steam_names.status.clone()
    };
    if let Some(s) = status.filter(|_| crate::config::secret(&st.config.achievements.steam_web_api_key).is_some()) {
        hint_text(ui, &s, pal);
    }
    ch
}

fn row_separator(ui: &mut Ui, pal: &Pal) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(
        (r.left() + 14.0)..=(r.right() - 14.0),
        r.center().y,
        egui::Stroke::new(1.0, with_alpha(pal.border, 200)),
    );
}

struct RowOut {
    toggled: Option<bool>,
    expand_clicked: bool,
}

/// One integration: chevron (when it has settings) · name + where it reads from · status ·
/// switch. Clicking the row (outside the switch) expands it.
fn source_row(
    ui: &mut Ui,
    r: &Row,
    enabled: bool,
    status: &Status,
    expandable: bool,
    expanded: bool,
    pal: &Pal,
) -> RowOut {
    let mut out = RowOut { toggled: None, expand_clicked: false };
    let h = 46.0;
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let id = ui.scope_id().with(("ls_lib_row", r.source.key()));
    let row_resp = ui.interact(rect, id, if expandable { Sense::click() } else { Sense::hover() });
    // switch first (it sits on top of the row's click area)
    let sw_h = 20.0;
    let sw_rect = egui::Rect::from_min_size(
        pos2(rect.right() - 14.0 - sw_h * 1.8, rect.center().y - sw_h / 2.0),
        vec2(sw_h * 1.8, sw_h),
    );
    let mut on = enabled;
    // A child Ui (not a scope): placing it must not move the parent's cursor.
    let sw = toggle_switch(&mut ui.new_child(egui::UiBuilder::new().max_rect(sw_rect)), &mut on, sw_h, pal.accent);
    let sw = sw.on_hover_text(if enabled {
        format!("Scanning {}. Click to switch it off.", r.name)
    } else {
        format!("{} is off. Click to scan it again.", r.name)
    });
    if sw.changed() {
        out.toggled = Some(on);
    }
    if expandable && row_resp.clicked() && !sw.hovered() {
        out.expand_clicked = true;
    }

    let p = ui.painter();
    if expandable && row_resp.hovered() && !sw.hovered() {
        p.rect_filled(rect.shrink2(vec2(3.0, 1.0)), 6.0, with_alpha(pal.text, 8));
    }
    let left = rect.left() + 14.0;
    if expandable {
        let ir = egui::Rect::from_center_size(pos2(left + 7.0, rect.center().y), vec2(13.0, 13.0));
        paint_icon(p, ir, if expanded { Icon::ChevronDown } else { Icon::ChevronRight }, pal.dim);
    }
    let text_x = left + 26.0;
    // status (right, before the switch)
    let status_right = sw_rect.left() - 14.0;
    let (status_text, status_color, status_icon) = match status {
        Status::Off => ("Off".to_string(), mix(pal.dim, pal.bg, 0.25), None),
        Status::Scanning => ("Scanning…".to_string(), pal.dim, None),
        Status::Found(n) => (games(*n), mix(pal.text, pal.dim, 0.2), None),
        Status::NotFound => ("No games".to_string(), pal.dim, None),
        Status::Error(_) => ("Error".to_string(), pal.danger, Some(Icon::Warning)),
        Status::InLibrary(n) => (format!("{} in library", games(*n)), pal.dim, None),
        Status::NotScanned => ("No games yet".to_string(), pal.dim, None),
    };
    let sfont = egui::FontId::proportional(12.5);
    let sg = p.layout_no_wrap(status_text, sfont, status_color);
    let sx = status_right - sg.size().x;
    let status_rect = egui::Rect::from_min_size(
        pos2(sx - if status_icon.is_some() { 20.0 } else { 0.0 }, rect.center().y - 10.0),
        vec2(sg.size().x + if status_icon.is_some() { 20.0 } else { 0.0 }, 20.0),
    );
    if matches!(status, Status::Scanning) {
        // a small spinner in front of the text
        let c = pal.dim;
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(egui::Rect::from_center_size(pos2(sx - 12.0, rect.center().y), vec2(14.0, 14.0))),
        );
        lazy_spinner(&mut child, 14.0, c);
    }
    let p = ui.painter();
    if let Some(icon) = status_icon {
        paint_icon(
            p,
            egui::Rect::from_center_size(pos2(sx - 10.0, rect.center().y), vec2(14.0, 14.0)),
            icon,
            status_color,
        );
    }
    p.galley(pos2(sx, rect.center().y - sg.size().y / 2.0), sg, status_color);
    // name + via (truncated to the room left)
    let name_w = (status_rect.left() - 16.0 - text_x).max(40.0);
    let name_color = if enabled { pal.text } else { pal.dim };
    let ng = crate::ui::widgets::truncated(p, r.name, egui::FontId::proportional(14.0), name_color, name_w);
    let vg = crate::ui::widgets::truncated(
        p,
        r.via,
        egui::FontId::proportional(11.5),
        mix(pal.dim, pal.bg, if enabled { 0.0 } else { 0.25 }),
        name_w,
    );
    let total = ng.size().y + 1.0 + vg.size().y;
    let y0 = rect.center().y - total / 2.0;
    p.galley(pos2(text_x, y0), ng.clone(), name_color);
    p.galley(pos2(text_x, y0 + ng.size().y + 1.0), vg, pal.dim);
    if let Status::Error(e) = status {
        ui.interact(status_rect, id.with("err"), Sense::hover()).on_hover_text(format!("The last scan failed:\n{e}"));
    } else if matches!(status, Status::NotFound) {
        ui.interact(status_rect, id.with("nf"), Sense::hover()).on_hover_text(
            "The last scan found no games here: the launcher isn't installed, or has no games installed yet.",
        );
    }
    out
}

fn target_list(st: &mut AppState, t: PathTarget) -> &mut Vec<PathBuf> {
    let s = &mut st.config.scan;
    match t {
        PathTarget::SteamRoots => &mut s.extra_steam_roots,
        PathTarget::DesktopDirs => &mut s.extra_desktop_dirs,
        PathTarget::PrefixDirs => &mut s.extra_prefix_dirs,
    }
}

fn target_texts(t: PathTarget) -> (&'static str, &'static str, &'static str) {
    match t {
        PathTarget::SteamRoots => (
            "Extra Steam libraries",
            "/mnt/games/SteamLibrary",
            "Steam's own library folders are always found; add ones it doesn't list.",
        ),
        PathTarget::DesktopDirs => (
            "Extra folders with .desktop shortcuts",
            "~/Games/shortcuts",
            "The usual application folders are always scanned.",
        ),
        PathTarget::PrefixDirs => (
            "Extra Wine prefixes",
            "~/Games/battlenet  or a folder of prefixes",
            "Prefixes of Lutris, Bottles, Faugus, Heroic, Steam and ~/Games are found by \
             themselves. Add a prefix, or a folder that holds several.",
        ),
    }
}

/// Title, hint, the list with Remove buttons, and an add row (text field, Add, Browse…).
fn paths_block(ui: &mut Ui, st: &mut AppState, d: &mut SettingsDlg, t: PathTarget, pal: &Pal) -> bool {
    let (title, hint, help) = target_texts(t);
    ui.label(RichText::new(title).size(13.0).strong().color(pal.text));
    hint_text(ui, help, pal);
    ui.add_space(2.0);
    let mut ch = false;
    let mut remove = None;
    {
        let list = target_list(st, t);
        for (i, p) in list.iter().enumerate() {
            ui.horizontal(|ui| {
                paint_icon(
                    ui.painter(),
                    egui::Rect::from_min_size(ui.cursor().min + vec2(0.0, 2.0), vec2(14.0, 14.0)),
                    Icon::Folder,
                    pal.dim,
                );
                ui.add_space(20.0);
                let exists = p.exists();
                let text = RichText::new(p.display().to_string()).monospace().size(12.5).color(if exists {
                    pal.text
                } else {
                    pal.danger
                });
                let r = ui.add(egui::Label::new(text).truncate());
                if !exists {
                    r.on_hover_text("This folder doesn't exist (any more).");
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(i);
                }
            });
        }
    }
    if let Some(i) = remove {
        target_list(st, t).remove(i);
        ch = true;
    }
    let new = d.new_path.entry(t).or_default();
    let mut add: Option<PathBuf> = None;
    let mut browse = false;
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(new)
                .id_salt(("ls_path_add", t as u8))
                .hint_text(hint)
                .desired_width((ui.available_width() - 150.0).clamp(160.0, 340.0))
                .font(egui::TextStyle::Monospace),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Add").clicked() || enter) && !new.trim().is_empty() {
            add = Some(expand_tilde(new.trim()));
            new.clear();
        }
        let picking = d.pick.as_ref().is_some_and(|(pt, _)| *pt == t);
        if ui.add_enabled(!picking, egui::Button::new("Browse…")).on_hover_text("Pick a folder").clicked() {
            browse = true;
        }
    });
    if browse {
        let start = target_list(st, t).last().cloned();
        d.pick = Some((t, Pick::start(ui.ctx(), PickKind::Folder, title, start)));
    }
    if let Some(p) = add {
        let list = target_list(st, t);
        if !list.contains(&p) {
            list.push(p);
            ch = true;
        }
    }
    ch
}

/// A folder picked with Browse… arrived: add it to its list. Returns true when added.
pub(crate) fn poll_pick(st: &mut AppState, d: &mut SettingsDlg) -> bool {
    let Some((t, pick)) = &d.pick else {
        return false;
    };
    let Some(paths) = pick.poll() else {
        return false;
    };
    let t = *t;
    d.pick = None;
    let mut ch = false;
    let list = target_list(st, t);
    for p in paths {
        if !list.contains(&p) {
            list.push(p);
            ch = true;
        }
    }
    ch
}

fn ignore_words(ui: &mut Ui, st: &mut AppState, d: &mut SettingsDlg, pal: &Pal) -> bool {
    let mut ch = false;
    ui.label(RichText::new("Ignore entries whose name contains").size(13.0).strong().color(pal.text));
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for (i, w) in st.config.scan.desktop_ignore.iter().enumerate() {
            if ui
                .add(egui::Button::new(RichText::new(format!("{w}  ×")).size(12.5)).corner_radius(12))
                .on_hover_text("Remove")
                .clicked()
            {
                remove = Some(i);
            }
        }
    });
    if let Some(i) = remove {
        st.config.scan.desktop_ignore.remove(i);
        ch = true;
    }
    ui.horizontal(|ui| {
        let r = ui.add(egui::TextEdit::singleline(&mut d.new_ignore).hint_text("word").desired_width(200.0));
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Add").clicked() || enter) && !d.new_ignore.trim().is_empty() {
            let w = d.new_ignore.trim().to_lowercase();
            if !st.config.scan.desktop_ignore.contains(&w) {
                st.config.scan.desktop_ignore.push(w);
                ch = true;
            }
            d.new_ignore.clear();
        }
    });
    ch
}
