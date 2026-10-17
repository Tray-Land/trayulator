//! The flyout: one borderless window, drawn with Direct2D and DirectWrite over the shell's acrylic
//! backdrop, styled after the WinUI controls the C# version used.

mod a11y;
mod anim;
mod entry;
pub mod flyout;
mod graphics;
mod page;
mod palette;
mod tooltip;
mod widgets;

/// A rectangle in device-independent pixels (1/96 inch), relative to the flyout's client area.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// Shrinks each side by its own amount, never below zero size.
    pub fn inset(&self, left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect::new(self.x + left, self.y + top, (self.w - left - right).max(0.0), (self.h - top - bottom).max(0.0))
    }

    pub fn inflate(&self, amount: f32) -> Rect {
        Rect::new(self.x - amount, self.y - amount, self.w + 2.0 * amount, self.h + 2.0 * amount)
    }

    pub fn offset(&self, dx: f32, dy: f32) -> Rect {
        Rect::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// The overlap of two rectangles, or `None` when they don't touch.
    pub fn intersect(&self, other: Rect) -> Option<Rect> {
        let (x, y) = (self.x.max(other.x), self.y.max(other.y));
        let (right, bottom) = (self.right().min(other.right()), self.bottom().min(other.bottom()));
        (right > x && bottom > y).then(|| Rect::new(x, y, right - x, bottom - y))
    }
}

/// Identifies an element of the flyout for hit testing, focus, tooltips and UI Automation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Id {
    Title,
    HistoryButton,
    PinButton,
    SettingsButton,
    Copied,
    Equation,
    Error,
    Entry,
    Key(u8),
    HistoryTitle,
    ClearHistory,
    HistoryList,
    HistoryItem(u16),
    HistoryEmpty,
    BackButton,
    SettingsTitle,
    CalculatorHeader,
    CopyResultToggle,
    GeneralHeader,
    StartupToggle,
    Version,
}

impl Id {
    /// A number unique among the flyout's elements, for UI Automation runtime IDs.
    pub fn number(self) -> i32 {
        match self {
            Id::Title => 1,
            Id::HistoryButton => 2,
            Id::PinButton => 3,
            Id::SettingsButton => 4,
            Id::Copied => 5,
            Id::Equation => 6,
            Id::Error => 7,
            Id::Entry => 8,
            Id::HistoryTitle => 9,
            Id::ClearHistory => 10,
            Id::HistoryList => 11,
            Id::HistoryEmpty => 12,
            Id::BackButton => 13,
            Id::SettingsTitle => 14,
            Id::CalculatorHeader => 15,
            Id::CopyResultToggle => 16,
            Id::GeneralHeader => 17,
            Id::StartupToggle => 18,
            Id::Version => 19,
            Id::Key(i) => 100 + i32::from(i),
            Id::HistoryItem(i) => 1000 + i32::from(i),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Text,
    Button,
    ToggleButton { checked: bool },
    Switch { on: bool },
    Edit,
    List,
    ListItem,
}

/// One element of the current layout: what it is, where, and what to call it.
#[derive(Clone, Debug)]
pub struct Element {
    pub id: Id,
    pub rect: Rect,
    pub role: Role,
    pub name: String,
    /// Tooltip text, also the UI Automation help text.
    pub help: Option<String>,
    /// UI Automation AutomationId, for elements tests and tools look up by name.
    pub automation_id: Option<&'static str>,
    pub enabled: bool,
    /// In the Tab order.
    pub focusable: bool,
    /// The containing element, for list items; `None` for the window's direct children.
    pub parent: Option<Id>,
}

impl Element {
    pub fn new(id: Id, rect: Rect, role: Role, name: impl Into<String>) -> Self {
        Self { id, rect, role, name: name.into(), help: None, automation_id: None, enabled: true, focusable: false, parent: None }
    }

    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn automation_id(mut self, id: &'static str) -> Self {
        self.automation_id = Some(id);
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn focusable(mut self) -> Self {
        self.focusable = true;
        self
    }

    pub fn parent(mut self, parent: Id) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Takes clicks: everything but plain text and list containers.
    pub fn is_interactive(&self) -> bool {
        self.enabled && !matches!(self.role, Role::Text | Role::List)
    }
}
