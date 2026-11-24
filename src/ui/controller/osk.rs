//! On-screen keyboard: d-pad navigable key grid plus a real text field, so a physical
//! keyboard can type into it too (while it has focus, `input::keyboard_nav` only reports
//! Enter/Escape).
//!
//! Gamepad: A = press key, B = close, X = backspace, Y = space, Start = done,
//! LB = shift, RB = symbols.

use super::look::Look;
use crate::input::Nav;
use crate::model::GameId;
use crate::theme::render::draw::{cr, fade, layout_line};
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, Rect, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Search,
    LaunchOptions(GameId),
    Command(GameId),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum KeyAct {
    Char(char),
    Shift,
    Symbols,
    Space,
    Backspace,
    Clear,
    Done,
}

struct Key {
    label: String,
    act: KeyAct,
    w: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OskResult {
    None,
    /// Text changed (live search).
    Changed,
    /// Accept the text.
    Done,
    /// Close without accepting (callers may keep the text for search).
    Cancel,
}

pub struct Osk {
    pub purpose: Purpose,
    pub text: String,
    pub title: String,
    row: usize,
    col: usize,
    shift: bool,
    symbols: bool,
    pub opened: f64,
    /// Text the `preview` line was computed for, and the line.
    pub preview: Option<(String, Result<String, String>)>,
    closing: bool,
    /// The gamepad changed the text: put the field's cursor at the end.
    cursor_to_end: bool,
}

const LETTERS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl'", "zxcvbnm,.-"];
const SYMBOLS: [&str; 4] = ["1234567890", "%=-_+/\\:;\"", "$~{}()[]<>", "!?@#&*|',."];

impl Osk {
    pub fn new(purpose: Purpose, title: impl Into<String>, text: String, now: f64) -> Osk {
        Osk {
            purpose,
            text,
            title: title.into(),
            row: 1,
            col: 0,
            shift: false,
            symbols: false,
            opened: now,
            preview: None,
            closing: false,
            cursor_to_end: true,
        }
    }

    fn rows(&self) -> Vec<Vec<Key>> {
        let src = if self.symbols { SYMBOLS } else { LETTERS };
        let mut rows: Vec<Vec<Key>> = src
            .iter()
            .map(|r| {
                r.chars()
                    .map(|c| {
                        let c = if self.shift && !self.symbols { c.to_ascii_uppercase() } else { c };
                        Key { label: c.to_string(), act: KeyAct::Char(c), w: 1.0 }
                    })
                    .collect()
            })
            .collect();
        rows.push(vec![
            Key { label: if self.shift { "SHIFT".into() } else { "Shift".into() }, act: KeyAct::Shift, w: 1.5 },
            Key { label: if self.symbols { "ABC".into() } else { "#+=".into() }, act: KeyAct::Symbols, w: 1.2 },
            Key { label: "Space".into(), act: KeyAct::Space, w: 3.3 },
            Key { label: "Delete".into(), act: KeyAct::Backspace, w: 1.2 },
            Key { label: "Clear".into(), act: KeyAct::Clear, w: 1.2 },
            Key { label: "Done".into(), act: KeyAct::Done, w: 1.6 },
        ]);
        rows
    }

    /// Centre (in key units) of key `col` in `row`.
    fn centre(rows: &[Vec<Key>], row: usize, col: usize) -> f32 {
        let mut x = 0.0;
        for (i, k) in rows[row].iter().enumerate() {
            if i == col {
                return x + k.w / 2.0;
            }
            x += k.w;
        }
        x
    }

    fn nearest(rows: &[Vec<Key>], row: usize, x: f32) -> usize {
        let mut best = (f32::MAX, 0);
        for i in 0..rows[row].len() {
            let d = (Self::centre(rows, row, i) - x).abs();
            if d < best.0 {
                best = (d, i);
            }
        }
        best.1
    }

    fn press(&mut self, act: KeyAct) -> OskResult {
        self.cursor_to_end = true;
        match act {
            KeyAct::Char(c) => {
                self.text.push(c);
                if self.shift && !self.symbols && self.purpose == Purpose::Search {
                    self.shift = false; // one-shot shift for search; sticky for launch options
                }
                OskResult::Changed
            }
            KeyAct::Space => {
                self.text.push(' ');
                OskResult::Changed
            }
            KeyAct::Backspace => {
                self.text.pop();
                OskResult::Changed
            }
            KeyAct::Clear => {
                self.text.clear();
                OskResult::Changed
            }
            KeyAct::Shift => {
                self.shift = !self.shift;
                OskResult::None
            }
            KeyAct::Symbols => {
                self.symbols = !self.symbols;
                OskResult::None
            }
            KeyAct::Done => OskResult::Done,
        }
    }

    /// `kb_enter` / `kb_escape`: the physical keys were pressed this frame (they also arrive
    /// as Accept/Back navs, which must not press the highlighted key).
    pub fn nav(&mut self, nav: Nav, kb_enter: bool, kb_escape: bool) -> OskResult {
        let rows = self.rows();
        self.row = self.row.min(rows.len() - 1);
        self.col = self.col.min(rows[self.row].len() - 1);
        match nav {
            Nav::Left => {
                let n = rows[self.row].len();
                self.col = (self.col + n - 1) % n;
            }
            Nav::Right => {
                let n = rows[self.row].len();
                self.col = (self.col + 1) % n;
            }
            Nav::Up | Nav::Down => {
                let x = Self::centre(&rows, self.row, self.col);
                let n = rows.len();
                self.row = if nav == Nav::Up { (self.row + n - 1) % n } else { (self.row + 1) % n };
                self.col = Self::nearest(&rows, self.row, x);
            }
            Nav::Accept if kb_enter => return OskResult::Done,
            Nav::Accept => return self.press(rows[self.row][self.col].act),
            Nav::Back if kb_escape => return OskResult::Cancel,
            Nav::Back => return OskResult::Cancel,
            Nav::Favorite => return self.press(KeyAct::Backspace),
            Nav::Search => return self.press(KeyAct::Space),
            Nav::Menu => return OskResult::Done,
            Nav::PageUp | Nav::Select => return self.press(KeyAct::Shift),
            Nav::PageDown => return self.press(KeyAct::Symbols),
            _ => {}
        }
        OskResult::None
    }

    pub fn field_id() -> egui::Id {
        egui::Id::NULL.with("lodestar-controller-osk-field")
    }

    /// Release keyboard focus (call when closing).
    pub fn release_focus(ctx: &egui::Context) {
        ctx.memory_mut(|m| m.surrender_focus(Self::field_id()));
    }

    /// Draw as a bottom sheet (Search: at the top, so the results stay visible), with the
    /// launch preview under the field when there is one.
    pub fn draw(&mut self, ui: &mut egui::Ui, screen: Rect, look: &Look, now: f64) -> (OskResult, bool) {
        let mut result = OskResult::None;
        let t = (((now - self.opened) / 0.18) as f32).clamp(0.0, 1.0);
        let e = super::menu::ease(t);
        let animating = t < 1.0;
        let painter = ui.painter().clone();
        painter.rect_filled(screen, 0.0, Color32::from_black_alpha((120.0 * e) as u8));
        let rows = self.rows();
        let key_h = look.h(0.056);
        let gap = look.h(0.01);
        let has_preview = self.preview.is_some();
        let sheet_h = look.h(0.19) + (key_h + gap) * rows.len() as f32 + if has_preview { look.h(0.06) } else { 0.0 };
        let width = (look.sw * 0.62).clamp(look.h(0.95).min(look.sw), look.sw);
        let off = (1.0 - e) * sheet_h * 0.4;
        let sheet = if self.purpose == Purpose::Search {
            Rect::from_min_size(
                pos2(screen.center().x - width / 2.0, screen.top() - look.h(0.02) - off),
                vec2(width, sheet_h + look.h(0.02)),
            )
        } else {
            Rect::from_min_size(
                pos2(screen.center().x - width / 2.0, screen.bottom() - sheet_h + off),
                vec2(width, sheet_h + look.h(0.02)),
            )
        };
        let mut p = painter.with_clip_rect(screen);
        p.multiply_opacity(e);
        crate::theme::render::draw::drop_shadow(&p, sheet, look.h(0.02), 0.8);
        p.rect_filled(sheet, cr(look.h(0.02)), look.surface);
        let pad = look.h(0.03);
        let inner = if self.purpose == Purpose::Search {
            Rect::from_min_max(sheet.min + vec2(pad, look.h(0.02) + pad * 0.8), sheet.max - vec2(pad, pad * 0.8))
        } else {
            sheet.shrink2(vec2(pad, pad * 0.8))
        };
        let mut y = inner.top();
        let tg = layout_line(&p, &self.title, look.bold(0.024), look.dim, inner.width());
        p.galley(pos2(inner.left(), y), tg.clone(), look.dim);
        y += tg.size().y + look.h(0.012);
        let field = Rect::from_min_size(pos2(inner.left(), y), vec2(inner.width(), look.h(0.065)));
        if self.text_field(ui, &p, look, field) {
            result = OskResult::Changed;
        }
        y = field.bottom() + look.h(0.012);
        y += self.paint_preview(&p, look, pos2(inner.left(), y), inner.width());
        let keys = Rect::from_min_max(pos2(inner.left(), y), inner.max);
        let (res, keys_bottom) = self.keys(ui, &p, look, keys, &rows);
        if res != OskResult::None {
            result = res;
        }
        paint_legend(
            &p,
            look,
            Rect::from_min_size(pos2(inner.left(), keys_bottom), vec2(inner.width(), look.h(0.035))),
        );
        (result, animating)
    }

    /// The text field (a real TextEdit, so physical keyboards type into it). True when the text
    /// changed.
    fn text_field(&mut self, ui: &mut egui::Ui, p: &egui::Painter, look: &Look, field: Rect) -> bool {
        p.rect_filled(field, cr(look.h(0.012)), look.surface_alt);
        p.rect_stroke(
            field,
            cr(look.h(0.012)),
            egui::Stroke::new(look.h(0.0025), fade(look.accent, 0.8)),
            egui::StrokeKind::Inside,
        );
        let edit_rect = field.shrink2(vec2(look.h(0.02), look.h(0.012)));
        let before = self.text.clone();
        if !self.closing {
            let hint = match self.purpose {
                Purpose::Search => "Type to search",
                Purpose::LaunchOptions(_) => "e.g. MANGOHUD=1 gamemoderun %command% -novid",
                Purpose::Command(_) => "Program and arguments, e.g. /usr/bin/game --flag",
            };
            if std::mem::take(&mut self.cursor_to_end) {
                let id = Self::field_id();
                let mut state = egui::widgets::text_edit::TextEditState::load(ui.ctx(), id).unwrap_or_default();
                let end = egui::text::CCursor::new(self.text.chars().count());
                state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                state.store(ui.ctx(), id);
            }
            let edit = egui::TextEdit::singleline(&mut self.text)
                .id(Self::field_id())
                .font(look.medium(0.03))
                .text_color(look.text)
                .hint_text(egui::RichText::new(hint).color(fade(look.dim, 0.7)).font(look.font(0.026)))
                .frame(egui::Frame::NONE)
                .desired_width(edit_rect.width());
            let resp = ui.put(edit_rect, edit);
            if !resp.has_focus() {
                resp.request_focus();
            }
        }
        self.text != before
    }

    /// "Will run: …" / "Can't run: …" under the field. Returns the height it takes.
    fn paint_preview(&self, p: &egui::Painter, look: &Look, at: egui::Pos2, width: f32) -> f32 {
        let Some((_, line)) = &self.preview else {
            return look.h(0.008);
        };
        let (txt, col) = match line {
            Ok(s) => (format!("Will run: {s}"), look.dim),
            Err(e) => (format!("Can't run: {e}"), look.danger),
        };
        let mut job =
            LayoutJob::single_section(txt, TextFormat { font_id: look.font(0.018), color: col, ..Default::default() });
        job.wrap =
            TextWrapping { max_width: width, max_rows: 2, break_anywhere: true, overflow_character: Some('…') };
        let g = p.layout_job(job);
        p.galley(at, g, col);
        look.h(0.06)
    }

    /// The key grid from the top of `area` (a click presses a key). Returns what a pressed key
    /// did and where the grid ends.
    fn keys(
        &mut self,
        ui: &egui::Ui,
        p: &egui::Painter,
        look: &Look,
        area: Rect,
        rows: &[Vec<Key>],
    ) -> (OskResult, f32) {
        let mut result = OskResult::None;
        let key_h = look.h(0.056);
        let gap = look.h(0.01);
        let units = 10.0;
        let unit_w = (area.width() - gap * (units - 1.0)) / units;
        let pointer = ui.input(|i| (i.pointer.hover_pos(), i.pointer.primary_clicked()));
        let mut y = area.top();
        for (ri, row) in rows.iter().enumerate() {
            let row_units: f32 = row.iter().map(|k| k.w).sum();
            let row_w = row_units * unit_w + gap * (row.len() as f32 - 1.0) + (row_units - row.len() as f32) * gap;
            let mut x = area.center().x - row_w / 2.0;
            for (ci, k) in row.iter().enumerate() {
                let w = k.w * unit_w + (k.w - 1.0) * gap;
                let r = Rect::from_min_size(pos2(x, y), vec2(w, key_h));
                x += w + gap;
                let selected = ri == self.row && ci == self.col;
                let active = (k.act == KeyAct::Shift && self.shift) || (k.act == KeyAct::Symbols && self.symbols);
                let (fill, fg) = if selected {
                    (look.accent, look.accent_text)
                } else if active || k.act == KeyAct::Done {
                    (fade(look.accent, 0.35), look.text)
                } else {
                    (look.surface_alt, look.text)
                };
                let rr = if selected { r.expand(look.h(0.003)) } else { r };
                p.rect_filled(rr, cr(look.h(0.01)), fill);
                let font = if k.label.chars().count() == 1 { look.medium(0.028) } else { look.medium(0.021) };
                let g = p.layout_no_wrap(k.label.clone(), font, fg);
                p.galley(rr.center() - g.size() / 2.0, g, fg);
                if let (Some(pos), true) = pointer
                    && r.contains(pos)
                    && !self.closing
                {
                    self.row = ri;
                    self.col = ci;
                    let res = self.press(k.act);
                    if res != OskResult::None {
                        result = res;
                    }
                }
            }
            y += key_h + gap;
        }
        (result, y)
    }

    /// Mark closed (the text field stops grabbing focus).
    pub fn close(&mut self, ctx: &egui::Context) {
        self.closing = true;
        Self::release_focus(ctx);
    }
}

/// The gamepad legend under the keys.
fn paint_legend(p: &egui::Painter, look: &Look, rect: Rect) {
    use crate::theme::render::draw::{Button, Hint, paint_hints};
    let legend = [
        Hint::new(Button::A, "Type"),
        Hint::new(Button::B, "Close"),
        Hint::new(Button::X, "Delete"),
        Hint::new(Button::Y, "Space"),
        Hint::new(Button::LB, "Shift"),
        Hint::new(Button::RB, "Symbols"),
        Hint::new(Button::Start, "Done"),
    ];
    paint_hints(
        p,
        rect,
        &legend,
        look.prompts,
        look.medium.clone(),
        look.h(0.0165),
        look.dim,
        fade(look.text, 0.85),
        look.surface,
        crate::theme::Align::Center,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_and_navigation() {
        let mut k = Osk::new(Purpose::Search, "Search", String::new(), 0.0);
        // Starts on 'q'.
        assert_eq!(k.nav(Nav::Accept, false, false), OskResult::Changed);
        assert_eq!(k.text, "q");
        k.nav(Nav::Right, false, false);
        k.nav(Nav::Accept, false, false);
        assert_eq!(k.text, "qw");
        k.nav(Nav::Favorite, false, false);
        assert_eq!(k.text, "q");
        k.nav(Nav::Search, false, false);
        assert_eq!(k.text, "q ");
        // Wrap left from the first column.
        k.nav(Nav::Left, false, false);
        k.nav(Nav::Left, false, false);
        k.nav(Nav::Accept, false, false);
        assert_eq!(k.text, "q p");
        // Down to the bottom row lands on a wide key, up again returns near the same column.
        for _ in 0..3 {
            k.nav(Nav::Down, false, false);
        }
        assert_eq!(k.row, 4);
        k.nav(Nav::Up, false, false);
        assert_eq!(k.row, 3);
        // Shift (LB) types capitals; keyboard Enter means done, not "press key".
        k.nav(Nav::PageUp, false, false);
        k.nav(Nav::Accept, false, false);
        assert!(k.text.ends_with(|c: char| c.is_ascii_uppercase() || !c.is_alphabetic()));
        assert_eq!(k.nav(Nav::Accept, true, false), OskResult::Done);
        assert_eq!(k.nav(Nav::Back, false, false), OskResult::Cancel);
        assert_eq!(k.nav(Nav::Menu, false, false), OskResult::Done);
    }

    #[test]
    fn symbols_have_launch_option_characters() {
        let all: String = SYMBOLS.concat();
        for c in ['%', '=', '-', '_', '/', ':', '"', '$', '~'] {
            assert!(all.contains(c) || LETTERS.concat().contains(c), "{c}");
        }
    }
}
