//! Settings → Metadata: the artwork source list (on/off, priority) and the keys / accounts of
//! the sources that need the user's own credentials. Edits `st.config.metadata`
//! in place; the settings dialog saves (debounced) and hands the config to the services.

use super::Pal;
use super::dialogs::hint_text;
use crate::app::AppState;
use crate::art::sources::ordered_sources_for;
use crate::art::{ArtList, SourceKind};
use crate::config::MetadataConfig;
use egui::{RichText, Ui, vec2};

/// Move `k` one place up (or down) in `list`'s priority order.
pub(crate) fn move_source(cfg: &mut MetadataConfig, list: ArtList, k: SourceKind, up: bool) {
    let mut order: Vec<SourceKind> = ordered_sources_for(cfg, list).into_iter().map(|(k, _)| k).collect();
    let Some(i) = order.iter().position(|o| *o == k) else {
        return;
    };
    let j = if up { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < order.len()) };
    if let Some(j) = j {
        order.swap(i, j);
        *list.order_mut(cfg) = order.iter().map(|k| k.name().to_string()).collect();
    }
}

pub(crate) fn set_source_enabled(cfg: &mut MetadataConfig, list: ArtList, k: SourceKind, on: bool) {
    let mut off = list.off(cfg);
    off.retain(|n| !n.eq_ignore_ascii_case(k.name()));
    if !on {
        off.push(k.name().to_string());
    }
    list.set_off(cfg, off);
}

/// Which list Settings → Metadata shows (kept while the dialog is open).
fn list_id() -> egui::Id {
    egui::Id::NULL.with("ls_source_list_tab")
}

fn section_id(k: SourceKind) -> egui::Id {
    egui::Id::NULL.with(("ls_source_keys", k.name()))
}

fn scroll_id() -> egui::Id {
    egui::Id::NULL.with("ls_source_scroll_to")
}

/// "Set it up below" was clicked: bring that source's key section into view (next frame).
fn request_scroll(ctx: &egui::Context, k: SourceKind) {
    ctx.data_mut(|d| d.insert_temp(scroll_id(), k.name().to_string()));
}

/// Is `k`'s key section to be scrolled into view now? (Consumes the request.)
pub(super) fn take_scroll(ui: &Ui, k: SourceKind) -> bool {
    let hit = ui.ctx().data(|d| d.get_temp::<String>(scroll_id())).is_some_and(|n| n == k.name());
    if hit {
        ui.ctx().data_mut(|d| d.remove::<String>(scroll_id()));
    }
    hit
}

/// The two source lists (PC games / emulated games, one shown at a time) and what is left of
/// the APIs' limits. True when something changed.
pub(super) fn source_list(ui: &mut Ui, st: &mut AppState, pal: &Pal) -> bool {
    let mut list = ui.ctx().data(|d| d.get_temp::<ArtList>(list_id())).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for l in ArtList::ALL {
            if list_tab(ui, l.label(), list == l, pal).clicked() {
                list = l;
            }
        }
    });
    ui.ctx().data_mut(|d| d.insert_temp(list_id(), list));
    hint_text(
        ui,
        match list {
            ArtList::Pc => "Used for games from stores, launchers and folders of PC games.",
            ArtList::Emulated => {
                "Used for emulated games and games on a console platform. The PC stores are off here: \
                 they rarely know console games."
            }
        },
        pal,
    );
    ui.add_space(2.0);
    let ch = source_rows(ui, st, list, pal);
    limits_lines(ui, st, pal);
    ch
}

/// An underlined tab (like Edit Game Details' tabs).
fn list_tab(ui: &mut Ui, label: &str, active: bool, pal: &Pal) -> egui::Response {
    let gal = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(13.5), pal.text);
    let (rect, resp) = ui.allocate_exact_size(vec2(gal.size().x + 22.0, 28.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if resp.hovered() && !active {
            p.rect_filled(rect.shrink2(vec2(0.0, 2.0)), 5.0, pal.hover);
        }
        let col = if active { pal.text } else { pal.dim };
        p.galley(egui::pos2(rect.left() + 11.0, rect.center().y - gal.size().y / 2.0), gal, col);
        if active {
            p.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(rect.left() + 6.0, rect.bottom() - 2.5),
                    egui::pos2(rect.right() - 6.0, rect.bottom()),
                ),
                1.5,
                pal.accent,
            );
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// "What's left": one line per source whose API counts requests.
fn limits_lines(ui: &mut Ui, st: &AppState, pal: &Pal) {
    let lines = st.art.limits_summary();
    ui.add_space(6.0);
    ui.label(RichText::new("Usage limits").size(13.0).strong().color(pal.text));
    if lines.is_empty() {
        hint_text(
            ui,
            "Lodestar keeps to every site's limits by itself (libretro: 60 lookups an hour through GitHub; \
             TheGamesDB: its monthly allowance). What is left shows here \
             once a source has been asked.",
            pal,
        );
        return;
    }
    for (k, text) in lines {
        let name = if k == SourceKind::Libretro { "libretro (GitHub)" } else { k.name() };
        ui.label(RichText::new(format!("{name}: {text}")).size(12.0).color(pal.dim));
    }
}

/// The rows of one list. True when something changed.
fn source_rows(ui: &mut Ui, st: &mut AppState, list: ArtList, pal: &Pal) -> bool {
    let mut ch = false;
    let infos = st.art.sources_for(list);
    let rows = ordered_sources_for(&st.config.metadata, list);
    let n = rows.len();
    let mut action: Option<(SourceKind, Option<bool>, Option<bool>)> = None;
    egui::Frame::new().fill(pal.surface).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        for (i, (k, on)) in rows.iter().copied().enumerate() {
            let info = infos.iter().find(|s| s.kind == k);
            ui.horizontal(|ui| {
                ui.set_min_height(26.0);
                let mut v = on;
                ui.allocate_ui_with_layout(vec2(132.0, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_min_width(132.0);
                    if ui.checkbox(&mut v, k.name()).on_hover_text(k.blurb()).changed() {
                        action = Some((k, Some(v), None));
                    }
                });
                let right = 2.0 * 26.0 + 8.0;
                let w = (ui.available_width() - right).max(60.0);
                let configured = k.configured(&st.config.metadata);
                let rejected = info.is_some_and(|s| s.reason.as_deref().is_some_and(|r| r.contains("rejected")));
                ui.allocate_ui_with_layout(vec2(w, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    if on && (rejected || !configured) {
                        let (text, color) = if rejected {
                            ("Key rejected — check it below", pal.danger)
                        } else {
                            (needs_short(k), pal.accent)
                        };
                        let r = ui.add(
                            egui::Label::new(RichText::new(text).size(12.0).color(color))
                                .truncate()
                                .sense(egui::Sense::click()),
                        );
                        if r.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            let mut s = egui::collapsing_header::CollapsingState::load_with_default_open(
                                ui.ctx(),
                                section_id(k),
                                false,
                            );
                            s.set_open(true);
                            s.store(ui.ctx());
                            request_scroll(ui.ctx(), k);
                        }
                    } else {
                        let c = if on { pal.dim } else { crate::ui::widgets::mix(pal.dim, pal.bg, 0.4) };
                        ui.add(egui::Label::new(RichText::new(k.blurb()).size(12.0).color(c)).truncate());
                    }
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(i + 1 < n, egui::Button::new("↓").small().min_size(vec2(24.0, 22.0)))
                        .on_hover_text("Ask later")
                        .clicked()
                    {
                        action = Some((k, None, Some(false)));
                    }
                    if ui
                        .add_enabled(i > 0, egui::Button::new("↑").small().min_size(vec2(24.0, 22.0)))
                        .on_hover_text("Ask earlier")
                        .clicked()
                    {
                        action = Some((k, None, Some(true)));
                    }
                });
            });
        }
    });
    if let Some((k, enable, up)) = action {
        if let Some(on) = enable {
            set_source_enabled(&mut st.config.metadata, list, k, on);
        }
        if let Some(up) = up {
            move_source(&mut st.config.metadata, list, k, up);
        }
        ch = true;
    }
    ch
}

fn needs_short(k: SourceKind) -> &'static str {
    match k {
        SourceKind::SteamGridDb | SourceKind::TheGamesDb => "Needs an API key — set it up below",
        SourceKind::Igdb => "Needs a Twitch app — set it up below",
        _ => "",
    }
}

/// One text field bound to an optional config value (empty = None). Stored as typed; users of
/// the value trim it.
/// `secret`: Some(show) for a secret (hidden unless `show`; a Show / Hide button next to it).
pub(crate) fn value_field(
    ui: &mut Ui,
    label: &str,
    v: &mut Option<String>,
    hint: &str,
    secret: Option<&mut bool>,
    width: f32,
) -> bool {
    // Each secret field has its own Show / Hide (kept in egui's memory by its label), so
    // showing one key never reveals the others. `secret` only says the field is a secret.
    let show_id = ui.make_persistent_id(("ls_secret_shown", label));
    let is_secret = secret.is_some();
    let mut shown = ui.data(|d| d.get_temp::<bool>(show_id)).unwrap_or(false);
    let password = is_secret && !shown;
    let mut ch = false;
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(vec2(130.0, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(130.0);
            ui.label(RichText::new(label).size(13.0));
        });
        let mut s = v.clone().unwrap_or_default();
        let r = ui.add(
            egui::TextEdit::singleline(&mut s)
                .password(password)
                .hint_text(hint)
                .desired_width(width)
                .font(egui::TextStyle::Monospace),
        );
        if r.changed() {
            *v = (!s.is_empty()).then_some(s);
            ch = true;
        }
        if is_secret && ui.small_button(if shown { "Hide" } else { "Show" }).clicked() {
            shown = !shown;
            ui.data_mut(|d| d.insert_temp(show_id, shown));
        }
    });
    ch
}

/// A collapsible section header: "<name>  ·  <status>".
fn keyed_section(ui: &mut Ui, k: SourceKind, status: (&str, egui::Color32), body: impl FnOnce(&mut Ui)) {
    // Scrolled to every frame until the section has finished opening (its height grows while
    // it animates open).
    let scroll = ui.ctx().data(|d| d.get_temp::<String>(scroll_id())).is_some_and(|n| n == k.name());
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), section_id(k), false);
    if scroll && state.openness(ui.ctx()) >= 1.0 {
        take_scroll(ui, k);
    }
    let (_, header, _) = state
        .show_header(ui, |ui| {
            ui.label(RichText::new(k.name()).size(14.0).strong());
            ui.label(RichText::new(status.0).size(12.0).color(status.1));
        })
        .body(|ui| {
            ui.add_space(2.0);
            body(ui);
            ui.add_space(6.0);
        });
    if scroll {
        header.response.scroll_to_me(Some(egui::Align::TOP));
    }
}

/// IGDB and TheGamesDB credentials. `show`: secrets in clear text.
pub(super) fn keyed_sources(ui: &mut Ui, st: &mut AppState, show: &mut bool, pal: &Pal) -> bool {
    let mut ch = false;
    let infos = st.art.sources();
    let status = |k: SourceKind, cfg: &MetadataConfig| -> (&'static str, egui::Color32) {
        let rejected = infos
            .iter()
            .find(|s| s.kind == k)
            .and_then(|s| s.reason.as_deref())
            .is_some_and(|r| r.contains("rejected"));
        if rejected {
            ("key rejected", pal.danger)
        } else if k.configured(cfg) {
            ("ready", pal.accent)
        } else {
            ("not set up", pal.dim)
        }
    };
    let w = (ui.available_width() - 150.0).clamp(160.0, 340.0);
    // A clicked help link, opened once the config borrow below has ended.
    let mut open: Option<&'static str> = None;
    let m = &mut st.config.metadata;

    let s = status(SourceKind::Igdb, m);
    keyed_section(ui, SourceKind::Igdb, s, |ui| {
        hint_text(
            ui,
            "1. On dev.twitch.tv/console, choose Register Your Application.\n\
             2. Name: anything (e.g. Lodestar) · OAuth Redirect URL: http://localhost (click Add) · \
             Category: Application Integration · Client Type: Confidential. Then Create.\n\
             3. Open the new application (Manage), copy its Client ID, click New Secret and copy \
             the secret, and paste both here.\n\
             Covers, artwork and descriptions then come from IGDB.",
            pal,
        );
        if super::dialogs::link_text(ui, "Open dev.twitch.tv/console", "https://dev.twitch.tv/console", pal) {
            open = Some("https://dev.twitch.tv/console");
        }
        ch |= value_field(ui, "Client ID", &mut m.igdb_client_id, "Client ID", None, w);
        ch |= value_field(ui, "Client secret", &mut m.igdb_client_secret, "Client secret", Some(&mut *show), w);
    });

    let s = status(SourceKind::TheGamesDb, m);
    keyed_section(ui, SourceKind::TheGamesDb, s, |ui| {
        hint_text(ui, "Ask for a personal API key on forums.thegamesdb.net (API Keys board) and paste it here.", pal);
        if super::dialogs::link_text(ui, "Open forums.thegamesdb.net", "https://forums.thegamesdb.net/", pal) {
            open = Some("https://forums.thegamesdb.net/");
        }
        ch |= value_field(ui, "API key", &mut m.thegamesdb_key, "API key", Some(&mut *show), w);
    });
    if let Some(url) = open {
        st.open_url(url);
    }
    ch
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::art::sources::ordered_sources;

    #[test]
    fn reorder_and_switch_off() {
        let mut c = MetadataConfig::default();
        let pc = ArtList::Pc;
        let names = |c: &MetadataConfig| -> Vec<&'static str> {
            ordered_sources(c).into_iter().map(|(k, _)| k.name()).collect()
        };
        let default = names(&c);
        assert_eq!(default[0], "Steam");
        move_source(&mut c, pc, SourceKind::Libretro, true);
        let moved = names(&c);
        let i = moved.iter().position(|n| *n == "libretro").unwrap();
        assert_eq!(moved[i + 1], "TheGamesDB");
        move_source(&mut c, pc, SourceKind::Steam, true); // already first
        assert_eq!(names(&c)[0], "Steam");
        set_source_enabled(&mut c, pc, SourceKind::Lutris, false);
        set_source_enabled(&mut c, pc, SourceKind::Lutris, false);
        assert_eq!(c.sources_off, vec!["Lutris".to_string()]);
        assert!(ordered_sources(&c).contains(&(SourceKind::Lutris, false)));
        set_source_enabled(&mut c, pc, SourceKind::Lutris, true);
        assert!(c.sources_off.is_empty());
        // The emulated games' list is untouched by all that.
        assert!(c.emu_source_order.is_empty() && c.emu_sources_off.is_none());
    }

    #[test]
    fn emulated_list_has_its_own_order_and_switches() {
        let mut c = MetadataConfig::default();
        let emu = ArtList::Emulated;
        let rows = ordered_sources_for(&c, emu);
        let names: Vec<&str> = rows.iter().map(|(k, _)| k.name()).collect();
        assert_eq!(
            names,
            ["libretro", "GameTDB", "SteamGridDB", "IGDB", "TheGamesDB", "Steam", "GOG", "PCGamingWiki", "Lutris"]
        );
        let on: Vec<&str> = rows.iter().filter(|(_, on)| *on).map(|(k, _)| k.name()).collect();
        assert_eq!(on, ["libretro", "GameTDB", "SteamGridDB", "IGDB", "TheGamesDB"]);
        // Switching Steam on keeps the other defaults off.
        set_source_enabled(&mut c, emu, SourceKind::Steam, true);
        let off = c.emu_sources_off.clone().unwrap();
        assert_eq!(off, ["GOG", "PCGamingWiki", "Lutris"]);
        move_source(&mut c, emu, SourceKind::Igdb, true);
        let names: Vec<&str> = ordered_sources_for(&c, emu).iter().map(|(k, _)| k.name()).collect();
        assert_eq!(&names[..4], ["libretro", "GameTDB", "IGDB", "SteamGridDB"]);
        // PC list unchanged.
        assert!(c.source_order.is_empty() && c.sources_off.is_empty());
        // Saved and read back; a broken value falls back to the defaults, keeping the rest.
        let text =
            toml::to_string_pretty(&crate::config::Config { metadata: c.clone(), ..Default::default() }).unwrap();
        let back: crate::config::Config = toml::from_str(&text).unwrap();
        assert_eq!(back.metadata, c);
        let (lenient, problems) = crate::config::Config::parse_lenient(
            "[metadata]\nemu_sources_off = 7\nemu_source_order = [\"IGDB\", \"nope\"]\nsources_off = [\"Lutris\"]\n",
        );
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(lenient.metadata.emu_sources_off.is_none());
        assert_eq!(ordered_sources_for(&lenient.metadata, emu)[0].0, SourceKind::Igdb);
        assert_eq!(lenient.metadata.sources_off, ["Lutris"]);
    }
}
