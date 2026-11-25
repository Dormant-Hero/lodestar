//! The first start's welcome step in controller mode (see welcome_app.rs): a card over the
//! library with the same choices as the desktop's, picked with the d-pad and A. The key goes in
//! through the on-screen keyboard, whose text field also takes a paste (Ctrl+V) from a browser
//! on this PC; with neither at hand, the step says where to add the key later.

use super::look::Look;
use super::osk::{Osk, Purpose};
use crate::app::{AppState, Mode};
use crate::art::KeyCheck;
use crate::input::Nav;
use crate::theme::render::draw::{self as rd, Button, Hint, fade};
use crate::welcome_app::{SGDB_KEY_PAGE, key_check_text, key_later};
use egui::{Rect, pos2, vec2};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Row {
    #[default]
    OpenPage,
    EnterKey,
    Continue,
    Skip,
}

#[derive(Default)]
pub(super) struct Card {
    sel: Row,
}

/// The rows shown: Continue only once there is a key SteamGridDB didn't refuse.
fn rows(st: &AppState) -> Vec<Row> {
    let key = st.welcome.key.trim();
    let usable = !key.is_empty() && !matches!(last_check(st), Some(KeyCheck::Rejected));
    let mut rows = vec![Row::OpenPage, Row::EnterKey];
    if usable {
        rows.push(Row::Continue);
    }
    rows.push(Row::Skip);
    rows
}

/// The check of the key entered (the desktop's field shares it).
fn last_check(st: &AppState) -> Option<KeyCheck> {
    st.art_state.key_check.answer_for(&st.welcome.key)
}

impl Card {
    /// One press while the step shows; returns a keyboard to open for the key.
    pub fn nav(&mut self, st: &mut AppState, nav: Nav, now: f64) -> Option<Osk> {
        let rows = rows(st);
        let at = rows.iter().position(|r| *r == self.sel).unwrap_or(0);
        match nav {
            Nav::Up => self.sel = rows[at.saturating_sub(1)],
            Nav::Down => self.sel = rows[(at + 1).min(rows.len() - 1)],
            Nav::Accept => return self.choose(st, rows[at], now),
            Nav::Back => st.finish_welcome(false),
            _ => {}
        }
        None
    }

    fn choose(&mut self, st: &mut AppState, row: Row, now: f64) -> Option<Osk> {
        self.sel = row;
        match row {
            Row::OpenPage => st.open_url(SGDB_KEY_PAGE),
            Row::EnterKey => {
                return Some(Osk::new(Purpose::ApiKey, "SteamGridDB API key", st.welcome.key.clone(), now));
            }
            Row::Continue => st.finish_welcome(true),
            Row::Skip => st.finish_welcome(false),
        }
        None
    }

    /// The keyboard closed with `key` (Done).
    pub fn key_entered(&mut self, st: &mut AppState, key: &str) {
        st.welcome.key = key.trim().to_string();
        if !st.welcome.key.is_empty() {
            self.sel = Row::Continue;
        }
    }

    /// Draw the card over the library. `pointer`: where the pointer is and whether it clicked
    /// (None while something covers the card); a click picks a row like A does.
    pub fn draw(
        &mut self,
        ui: &egui::Ui,
        st: &mut AppState,
        screen: Rect,
        look: &Look,
        pointer: Option<(egui::Pos2, bool)>,
        now: f64,
    ) -> Option<Osk> {
        // Keeps the key's answer coming while the card shows.
        let key = st.welcome.key.clone();
        let check = st.sgdb_key_check(&key);
        let rows = rows(st);
        if !rows.contains(&self.sel) {
            self.sel = Row::EnterKey;
        }
        let p = ui.painter();
        p.rect_filled(screen, 0.0, egui::Color32::from_black_alpha(190));
        let w = (look.sw * 0.5).clamp(look.h(0.8), look.sw * 0.92);
        let pad = look.h(0.04);
        let inner_w = w - 2.0 * pad;
        let title = rd::layout_line(p, "Welcome to Lodestar", look.title(0.04), look.text, inner_w);
        let body = p.layout(
            "Steam and GOG games bring their own artwork. Covers, backgrounds and logos for emulated games, \
             shortcuts and launchers without art of their own come from SteamGridDB, a free community site. \
             Add your free SteamGridDB key now, and artwork shows up for all your games from the start."
                .into(),
            look.font(0.023),
            look.text,
            inner_w,
        );
        let later = p.layout(
            "No keyboard at hand? Skip for now and add the key later in desktop mode.".into(),
            look.font(0.018),
            look.dim,
            inner_w,
        );
        // The page's address under its row: a browser that opens out of sight (gamescope) can
        // still be pointed there by typing it, on a phone for example.
        let address = rd::layout_line(p, page_address(), look.font(0.018), look.dim, inner_w);
        let address_h = address.size().y + look.h(0.008);
        let row_h = look.h(0.075);
        let row_gap = look.h(0.012);
        let rows_h = rows.len() as f32 * (row_h + row_gap) + address_h;
        let hints_h = look.h(0.05);
        let h = pad
            + title.size().y
            + look.h(0.02)
            + body.size().y
            + look.h(0.03)
            + rows_h
            + look.h(0.01)
            + later.size().y
            + look.h(0.02)
            + hints_h
            + pad;
        let card = Rect::from_center_size(screen.center(), vec2(w, h));
        rd::drop_shadow(p, card, look.h(0.016), 0.9);
        p.rect_filled(card, rd::cr(look.h(0.016)), look.surface);
        let x = card.left() + pad;
        let mut y = card.top() + pad;
        p.galley(pos2(x, y), title.clone(), look.text);
        y += title.size().y + look.h(0.02);
        p.galley(pos2(x, y), body.clone(), look.text);
        y += body.size().y + look.h(0.03);

        let mut picked = None;
        for row in &rows {
            let r = Rect::from_min_size(pos2(x, y), vec2(inner_w, row_h));
            let hover = pointer.is_some_and(|(pos, _)| r.contains(pos));
            if let Some((pos, true)) = pointer
                && r.contains(pos)
            {
                picked = Some(*row);
            }
            let on = *row == self.sel;
            let (fill, fg) = if on { (look.accent, look.accent_text) } else { (look.surface_alt, look.text) };
            p.rect_filled(r, rd::cr(row_h * 0.2), if hover && !on { fade(fill, 0.85) } else { fill });
            let (label, sub, sub_color) = row_text(*row, &key, check.as_ref(), look);
            let sub_color = if on { fade(fg, 0.85) } else { sub_color };
            let lx = r.left() + row_h * 0.3;
            let tw = inner_w - row_h * 0.6;
            let lg = rd::layout_line(p, &label, look.bold(0.024), fg, tw);
            let sg = rd::layout_line(p, &sub, look.font(0.018), sub_color, tw);
            let top = r.center().y - (lg.size().y + sg.size().y) / 2.0;
            p.galley(pos2(lx, top), lg.clone(), fg);
            p.galley(pos2(lx, top + lg.size().y), sg, sub_color);
            y += row_h + row_gap;
            if *row == Row::OpenPage {
                p.galley(pos2(lx, y - row_gap * 0.5), address.clone(), look.dim);
                y += address_h;
            }
        }
        y += look.h(0.01);
        p.galley(pos2(x, y), later, look.dim);
        let hb =
            Rect::from_min_max(pos2(x, card.bottom() - pad - hints_h), pos2(card.right() - pad, card.bottom() - pad));
        let hints = [Hint::new(Button::A, "Select"), Hint::new(Button::B, "Skip for now")];
        look.paint_screen_hints(p, hb, &hints, look.text);
        picked.and_then(|row| self.choose(st, row, now))
    }
}

/// A row's label and the line under it (with its colour).
fn row_text(row: Row, key: &str, check: Option<&KeyCheck>, look: &Look) -> (String, String, egui::Color32) {
    let dim = look.dim;
    match row {
        Row::OpenPage => (
            "Open SteamGridDB's API page".into(),
            "In the browser: sign in (a Steam account works) and copy the key".into(),
            dim,
        ),
        Row::EnterKey => match check {
            None => ("Enter the key".into(), "Paste it with Ctrl+V, or type it".into(), dim),
            Some(c) => {
                let color = match c {
                    KeyCheck::Rejected => look.danger,
                    KeyCheck::Works => look.accent,
                    _ => dim,
                };
                (masked(key), key_check_text(c), color)
            }
        },
        Row::Continue => ("Continue".into(), "Save the key and look up artwork".into(), dim),
        Row::Skip => ("Skip for now".into(), format!("Add it later in {}", key_later(Mode::Controller)), dim),
    }
}

/// The API page's address without the scheme: short enough to type by hand.
fn page_address() -> &'static str {
    SGDB_KEY_PAGE.trim_start_matches("https://").trim_start_matches("www.")
}

/// The key as the card shows it: only its last four characters, like the hidden key fields of
/// the desktop's Settings.
fn masked(key: &str) -> String {
    let key = key.trim();
    let n = key.chars().count();
    if n <= 4 {
        return "Key: ••••".into();
    }
    let last: String = key.chars().skip(n - 4).collect();
    format!("Key ending in {last}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_key_shows_only_its_last_four_characters() {
        assert_eq!(super::masked(" 0123456789abcdef "), "Key ending in cdef");
        assert_eq!(super::masked("abc"), "Key: ••••");
    }

    #[test]
    fn the_address_can_be_typed() {
        assert_eq!(super::page_address(), "steamgriddb.com/profile/preferences/api");
    }
}
