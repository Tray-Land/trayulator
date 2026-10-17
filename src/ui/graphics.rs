//! Direct2D and DirectWrite: a window's render target, cached text formats, and a `Canvas` that
//! draws in DIPs with an offset and opacity applied, so whole views can slide and fade.

use std::cell::RefCell;
use std::collections::HashMap;

use windows::Win32::Foundation::{D2DERR_RECREATE_TARGET, HWND, RECT};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_PRESENT_OPTIONS_NONE,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS, DWRITE_TRIMMING,
    DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP, DWRITE_WORD_WRAPPING_WRAP, DWriteCreateFactory,
    IDWriteFactory, IDWriteFontCollection, IDWriteTextFormat, IDWriteTextLayout,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;
use windows::core::{BOOL, HSTRING, Result, w};
use windows_numerics::{Matrix3x2, Vector2};

use super::Rect;
use super::palette::{Color, Palette};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Font {
    /// Segoe UI Variable Text: body copy, labels and keys.
    Text,
    /// Segoe UI Variable Display: the large entry.
    Display,
    /// Segoe Fluent Icons (Segoe MDL2 Assets on Windows 10).
    Icons,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub font: Font,
    pub size: f32,
    pub semibold: bool,
}

impl TextStyle {
    pub const BODY: Self = Self::text(14.0, false);
    pub const BODY_STRONG: Self = Self::text(14.0, true);
    pub const CAPTION: Self = Self::text(12.0, false);

    pub const fn text(size: f32, semibold: bool) -> Self {
        Self { font: Font::Text, size, semibold }
    }

    pub const fn display(size: f32) -> Self {
        Self { font: Font::Display, size, semibold: true }
    }

    pub const fn icon(size: f32) -> Self {
        Self { font: Font::Icons, size, semibold: false }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Leading,
    Center,
    Trailing,
}

impl Align {
    fn dwrite(self) -> DWRITE_TEXT_ALIGNMENT {
        match self {
            Align::Leading => DWRITE_TEXT_ALIGNMENT_LEADING,
            Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
            Align::Trailing => DWRITE_TEXT_ALIGNMENT_TRAILING,
        }
    }
}

/// DirectWrite with the flyout's fonts resolved and its text formats cached.
pub struct TextSystem {
    factory: IDWriteFactory,
    text_family: HSTRING,
    display_family: HSTRING,
    icon_family: HSTRING,
    formats: RefCell<HashMap<(Font, u32, bool, bool), IDWriteTextFormat>>,
}

impl TextSystem {
    pub fn new() -> Result<Self> {
        let factory: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let mut collection: Option<IDWriteFontCollection> = None;
        unsafe { factory.GetSystemFontCollection(&mut collection, false)? };

        // Windows 10 has neither Segoe UI Variable nor Segoe Fluent Icons.
        let pick = |preferred: &str, fallback: &str| {
            let (mut index, mut exists) = (0, BOOL(0));
            let found = collection.as_ref().is_some_and(|c| unsafe {
                c.FindFamilyName(&HSTRING::from(preferred), &mut index, &mut exists).is_ok() && exists.as_bool()
            });
            HSTRING::from(if found { preferred } else { fallback })
        };

        Ok(Self {
            text_family: pick("Segoe UI Variable Text", "Segoe UI"),
            display_family: pick("Segoe UI Variable Display", "Segoe UI"),
            icon_family: pick("Segoe Fluent Icons", "Segoe MDL2 Assets"),
            factory,
            formats: RefCell::default(),
        })
    }

    /// Lays `text` out in a `width` × `height` box. Unwrapped text stays on one line and is
    /// trimmed with an ellipsis when it doesn't fit.
    pub fn layout(&self, text: &str, style: TextStyle, width: f32, height: f32, wrap: bool) -> Result<IDWriteTextLayout> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe { self.factory.CreateTextLayout(&wide, &self.format(style, wrap)?, width.max(0.0), height.max(0.0)) }
    }

    /// The width and height of `text`, wrapped to `width` when `wrap` is set.
    pub fn measure(&self, text: &str, style: TextStyle, width: f32, wrap: bool) -> (f32, f32) {
        let metrics = self.layout(text, style, width, f32::MAX, wrap).and_then(|layout| metrics(&layout));
        metrics.map(|m| (m.widthIncludingTrailingWhitespace, m.height)).unwrap_or((0.0, style.size * 1.33))
    }

    fn format(&self, style: TextStyle, wrap: bool) -> Result<IDWriteTextFormat> {
        let key = (style.font, (style.size * 100.0) as u32, style.semibold, wrap);
        if let Some(format) = self.formats.borrow().get(&key) {
            return Ok(format.clone());
        }

        let family = match style.font {
            Font::Text => &self.text_family,
            Font::Display => &self.display_family,
            Font::Icons => &self.icon_family,
        };
        let weight = if style.semibold { DWRITE_FONT_WEIGHT_SEMI_BOLD } else { DWRITE_FONT_WEIGHT_NORMAL };
        let format = unsafe {
            self.factory.CreateTextFormat(
                family,
                None,
                weight,
                DWRITE_FONT_STYLE_NORMAL,
                DWRITE_FONT_STRETCH_NORMAL,
                style.size,
                w!("en-us"),
            )?
        };

        unsafe {
            if wrap {
                format.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP)?;
            } else {
                format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                let trimming = DWRITE_TRIMMING { granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER, ..Default::default() };
                let ellipsis = self.factory.CreateEllipsisTrimmingSign(&format)?;
                format.SetTrimming(&trimming, &ellipsis)?;
            }
        }

        self.formats.borrow_mut().insert(key, format.clone());
        Ok(format)
    }
}

pub fn metrics(layout: &IDWriteTextLayout) -> Result<DWRITE_TEXT_METRICS> {
    let mut metrics = DWRITE_TEXT_METRICS::default();
    unsafe { layout.GetMetrics(&mut metrics)? };
    Ok(metrics)
}

/// A window's Direct2D state. The render target is created on first paint and again after the
/// device is lost.
pub struct Graphics {
    factory: ID2D1Factory,
    target: Option<(ID2D1HwndRenderTarget, ID2D1SolidColorBrush)>,
    pub text: TextSystem,
}

impl Graphics {
    pub fn new() -> Result<Self> {
        let factory = unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        Ok(Self { factory, target: None, text: TextSystem::new()? })
    }

    /// Draws a frame: clears to `background`, then hands `paint` a canvas in DIPs.
    pub fn draw(&mut self, hwnd: HWND, dpi: u32, palette: &Palette, background: Color, paint: impl FnOnce(&mut Canvas)) {
        let Ok((target, brush)) = self.target(hwnd, dpi) else {
            return;
        };

        unsafe {
            target.BeginDraw();
            target.SetTransform(&Matrix3x2::identity());
            target.Clear(Some(&d2d_color(background)));
        }

        let mut canvas = Canvas { target: &target, brush: &brush, text: &self.text, palette, opacity: 1.0, offset: (0.0, 0.0) };
        paint(&mut canvas);

        if let Err(error) = unsafe { target.EndDraw(None, None) }
            && error.code() == D2DERR_RECREATE_TARGET
        {
            self.target = None;
        }
    }

    fn target(&mut self, hwnd: HWND, dpi: u32) -> Result<(ID2D1HwndRenderTarget, ID2D1SolidColorBrush)> {
        let mut client = RECT::default();
        unsafe { GetClientRect(hwnd, &mut client)? };
        let size = D2D_SIZE_U { width: (client.right - client.left) as u32, height: (client.bottom - client.top) as u32 };

        if let Some((target, brush)) = &self.target {
            unsafe {
                if target.GetPixelSize() != size {
                    target.Resize(&size)?;
                }
                target.SetDpi(dpi as f32, dpi as f32);
            }
            return Ok((target.clone(), brush.clone()));
        }

        let properties = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: dpi as f32,
            dpiY: dpi as f32,
            ..Default::default()
        };
        let window = D2D1_HWND_RENDER_TARGET_PROPERTIES { hwnd, pixelSize: size, presentOptions: D2D1_PRESENT_OPTIONS_NONE };
        let target = unsafe { self.factory.CreateHwndRenderTarget(&properties, &window)? };
        let brush = unsafe { target.CreateSolidColorBrush(&d2d_color(Color::TRANSPARENT), None)? };

        // ClearType needs an opaque background; over the backdrop, text is grayscale antialiased.
        unsafe { target.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE) };
        self.target = Some((target.clone(), brush.clone()));
        Ok((target, brush))
    }
}

/// Draws in DIPs, shifted by `offset` and faded by `opacity`.
pub struct Canvas<'a> {
    target: &'a ID2D1HwndRenderTarget,
    brush: &'a ID2D1SolidColorBrush,
    pub text: &'a TextSystem,
    pub palette: &'a Palette,
    opacity: f32,
    offset: (f32, f32),
}

impl Canvas<'_> {
    /// Runs `paint` shifted by (`dx`, `dy`) and faded by `opacity`, on top of the current state.
    pub fn layer(&mut self, dx: f32, dy: f32, opacity: f32, paint: impl FnOnce(&mut Self)) {
        let saved = (self.offset, self.opacity);
        self.offset = (self.offset.0 + dx, self.offset.1 + dy);
        self.opacity *= opacity;
        self.apply_offset();
        paint(self);
        (self.offset, self.opacity) = saved;
        self.apply_offset();
    }

    /// Runs `paint` with drawing clipped to `rect`.
    pub fn clip(&self, rect: Rect, paint: impl FnOnce(&Self)) {
        unsafe { self.target.PushAxisAlignedClip(&d2d_rect(rect), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE) };
        paint(self);
        unsafe { self.target.PopAxisAlignedClip() };
    }

    pub fn fill(&self, rect: Rect, radius: f32, color: Color) {
        if !self.set_color(color) {
            return;
        }
        unsafe {
            if radius > 0.0 {
                self.target.FillRoundedRectangle(&rounded(rect, radius), self.brush);
            } else {
                self.target.FillRectangle(&d2d_rect(rect), self.brush);
            }
        }
    }

    /// A border of `width` drawn inside `rect`, like a XAML Border's.
    pub fn stroke(&self, rect: Rect, radius: f32, width: f32, color: Color) {
        if !self.set_color(color) {
            return;
        }
        let half = width / 2.0;
        let inner = rect.inset(half, half, half, half);
        unsafe { self.target.DrawRoundedRectangle(&rounded(inner, (radius - half).max(0.0)), self.brush, width, None) };
    }

    /// One line of text in `rect`, vertically centered, trimmed with an ellipsis if too long.
    pub fn text(&self, text: &str, style: TextStyle, rect: Rect, align: Align, color: Color) {
        let Ok(layout) = self.text.layout(text, style, rect.w, rect.h, false) else {
            return;
        };
        unsafe {
            let _ = layout.SetTextAlignment(align.dwrite());
            let _ = layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
        }
        self.draw_layout(&layout, rect.x, rect.y, color);
    }

    /// Text wrapped to `rect`'s width, from its top-left corner.
    pub fn wrapped_text(&self, text: &str, style: TextStyle, rect: Rect, color: Color) {
        if let Ok(layout) = self.text.layout(text, style, rect.w, rect.h, true) {
            self.draw_layout(&layout, rect.x, rect.y, color);
        }
    }

    pub fn draw_layout(&self, layout: &IDWriteTextLayout, x: f32, y: f32, color: Color) {
        if !self.set_color(color) {
            return;
        }
        unsafe {
            self.target.DrawTextLayout(Vector2 { X: x, Y: y }, layout, self.brush, D2D1_DRAW_TEXT_OPTIONS_ENABLE_COLOR_FONT);
        }
    }

    /// Sets the brush color with the layer opacity applied; false when it's invisible.
    fn set_color(&self, color: Color) -> bool {
        let color = color.fade(self.opacity);
        if color.a <= 0.0 {
            return false;
        }
        unsafe { self.brush.SetColor(&d2d_color(color)) };
        true
    }

    fn apply_offset(&self) {
        unsafe { self.target.SetTransform(&Matrix3x2::translation(self.offset.0, self.offset.1)) };
    }
}

fn d2d_color(color: Color) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r: color.r, g: color.g, b: color.b, a: color.a }
}

fn d2d_rect(rect: Rect) -> D2D_RECT_F {
    D2D_RECT_F { left: rect.x, top: rect.y, right: rect.right(), bottom: rect.bottom() }
}

fn rounded(rect: Rect, radius: f32) -> D2D1_ROUNDED_RECT {
    D2D1_ROUNDED_RECT { rect: d2d_rect(rect), radiusX: radius, radiusY: radius }
}
