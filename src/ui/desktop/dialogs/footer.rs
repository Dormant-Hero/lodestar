//! The row at the bottom of every dialog. One layout everywhere: secondary actions and notes
//! from the left edge, the dialog's own answer at the right as `[Cancel] [Primary]`, the primary
//! button rightmost.

use super::super::{Pal, own_window};
use egui::{Align, Color32, Layout, Rect, RichText, Sense, Ui, UiBuilder, pos2, vec2};

const BUTTON_H: f32 = 30.0;
/// Between the body and the footer's line, and between the line and the buttons.
const ABOVE_LINE: f32 = 10.0;
const BELOW_LINE: f32 = 12.0;
/// Under the buttons, on top of the dialog's own padding: they never sit on the window's edge.
const BOTTOM: f32 = 6.0;
const HEIGHT: f32 = ABOVE_LINE + BELOW_LINE + BUTTON_H + BOTTOM;

/// Between a dialog drawn inside the main window and the window's edges.
const SCREEN_GAP: f32 = 16.0;
/// The least a dialog's body gets, however small the window (a dialog window's MIN_SIZE leaves
/// every dialog more than this).
const MIN_BODY: f32 = 120.0;

/// The buttons at the right of a footer.
#[derive(Clone, Copy, Default)]
pub(crate) struct Buttons<'a> {
    primary: Option<Primary<'a>>,
    cancel: Option<&'a str>,
}

#[derive(Clone, Copy)]
struct Primary<'a> {
    text: &'a str,
    enabled: bool,
    danger: bool,
    hint: Option<&'a str>,
    disabled_hint: Option<&'a str>,
}

impl<'a> Buttons<'a> {
    /// The dialog's main answer: rightmost, in the accent colour.
    pub fn primary(text: &'a str, enabled: bool) -> Self {
        let primary = Primary { text, enabled, danger: false, hint: None, disabled_hint: None };
        Buttons { primary: Some(primary), cancel: None }
    }

    /// A plain button left of the primary one ("Cancel", or "Close" when nothing is pending).
    pub fn cancel(self, text: &'a str) -> Self {
        Buttons { cancel: Some(text), ..self }
    }

    /// The primary answer removes something: in the danger colour.
    pub fn danger(mut self) -> Self {
        if let Some(p) = &mut self.primary {
            p.danger = true;
        }
        self
    }

    /// Tooltip on the primary button.
    pub fn hint(mut self, text: &'a str) -> Self {
        if let Some(p) = &mut self.primary {
            p.hint = Some(text);
        }
        self
    }

    /// Tooltip on the primary button while it is disabled (what is missing).
    pub fn disabled_hint(mut self, text: &'a str) -> Self {
        if let Some(p) = &mut self.primary {
            p.disabled_hint = Some(text);
        }
        self
    }
}

/// Which of the right-hand buttons were clicked.
pub(crate) struct Clicked {
    pub primary: bool,
    pub cancel: bool,
}

/// A dialog's footer: a line across, then [`button_row`]. In the dialog's own window it sits on
/// the window's bottom edge, over anything the body pushed that far (the body is sized with
/// [`body_height`] to end above it).
pub(crate) fn footer(ui: &mut Ui, pal: &Pal, buttons: Buttons<'_>, left: impl FnOnce(&mut Ui)) -> Clicked {
    let builder = match own_window::filled_body(ui) {
        Some(body) => {
            let rect = Rect::from_min_max(pos2(body.left(), body.bottom() - HEIGHT), body.right_bottom());
            ui.painter().rect_filled(rect, 0.0, pal.surface);
            UiBuilder::new().max_rect(rect)
        }
        None => UiBuilder::new(),
    };
    ui.scope_builder(builder, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), ABOVE_LINE + BELOW_LINE), Sense::hover());
        ui.painter().hline(rect.x_range(), rect.top() + ABOVE_LINE, ui.visuals().widgets.noninteractive.bg_stroke);
        let clicked = button_row(ui, pal, buttons, left);
        ui.add_space(BOTTOM);
        clicked
    })
    .inner
}

/// The footer's buttons on their own (a confirmation inside a page): `left` from the left edge,
/// then `[cancel] [primary]` at the right.
pub(crate) fn button_row(ui: &mut Ui, pal: &Pal, buttons: Buttons<'_>, left: impl FnOnce(&mut Ui)) -> Clicked {
    let mut clicked = Clicked { primary: false, cancel: false };
    ui.allocate_ui_with_layout(vec2(ui.available_width(), BUTTON_H), Layout::right_to_left(Align::Center), |ui| {
        if let Some(p) = buttons.primary {
            clicked.primary = primary(ui, pal, p).clicked();
        }
        if let Some(text) = buttons.cancel {
            clicked.cancel = super::plain_button(ui, text).clicked();
        }
        // The room left of the buttons, filled from the left edge.
        ui.with_layout(Layout::left_to_right(Align::Center), left);
    });
    clicked
}

fn primary(ui: &mut Ui, pal: &Pal, p: Primary<'_>) -> egui::Response {
    let mut r = if p.danger {
        let b = egui::Button::new(RichText::new(p.text).color(Color32::WHITE).strong())
            .fill(pal.danger)
            .min_size(vec2(90.0, BUTTON_H));
        ui.add_enabled(p.enabled, b)
    } else {
        super::primary_button(ui, p.text, pal, p.enabled)
    };
    if let Some(h) = p.hint {
        r = r.on_hover_text(h);
    }
    if let Some(h) = p.disabled_hint {
        r = r.on_disabled_hover_text(h);
    }
    r
}

/// The height for a dialog's body (it starts at `ui`'s cursor) so that the footer fits under it:
/// in the dialog's own window, all of the window down to the footer; inside the main window,
/// `in_main` (the dialog's preferred height) as long as the whole dialog stays clear of the
/// window's edges.
pub(crate) fn body_height(ui: &Ui, in_main: f32) -> f32 {
    let own = own_window::filled_body(ui).is_some();
    let room = if own {
        ui.available_height()
    } else {
        let above = ui.cursor().top() - ui.min_rect().top();
        ui.ctx().content_rect().height() - 2.0 * (SCREEN_GAP + f32::from(super::MODAL_PAD)) - above
    };
    let fit = room - ui.spacing().item_spacing.y - HEIGHT;
    if own { fit } else { in_main.min(fit) }.max(MIN_BODY)
}
