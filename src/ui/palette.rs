//! Colors from the WinUI 3 (Fluent) theme resources, for the light or dark taskbar theme.

use windows::UI::ViewManagement::UIColorType;

use crate::theme;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color::argb(0);

    /// From 0xAARRGGBB, the notation the XAML theme resources use.
    pub const fn argb(value: u32) -> Self {
        const fn channel(value: u32, shift: u32) -> f32 {
            ((value >> shift) & 0xFF) as f32 / 255.0
        }
        Self { a: channel(value, 24), r: channel(value, 16), g: channel(value, 8), b: channel(value, 0) }
    }

    /// From 0xRRGGBB, fully opaque.
    pub const fn rgb(value: u32) -> Self {
        Self::argb(0xFF00_0000 | value)
    }

    /// The same color with its alpha multiplied by `opacity`.
    pub fn fade(self, opacity: f32) -> Self {
        Self { a: self.a * opacity, ..self }
    }
}

/// The user's accent color shades, read once per theme application.
#[derive(Clone, Copy, Debug)]
pub struct Accents {
    pub accent: u32,
    pub dark1: u32,
    pub dark2: u32,
    pub light2: u32,
}

impl Accents {
    /// Reads the accent shades, falling back to the Windows default blue.
    pub fn read() -> Self {
        Self {
            accent: theme::accent(UIColorType::Accent).unwrap_or(0x0078D4),
            dark1: theme::accent(UIColorType::AccentDark1).unwrap_or(0x005FB8),
            dark2: theme::accent(UIColorType::AccentDark2).unwrap_or(0x003E92),
            light2: theme::accent(UIColorType::AccentLight2).unwrap_or(0x60CDFF),
        }
    }
}

/// The brushes the flyout draws with. Names follow the XAML resources they come from.
#[derive(Clone, Debug)]
pub struct Palette {
    /// Under everything: the acrylic's accent tint, or the solid fallback when there's no acrylic.
    pub tint: Option<Color>,
    pub solid_background: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_tertiary: Color,
    pub text_disabled: Color,
    pub control_fill: Color,
    pub control_fill_secondary: Color,
    pub control_fill_tertiary: Color,
    pub control_fill_disabled: Color,
    pub subtle_fill_secondary: Color,
    pub subtle_fill_tertiary: Color,
    pub control_stroke: Color,
    pub control_stroke_secondary: Color,
    pub card_background: Color,
    pub card_stroke: Color,
    pub critical: Color,
    pub accent_fill: Color,
    pub accent_fill_secondary: Color,
    pub accent_fill_tertiary: Color,
    pub accent_fill_disabled: Color,
    pub text_on_accent: Color,
    pub text_on_accent_secondary: Color,
    pub text_on_accent_disabled: Color,
    pub accent_stroke: Color,
    pub accent_stroke_secondary: Color,
    pub selected_text_background: Color,
    pub control_strong_stroke: Color,
    pub control_strong_stroke_disabled: Color,
    pub control_alt_fill_secondary: Color,
    pub control_alt_fill_tertiary: Color,
    pub focus_stroke_outer: Color,
    pub focus_stroke_inner: Color,
    pub surface_stroke_flyout: Color,
}

impl Palette {
    /// The theme for a light or dark taskbar; `accent_tint` gives the flyout the shell's accent
    /// tint ("Show accent color on Start and taskbar").
    pub fn new(light: bool, accents: &Accents, accent_tint: bool) -> Self {
        let accent_fill = Color::rgb(if light { accents.dark1 } else { accents.light2 });
        let tint = accent_tint.then(|| Color::rgb(accents.dark2).fade(0.7));
        let selected_text_background = Color::rgb(accents.accent);

        if light {
            Self {
                tint,
                solid_background: Color::rgb(0xF9F9F9),
                text_primary: Color::argb(0xE4000000),
                text_secondary: Color::argb(0x9E000000),
                text_tertiary: Color::argb(0x72000000),
                text_disabled: Color::argb(0x5C000000),
                control_fill: Color::argb(0xB3FFFFFF),
                control_fill_secondary: Color::argb(0x80F9F9F9),
                control_fill_tertiary: Color::argb(0x4DF9F9F9),
                control_fill_disabled: Color::argb(0x4DF9F9F9),
                subtle_fill_secondary: Color::argb(0x09000000),
                subtle_fill_tertiary: Color::argb(0x06000000),
                control_stroke: Color::argb(0x0F000000),
                control_stroke_secondary: Color::argb(0x29000000),
                card_background: Color::argb(0xB3FFFFFF),
                card_stroke: Color::argb(0x0F000000),
                critical: Color::rgb(0xC42B1C),
                accent_fill,
                accent_fill_secondary: accent_fill.fade(0.9),
                accent_fill_tertiary: accent_fill.fade(0.8),
                accent_fill_disabled: Color::argb(0x37000000),
                text_on_accent: Color::rgb(0xFFFFFF),
                text_on_accent_secondary: Color::argb(0xB3FFFFFF),
                text_on_accent_disabled: Color::rgb(0xFFFFFF),
                accent_stroke: Color::argb(0x14FFFFFF),
                accent_stroke_secondary: Color::argb(0x66000000),
                selected_text_background,
                control_strong_stroke: Color::argb(0x72000000),
                control_strong_stroke_disabled: Color::argb(0x37000000),
                control_alt_fill_secondary: Color::argb(0x06000000),
                control_alt_fill_tertiary: Color::argb(0x0F000000),
                focus_stroke_outer: Color::argb(0xE4000000),
                focus_stroke_inner: Color::argb(0xB3FFFFFF),
                surface_stroke_flyout: Color::argb(0x0F000000),
            }
        } else {
            Self {
                tint,
                solid_background: Color::rgb(0x2C2C2C),
                text_primary: Color::rgb(0xFFFFFF),
                text_secondary: Color::argb(0xC5FFFFFF),
                text_tertiary: Color::argb(0x87FFFFFF),
                text_disabled: Color::argb(0x5DFFFFFF),
                control_fill: Color::argb(0x0FFFFFFF),
                control_fill_secondary: Color::argb(0x15FFFFFF),
                control_fill_tertiary: Color::argb(0x08FFFFFF),
                control_fill_disabled: Color::argb(0x0BFFFFFF),
                subtle_fill_secondary: Color::argb(0x0FFFFFFF),
                subtle_fill_tertiary: Color::argb(0x0AFFFFFF),
                control_stroke: Color::argb(0x12FFFFFF),
                control_stroke_secondary: Color::argb(0x18FFFFFF),
                card_background: Color::argb(0x0DFFFFFF),
                card_stroke: Color::argb(0x19000000),
                critical: Color::rgb(0xFF99A4),
                accent_fill,
                accent_fill_secondary: accent_fill.fade(0.9),
                accent_fill_tertiary: accent_fill.fade(0.8),
                accent_fill_disabled: Color::argb(0x28FFFFFF),
                text_on_accent: Color::rgb(0x000000),
                text_on_accent_secondary: Color::argb(0x80000000),
                text_on_accent_disabled: Color::argb(0x87FFFFFF),
                accent_stroke: Color::argb(0x14FFFFFF),
                accent_stroke_secondary: Color::argb(0x23000000),
                selected_text_background,
                control_strong_stroke: Color::argb(0x8BFFFFFF),
                control_strong_stroke_disabled: Color::argb(0x28FFFFFF),
                control_alt_fill_secondary: Color::argb(0x19000000),
                control_alt_fill_tertiary: Color::argb(0x0BFFFFFF),
                focus_stroke_outer: Color::rgb(0xFFFFFF),
                focus_stroke_inner: Color::argb(0xB3000000),
                surface_stroke_flyout: Color::argb(0x33000000),
            }
        }
    }
}
