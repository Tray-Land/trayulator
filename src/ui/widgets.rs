//! Drawing for the WinUI controls the flyout imitates: buttons, toggle switches, cards and the
//! keyboard focus rectangle. Metrics and colors follow the WinUI 3 control templates.

use super::Rect;
use super::graphics::{Align, Canvas, TextStyle};
use super::palette::Color;

/// ControlCornerRadius.
pub const CONTROL_RADIUS: f32 = 4.0;
/// OverlayCornerRadius, used by cards.
pub const CARD_RADIUS: f32 = 8.0;
/// A ToggleSwitch's track.
pub const SWITCH_SIZE: (f32, f32) = (40.0, 20.0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonKind {
    /// DefaultButtonStyle: a raised control.
    Standard,
    /// A raised control on ControlFillColorSecondary: operator and function keys.
    Secondary,
    /// AccentButtonStyle: the = key.
    Accent,
    /// The flyout's icon buttons: transparent until hovered, accent-filled when checked.
    Subtle { checked: bool },
}

/// Pointer and enabled state, which pick a control's colors.
#[derive(Clone, Copy, Debug)]
pub struct State {
    pub hover: bool,
    pub pressed: bool,
    pub enabled: bool,
}

pub fn button(canvas: &Canvas, rect: Rect, kind: ButtonKind, state: State, label: &str, style: TextStyle) {
    let p = canvas.palette;
    let accent = matches!(kind, ButtonKind::Accent | ButtonKind::Subtle { checked: true });
    let (fill, foreground) = if accent {
        match (state.enabled, state.pressed, state.hover) {
            (false, ..) => (p.accent_fill_disabled, p.text_on_accent_disabled),
            (true, true, _) => (p.accent_fill_tertiary, p.text_on_accent_secondary),
            (true, false, true) => (p.accent_fill_secondary, p.text_on_accent),
            _ => (p.accent_fill, p.text_on_accent),
        }
    } else {
        let rest = match kind {
            ButtonKind::Secondary => p.control_fill_secondary,
            ButtonKind::Subtle { .. } => Color::TRANSPARENT,
            _ => p.control_fill,
        };
        match (state.enabled, state.pressed, state.hover) {
            (false, ..) if matches!(kind, ButtonKind::Subtle { .. }) => (rest, p.text_disabled),
            (false, ..) => (p.control_fill_disabled, p.text_disabled),
            (true, true, _) => (p.control_fill_tertiary, p.text_secondary),
            (true, false, true) => (p.control_fill_secondary, p.text_primary),
            _ => (rest, p.text_primary),
        }
    };

    canvas.fill(rect, CONTROL_RADIUS, fill);
    if kind == ButtonKind::Secondary && state.enabled && state.hover && !state.pressed {
        // Operator keys already rest on the hover fill; lift them a little more.
        canvas.fill(rect, CONTROL_RADIUS, p.subtle_fill_secondary);
    }

    match kind {
        ButtonKind::Standard | ButtonKind::Secondary if state.enabled => {
            elevation_border(canvas, rect, p.control_stroke, p.control_stroke_secondary);
        }
        ButtonKind::Accent if state.enabled => elevation_border(canvas, rect, p.accent_stroke, p.accent_stroke_secondary),
        _ => {}
    }

    canvas.text(label, style, rect, Align::Center, foreground);
}

/// ControlElevationBorderBrush: a hairline border whose bottom edge is darker, so the control
/// looks raised.
fn elevation_border(canvas: &Canvas, rect: Rect, stroke: Color, bottom: Color) {
    let edge = 1.5;
    canvas.clip(Rect::new(rect.x, rect.y, rect.w, rect.h - edge), |c| c.stroke(rect, CONTROL_RADIUS, 1.0, stroke));
    canvas.clip(Rect::new(rect.x, rect.bottom() - edge, rect.w, edge), |c| c.stroke(rect, CONTROL_RADIUS, 1.0, bottom));
}

/// A ToggleSwitch with no header or on/off text: the track at `track`, the knob inside.
pub fn toggle_switch(canvas: &Canvas, track: Rect, on: bool, state: State) {
    let p = canvas.palette;
    let radius = track.h / 2.0;
    let interacting = state.enabled && (state.hover || state.pressed);
    let knob_height: f32 = if interacting { 14.0 } else { 12.0 };
    let knob_width = if state.enabled && state.pressed { 17.0 } else { knob_height };

    let (knob_color, knob_center) = if on {
        let fill = match (state.enabled, state.pressed, state.hover) {
            (false, ..) => p.accent_fill_disabled,
            (true, true, _) => p.accent_fill_tertiary,
            (true, false, true) => p.accent_fill_secondary,
            _ => p.accent_fill,
        };
        canvas.fill(track, radius, fill);
        let color = if state.enabled { p.text_on_accent } else { p.text_on_accent_disabled };
        (color, track.right() - radius - (knob_width - knob_height) / 2.0)
    } else {
        let fill = match (state.enabled, interacting) {
            (false, _) => Color::TRANSPARENT,
            (true, true) => p.control_alt_fill_tertiary,
            (true, false) => p.control_alt_fill_secondary,
        };
        canvas.fill(track, radius, fill);
        let stroke = if state.enabled { p.control_strong_stroke } else { p.control_strong_stroke_disabled };
        canvas.stroke(track, radius, 1.0, stroke);
        let color = if state.enabled { p.text_secondary } else { p.text_disabled };
        (color, track.x + radius + (knob_width - knob_height) / 2.0)
    };

    let center_y = track.y + track.h / 2.0;
    let knob = Rect::new(knob_center - knob_width / 2.0, center_y - knob_height / 2.0, knob_width, knob_height);
    canvas.fill(knob, knob_height / 2.0, knob_color);
}

/// The settings-card pattern: CardBackgroundFillColorDefault with CardStrokeColorDefault.
pub fn card(canvas: &Canvas, rect: Rect) {
    canvas.fill(rect, CARD_RADIUS, canvas.palette.card_background);
    canvas.stroke(rect, CARD_RADIUS, 1.0, canvas.palette.card_stroke);
}

/// The keyboard focus visual: a 2 px outer ring with a 1 px inner one, just outside `rect`.
pub fn focus_rect(canvas: &Canvas, rect: Rect, radius: f32) {
    canvas.stroke(rect.inflate(3.0), radius + 3.0, 2.0, canvas.palette.focus_stroke_outer);
    canvas.stroke(rect.inflate(1.0), radius + 1.0, 1.0, canvas.palette.focus_stroke_inner);
}
