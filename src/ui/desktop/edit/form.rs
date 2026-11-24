//! The Edit Game Details form: fields that follow the source until changed, date / duration /
//! score parsing, and the form widgets the tabs are built from.

use super::*;

/// A text field whose value is either the source's or the user's own.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Field {
    pub text: String,
    /// The source's value when known ("" = the source has none). None: hidden behind an older
    /// edit (it shows again after a reset is saved).
    pub source: Option<String>,
    /// The user's own value (saved as an edit).
    pub over: bool,
    /// Download metadata… just gave it a new value (highlighted until the user changes it or
    /// the dialog is saved / cancelled).
    pub updated: bool,
}

impl Field {
    /// `edit`: the user's value, if any; `source`: see [`Field::source`].
    pub fn new(edit: Option<String>, source: Option<String>) -> Field {
        match edit {
            Some(e) => Field { text: e, source, over: true, updated: false },
            None => Field { text: source.clone().unwrap_or_default(), source, over: false, updated: false },
        }
    }

    /// A field only the user sets (no source value).
    pub fn own(edit: Option<String>) -> Field {
        Field::new(edit, Some(String::new()))
    }

    /// Call after the user typed: overridden unless it is back to the source's value.
    pub fn touched(&mut self) {
        self.updated = false;
        self.over = match &self.source {
            Some(s) => self.text.trim() != s.trim(),
            None => true,
        };
    }

    pub fn reset(&mut self) {
        self.text = self.source.clone().unwrap_or_default();
        self.over = false;
        self.updated = false;
    }

    /// What to save: the trimmed text when overridden. An overridden empty field is saved as
    /// `Some("")` = "the user cleared it" (saving `None` would bring the source's text back);
    /// `Game::with_edits_applied` shows it as empty.
    pub fn value(&self) -> Option<String> {
        self.over.then(|| self.text.trim().to_string())
    }

    /// Follow a new source value (after Download metadata / a rescan) unless overridden.
    pub(super) fn follow(&mut self, fresh: &Field) {
        if !self.over && fresh.source.is_some() {
            let updated = self.updated && self.text == fresh.text;
            *self = fresh.clone();
            self.updated = updated;
        }
    }

    /// Take the value a lookup the user asked for found (`fresh`: this field of the game with
    /// the answer stored). The user's own typed value stays; a field they cleared is filled
    /// again when the lookup has a value for it (they asked for data); a field following the
    /// source takes the new value, empty included (nothing left from an earlier match).
    /// True when the text changed (it is then marked `updated`).
    pub(super) fn take_lookup(&mut self, fresh: &Field) -> bool {
        let Some(src) = fresh.source.clone() else { return false };
        if self.over && (!self.text.trim().is_empty() || src.trim().is_empty()) {
            return false;
        }
        let changed = self.text.trim() != src.trim();
        *self = Field { text: src.clone(), source: Some(src), over: false, updated: changed || self.updated };
        changed
    }
}

/// The subtle "just filled in by Download metadata…" frame around a field's box.
pub(super) fn mark_updated(ui: &Ui, r: &egui::Response, f: &Field, pal: &Pal) {
    if !f.updated {
        return;
    }
    ui.painter().rect_stroke(
        r.rect.expand(1.0),
        5.0,
        egui::Stroke::new(1.5, with_alpha(pal.accent, 190)),
        egui::StrokeKind::Outside,
    );
}

/// "A, B ,, C" → ["A", "B", "C"].
pub(crate) fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

/// "2024-03-05" from unix seconds (UTC).
pub(crate) fn format_date(ts: i64) -> String {
    let (y, m, d) = civil_from_days(ts.div_euclid(86400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 for a civil date (inverse of `civil_from_days`).
pub(super) fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "YYYY-MM-DD" → unix seconds at midnight UTC; None when it isn't a real date.
pub(crate) fn parse_date(s: &str) -> Option<i64> {
    let mut it = s.trim().split('-');
    let y: i64 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let days = days_from_civil(y, m, d);
    // Reject 2023-02-30 and friends (they would round-trip to another date).
    (civil_from_days(days) == (y, m, d)).then_some(days * 86400)
}

/// "12h 30m" for the time-played field.
pub(crate) fn format_duration(secs: u64) -> String {
    let (h, m) = (secs / 3600, (secs % 3600) / 60);
    match (h, m) {
        (0, m) => format!("{m}m"),
        (h, 0) => format!("{h}h"),
        (h, m) => format!("{h}h {m}m"),
    }
}

/// "12h 30m", "12h", "45m", "12:30", "1.5" (hours), "" (0) → seconds.
pub(crate) fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim().to_lowercase();
    if s.is_empty() {
        return Some(0);
    }
    if let Some((h, m)) = s.split_once(':') {
        let h: u64 = h.trim().parse().ok()?;
        let m: u64 = m.trim().parse().ok()?;
        return (m < 60).then_some(h * 3600 + m * 60);
    }
    if let Ok(h) = s.parse::<f64>() {
        return (h >= 0.0 && h.is_finite()).then_some((h * 3600.0).round() as u64);
    }
    let mut total = 0u64;
    let mut num = String::new();
    let mut any = false;
    for c in s.chars() {
        match c {
            '0'..='9' | '.' => num.push(c),
            'h' | 'm' | 's' => {
                let v: f64 = num.parse().ok()?;
                num.clear();
                any = true;
                total += (v * match c {
                    'h' => 3600.0,
                    'm' => 60.0,
                    _ => 1.0,
                })
                .round() as u64;
            }
            ' ' => {}
            _ => return None,
        }
    }
    (any && num.is_empty()).then_some(total)
}

/// Score text "0".."100" (empty = not set); Err for anything else.
pub(super) fn parse_score(s: &str) -> Result<Option<u8>, ()> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    match s.parse::<u8>() {
        Ok(v) if v <= 100 => Ok(Some(v)),
        _ => Err(()),
    }
}

/// A yes/no setting that may follow the source (like [`Field`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BoolField {
    pub(super) val: bool,
    pub(super) source: Option<bool>,
    pub(super) over: bool,
}

impl BoolField {
    pub(super) fn touched(&mut self) {
        self.over = self.source != Some(self.val);
    }
}

// ------------------------------------------------------------------------------- widgets

/// Underlined tab (like the artwork editor's slot tabs).
pub(super) fn tab_button(ui: &mut Ui, label: &str, active: bool, pal: &Pal) -> egui::Response {
    let font = egui::FontId::proportional(13.5);
    let gal = ui.painter().layout_no_wrap(label.to_owned(), font, pal.text);
    let (rect, resp) = ui.allocate_exact_size(vec2(gal.size().x + 22.0, 30.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if resp.hovered() && !active {
            p.rect_filled(rect.shrink2(vec2(0.0, 2.0)), 5.0, pal.hover);
        }
        let col = if active {
            pal.text
        } else if resp.hovered() {
            mix(pal.text, pal.dim, 0.3)
        } else {
            pal.dim
        };
        p.galley(pos2(rect.left() + 11.0, rect.center().y - gal.size().y / 2.0), gal, col);
        if active {
            p.rect_filled(
                egui::Rect::from_min_max(
                    pos2(rect.left() + 6.0, rect.bottom() - 2.5),
                    pos2(rect.right() - 6.0, rect.bottom()),
                ),
                1.5,
                pal.accent,
            );
        }
    }
    resp
}

pub(super) const LABEL_W: f32 = 112.0;

pub(super) const RESET_W: f32 = 22.0;

/// A form row: fixed-width label, then `add` (given the remaining width).
pub(super) fn form_row<R>(ui: &mut Ui, label: &str, pal: &Pal, add: impl FnOnce(&mut Ui, f32) -> R) -> R {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(LABEL_W, 26.0), Sense::hover());
        ui.painter().text(
            pos2(r.left(), r.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::proportional(13.0),
            pal.dim,
        );
        let w = ui.available_width();
        add(ui, w)
    })
    .inner
}

pub(super) fn te<'a>(s: &'a mut String, hint: &str, w: f32) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(s).hint_text(hint).desired_width(w - 4.0).margin(egui::Margin::symmetric(7, 4))
}

/// The small "reset to the source's value" arrow next to an overridden field.
pub(super) fn reset_button(ui: &mut Ui, f: &Field) -> bool {
    if !f.over {
        ui.add_space(RESET_W);
        return false;
    }
    let tip = match f.source.as_deref() {
        Some("") => "Clear (the source has no value)".to_string(),
        Some(s) => format!("Reset to the source's value: {}", truncate_chars(s, 80)),
        None => "Reset to the source's value (shown after saving)".to_string(),
    };
    icon_button(ui, Icon::Reset, false, RESET_W, &tip).clicked()
}

pub(super) fn truncate_chars(s: &str, n: usize) -> String {
    let s = s.trim();
    match s.char_indices().nth(n) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// A [`Field`] row: text box + reset arrow while it differs from the source.
pub(super) fn field_row(ui: &mut Ui, label: &str, f: &mut Field, hint: &str, pal: &Pal) {
    form_row(ui, label, pal, |ui, w| {
        let hint = match (&f.source, f.over) {
            (None, true) => "(the source's value after saving)",
            _ => hint,
        };
        let r = ui.add(te(&mut f.text, hint, w - RESET_W - ui.spacing().item_spacing.x));
        if r.changed() {
            f.touched();
        }
        mark_updated(ui, &r, f, pal);
        if reset_button(ui, f) {
            f.reset();
        }
    });
}

/// A short numeric field (score, year).
pub(super) fn small_field(ui: &mut Ui, f: &mut Field, hint: &str, w: f32, pal: &Pal) -> egui::Response {
    let bad = !f.text.trim().is_empty() && f.text.trim().parse::<i64>().is_err();
    let r = ui.add(te(&mut f.text, hint, w).text_color_opt(bad.then_some(pal.danger)));
    if r.changed() {
        f.text.retain(|c| c.is_ascii_digit());
        f.touched();
    }
    mark_updated(ui, &r, f, pal);
    r
}

pub(super) fn section_heading(ui: &mut Ui, text: &str, pal: &Pal) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).size(14.5).strong().color(pal.text));
}

pub(super) fn date_row(ui: &mut Ui, label: &str, v: &mut String, hint: &str, pal: &Pal) {
    form_row(ui, label, pal, |ui, _| {
        let bad = !v.trim().is_empty() && parse_date(v).is_none();
        ui.add(te(v, hint, 120.0).text_color_opt(bad.then_some(pal.danger)));
        if !v.trim().is_empty()
            && let Some(t) = parse_date(v)
        {
            ui.label(RichText::new(format_last_played(Some(t))).size(12.0).color(pal.dim));
        }
    });
}
