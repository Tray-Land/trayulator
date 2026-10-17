//! The flyout's content: the calculator (display, keypad and history) and, swapped in when opened,
//! the settings page. Both are laid out in DIPs on the fixed-size flyout and drawn every frame;
//! `elements` describes the same layout for hit testing, focus, tooltips and UI Automation.
//!
//! Enter, "=" or the = key replaces the expression with its result, and the equation moves to the
//! line above, like Windows Calculator. Nothing here runs while the flyout is hidden.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use windows::ApplicationModel::StartupTaskState;
use windows::Win32::Graphics::DirectWrite::{DWRITE_HIT_TEST_METRICS, IDWriteTextLayout};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    VIRTUAL_KEY, VK_A, VK_BACK, VK_C, VK_DELETE, VK_DOWN, VK_END, VK_HOME, VK_LEFT, VK_RETURN, VK_RIGHT, VK_SPACE, VK_UP,
    VK_V, VK_X,
};
use windows::core::BOOL;

use super::anim::{Ease, Tween};
use super::entry::TextEntry;
use super::graphics::{self, Align, Canvas, TextStyle, TextSystem};
use super::palette::Color;
use super::widgets::{self, ButtonKind, CONTROL_RADIUS, SWITCH_SIZE, State};
use super::{Element, Id, Rect, Role};
use crate::app::DISPLAY_NAME;
use crate::calc::engine::{self, Number};
use crate::calc::history::CalculationHistory;
use crate::{clipboard, settings, startup};

/// The flyout's size in logical pixels.
pub const WIDTH: f32 = 340.0;
pub const HEIGHT: f32 = 520.0;

const PADDING: f32 = 12.0;
const PADDING_TOP: f32 = 10.0;
const ROW_SPACING: f32 = 10.0;
const HEADER_HEIGHT: f32 = 32.0;
const ICON_BUTTON_WIDTH: f32 = 36.0;
const CARD_HEIGHT: f32 = 91.0;
const EQUATION_HEIGHT: f32 = 20.0;
const KEY_SPACING: f32 = 4.0;
const HISTORY_ITEM_HEIGHT: f32 = 60.0;
const HISTORY_ITEM_MARGIN: (f32, f32) = (4.0, 2.0);
/// How far a view slides when the calculator and settings swap.
const PAGE_SLIDE: f32 = 48.0;
const WHEEL_STEP: f32 = 48.0;

const HISTORY_GLYPH: &str = "\u{E81C}";
const PIN_GLYPH: &str = "\u{E718}";
const PINNED_GLYPH: &str = "\u{E840}";
const SETTINGS_GLYPH: &str = "\u{E713}";
const BACK_GLYPH: &str = "\u{E72B}";
const BACKSPACE_GLYPH: &str = "\u{E750}";
const DELETE_GLYPH: &str = "\u{E74D}";
const COPY_GLYPH: &str = "\u{E8C8}";

/// What the page needs the window to do after handling input.
#[derive(Debug, PartialEq)]
pub enum Command {
    SetTrayStatus(String),
    /// Read the start-with-Windows state (off the UI thread) and pass it to `startup_state`.
    QueryStartup,
    SetStartup(bool),
    /// A toggle button or switch flipped to `on`, for UI Automation clients (Narrator) to hear.
    Toggled { id: Id, on: bool },
}

#[derive(Clone, Copy)]
enum KeyAction {
    Insert(&'static str),
    Clear,
    Backspace,
    Evaluate,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum KeyKind {
    Digit,
    Operator,
    Equals,
}

/// A keypad key. Keys never take focus, so the entry keeps its caret and selection while they're
/// clicked.
struct Key {
    label: &'static str,
    action: KeyAction,
    kind: KeyKind,
    name: &'static str,
    tooltip: Option<&'static str>,
    row: u8,
    column: u8,
    span: u8,
}

const fn operator(label: &'static str, action: KeyAction, name: &'static str, row: u8, column: u8) -> Key {
    Key { label, action, kind: KeyKind::Operator, name, tooltip: None, row, column, span: 1 }
}

const fn digit(label: &'static str, row: u8, column: u8) -> Key {
    Key { label, action: KeyAction::Insert(label), kind: KeyKind::Digit, name: label, tooltip: None, row, column, span: 1 }
}

const KEYS: [Key; 23] = [
    operator("(", KeyAction::Insert("("), "Open bracket", 0, 0),
    operator(")", KeyAction::Insert(")"), "Close bracket", 0, 1),
    operator("√x", KeyAction::Insert("sqrt("), "Square root", 0, 2),
    operator("xʸ", KeyAction::Insert("^"), "Power", 0, 3),
    Key { tooltip: Some("Clear"), ..operator("C", KeyAction::Clear, "Clear", 1, 0) },
    Key { tooltip: Some("Backspace"), ..operator(BACKSPACE_GLYPH, KeyAction::Backspace, "Backspace", 1, 1) },
    operator("π", KeyAction::Insert("π"), "Pi", 1, 2),
    operator("÷", KeyAction::Insert("÷"), "Divide", 1, 3),
    digit("7", 2, 0),
    digit("8", 2, 1),
    digit("9", 2, 2),
    operator("×", KeyAction::Insert("×"), "Multiply", 2, 3),
    digit("4", 3, 0),
    digit("5", 3, 1),
    digit("6", 3, 2),
    operator("−", KeyAction::Insert("−"), "Minus", 3, 3),
    digit("1", 4, 0),
    digit("2", 4, 1),
    digit("3", 4, 2),
    operator("+", KeyAction::Insert("+"), "Plus", 4, 3),
    Key { span: 2, ..digit("0", 5, 0) },
    Key { name: "Decimal point", ..digit(".", 5, 2) },
    Key {
        label: "=",
        action: KeyAction::Evaluate,
        kind: KeyKind::Equals,
        name: "Equals",
        tooltip: Some("Evaluate (Enter)"),
        row: 5,
        column: 3,
        span: 1,
    },
];

/// After a result, a number starts a new expression while an operator continues from it.
fn starts_new_expression(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '(' | 'π')
}

/// Shrinks long expressions so more of them fits, like Windows Calculator's display.
fn entry_style(length: usize) -> TextStyle {
    TextStyle::display(match length {
        0..=11 => 40.0,
        12..=15 => 30.0,
        16..=21 => 22.0,
        _ => 18.0,
    })
}

/// The calculator's fixed layout.
struct CalculatorLayout {
    title: Rect,
    history_button: Rect,
    pin_button: Rect,
    settings_button: Rect,
    card: Rect,
    copied: Rect,
    equation: Rect,
    entry: Rect,
    keypad: Rect,
    history_pane: Rect,
    history_title: Rect,
    clear_history: Rect,
    history_list: Rect,
}

impl CalculatorLayout {
    fn new(text: &TextSystem, copied: bool) -> Self {
        let header = Rect::new(PADDING, PADDING_TOP, WIDTH - 2.0 * PADDING, HEADER_HEIGHT);
        let settings_button = Rect::new(header.right() - ICON_BUTTON_WIDTH, header.y, ICON_BUTTON_WIDTH, header.h);
        let pin_button = settings_button.offset(-(ICON_BUTTON_WIDTH + 4.0), 0.0);
        let history_button = pin_button.offset(-(ICON_BUTTON_WIDTH + 4.0), 0.0);
        let title = Rect::new(header.x + 4.0, header.y, history_button.x - header.x - 8.0, header.h);

        // The display: the previous equation (or an error) above a large, editable entry.
        let card = Rect::new(PADDING, header.bottom() + ROW_SPACING, header.w, CARD_HEIGHT);
        let inner = card.inset(13.0, 9.0, 5.0, 9.0);
        let line = Rect::new(inner.x, inner.y, inner.w - 8.0, EQUATION_HEIGHT);
        let badge_width = 16.0 + text.measure("Copied", TextStyle::CAPTION, f32::MAX, false).0;
        let copied_rect = Rect::new(line.x, line.y, badge_width, line.h);
        let equation = if copied { line.inset(badge_width + 8.0, 0.0, 0.0, 0.0) } else { line };
        let entry = Rect::new(inner.x, line.bottom(), inner.w - 8.0, inner.bottom() - line.bottom());

        let keypad = keypad_rect();
        let pane_header = history_header_rect();
        let clear_history = Rect::new(pane_header.right() - ICON_BUTTON_WIDTH, pane_header.y, ICON_BUTTON_WIDTH, HEADER_HEIGHT);
        let history_title = pane_header.inset(12.0, 0.0, ICON_BUTTON_WIDTH, 0.0);

        Self {
            title,
            history_button,
            pin_button,
            settings_button,
            card,
            copied: copied_rect,
            equation,
            entry,
            keypad,
            history_pane: keypad,
            history_title,
            clear_history,
            history_list: history_list_rect(),
        }
    }

    fn key(&self, key: &Key) -> Rect {
        let width = (self.keypad.w - 3.0 * KEY_SPACING) / 4.0;
        let height = (self.keypad.h - 5.0 * KEY_SPACING) / 6.0;
        let x = self.keypad.x + f32::from(key.column) * (width + KEY_SPACING);
        let y = self.keypad.y + f32::from(key.row) * (height + KEY_SPACING);
        let span = f32::from(key.span);
        Rect::new(x, y, width * span + KEY_SPACING * (span - 1.0), height)
    }

    fn history_item(&self, index: usize, scroll: f32) -> Rect {
        let (margin_x, margin_y) = HISTORY_ITEM_MARGIN;
        let pitch = HISTORY_ITEM_HEIGHT + 2.0 * margin_y;
        let y = self.history_list.y + index as f32 * pitch + margin_y - scroll;
        Rect::new(self.history_list.x + margin_x, y, self.history_list.w - 2.0 * margin_x, HISTORY_ITEM_HEIGHT)
    }

    fn history_max_scroll(&self, count: usize) -> f32 {
        let content = count as f32 * (HISTORY_ITEM_HEIGHT + 2.0 * HISTORY_ITEM_MARGIN.1);
        (content - self.history_list.h).max(0.0)
    }
}

/// The keypad fills the rest of the flyout below the display card.
fn keypad_rect() -> Rect {
    let top = PADDING_TOP + HEADER_HEIGHT + ROW_SPACING + CARD_HEIGHT + ROW_SPACING;
    Rect::new(PADDING, top, WIDTH - 2.0 * PADDING, HEIGHT - PADDING - top)
}

/// History covers the keypad, leaving the display in view: a header row, then the list. The
/// pane's border and padding take 5 DIPs on each side.
fn history_header_rect() -> Rect {
    let inner = keypad_rect().inset(5.0, 5.0, 5.0, 5.0);
    Rect::new(inner.x, inner.y, inner.w, HEADER_HEIGHT)
}

fn history_list_rect() -> Rect {
    let inner = keypad_rect().inset(5.0, 5.0, 5.0, 5.0);
    let top = history_header_rect().bottom() + 4.0;
    Rect::new(inner.x, top, inner.w, inner.bottom() - top)
}

/// The settings page's layout, which flows around wrapped descriptions.
struct SettingsLayout {
    back: Rect,
    title: Rect,
    calculator_header: Rect,
    copy_card: SettingCard,
    general_header: Rect,
    startup_card: SettingCard,
    version: Rect,
}

/// A setting as a card: a title and a description on the left, a toggle switch on the right.
struct SettingCard {
    rect: Rect,
    title: Rect,
    description: Rect,
    switch: Rect,
}

impl SettingCard {
    fn new(text: &TextSystem, top: f32, title: &str, description: &str) -> Self {
        let rect_x = PADDING;
        let width = WIDTH - 2.0 * PADDING;
        let text_x = rect_x + 15.0;
        let text_width = width - 30.0 - 12.0 - SWITCH_SIZE.0;
        let title_height = text.measure(title, TextStyle::BODY, text_width, true).1;
        let description_height = text.measure(description, TextStyle::CAPTION, text_width, true).1;
        let content = (title_height + description_height).max(SWITCH_SIZE.1);
        let rect = Rect::new(rect_x, top, width, content + 26.0);

        let text_top = rect.y + (rect.h - title_height - description_height) / 2.0;
        let switch_x = rect.right() - 15.0 - SWITCH_SIZE.0;
        Self {
            rect,
            title: Rect::new(text_x, text_top, text_width, title_height),
            description: Rect::new(text_x, text_top + title_height, text_width, description_height),
            switch: Rect::new(switch_x, rect.y + (rect.h - SWITCH_SIZE.1) / 2.0, SWITCH_SIZE.0, SWITCH_SIZE.1),
        }
    }
}

impl SettingsLayout {
    fn new(text: &TextSystem, settings: &SettingsState) -> Self {
        // The header matches the calculator's so the swap doesn't jump.
        let back = Rect::new(PADDING, PADDING_TOP, ICON_BUTTON_WIDTH, HEADER_HEIGHT);
        let title = Rect::new(back.right() + 4.0, back.y, WIDTH - back.right() - 4.0 - PADDING, HEADER_HEIGHT);
        let section_height = text.measure("Calculator", TextStyle::BODY_STRONG, f32::MAX, false).1;

        let calculator_header = Rect::new(PADDING + 4.0, back.bottom() + ROW_SPACING, 200.0, section_height);
        let copy_card = SettingCard::new(text, calculator_header.bottom() + 8.0, COPY_RESULT_TITLE, COPY_RESULT_DESCRIPTION);
        let general_header = Rect::new(PADDING + 4.0, copy_card.rect.bottom() + 20.0, 200.0, section_height);
        let startup_card =
            SettingCard::new(text, general_header.bottom() + 8.0, STARTUP_TITLE, settings.startup_description());
        let version_height = text.measure(&settings.version, TextStyle::CAPTION, f32::MAX, false).1;
        let version = Rect::new(PADDING + 4.0, startup_card.rect.bottom() + 20.0, WIDTH - 2.0 * PADDING - 4.0, version_height);

        Self { back, title, calculator_header, copy_card, general_header, startup_card, version }
    }
}

const COPY_RESULT_TITLE: &str = "Copy result on evaluate";
const COPY_RESULT_DESCRIPTION: &str = "Put each result on the clipboard when you press = or Enter.";
const STARTUP_TITLE: &str = "Start with Windows";

/// Where the start-with-Windows task stands, as far as the page knows.
#[derive(Clone, Copy, Debug, PartialEq)]
enum StartupView {
    Loading,
    /// The state Windows reported; `None` when unpackaged.
    Known(Option<StartupTaskState>),
    /// Waiting for Windows to apply a change the user made.
    Changing(bool),
}

/// Settings, shown in place of the calculator. Built fresh each time it opens, so it always
/// reflects the current startup state. Changes apply immediately.
struct SettingsState {
    copy_result: bool,
    startup: StartupView,
    version: String,
    motion: Slide,
}

impl SettingsState {
    fn startup_on(&self) -> bool {
        match self.startup {
            StartupView::Known(state) => matches!(state, Some(StartupTaskState::Enabled | StartupTaskState::EnabledByPolicy)),
            StartupView::Changing(on) => on,
            StartupView::Loading => false,
        }
    }

    /// The user (Task Manager, Settings > Apps > Startup) or policy has the final say; the app
    /// can't override it, so say where to change it instead of offering a dead toggle.
    fn startup_enabled(&self) -> bool {
        matches!(self.startup, StartupView::Known(Some(StartupTaskState::Enabled | StartupTaskState::Disabled)))
    }

    fn startup_description(&self) -> &'static str {
        match self.startup {
            StartupView::Known(Some(StartupTaskState::DisabledByUser)) => "Turned off in Settings > Apps > Startup. Turn it on there.",
            StartupView::Known(Some(StartupTaskState::DisabledByPolicy | StartupTaskState::EnabledByPolicy)) => {
                "Managed by your organization."
            }
            StartupView::Known(None) => "Only available when the app is installed.",
            _ => "Keep the tray icon ready after you sign in.",
        }
    }
}

/// A view sliding in horizontally while it fades in.
#[derive(Clone, Copy)]
struct Slide {
    offset: Tween,
    opacity: Tween,
}

impl Slide {
    fn enter(from: f32) -> Self {
        Self { offset: Tween::new(from, 0.0, 300, Ease::ExponentialOut(7.0)), opacity: Tween::new(0.0, 1.0, 200, Ease::Linear) }
    }

    fn values(&self, now: Instant) -> (f32, f32) {
        (self.offset.value(now), self.opacity.value(now))
    }

    fn is_done(&self, now: Instant) -> bool {
        self.offset.is_done(now) && self.opacity.is_done(now)
    }
}

/// History sliding up over the keypad as the keys fade out, or back down.
#[derive(Clone, Copy)]
struct HistoryMotion {
    offset: Tween,
    pane_opacity: Tween,
    keypad_opacity: Tween,
}

impl HistoryMotion {
    fn is_done(&self, now: Instant) -> bool {
        self.offset.is_done(now) && self.pane_opacity.is_done(now) && self.keypad_opacity.is_done(now)
    }
}

/// Where the entry's text was last drawn, for hit-testing clicks against it.
struct EntryView {
    layout: IDWriteTextLayout,
    x: f32,
    y: f32,
    height: f32,
}

pub struct Page {
    history: Rc<RefCell<CalculationHistory>>,
    entry: TextEntry,
    /// How far the text is scrolled right of its end-aligned position when it overflows.
    entry_scroll: f32,
    reveal_caret: bool,
    entry_view: Option<EntryView>,
    just_evaluated: bool,
    answer: Option<Number>,
    last_expression: Option<String>,
    equation: String,
    error: Option<String>,
    copied: bool,
    pinned: bool,
    history_open: bool,
    history_scroll: f32,
    history_focus: usize,
    history_motion: Option<HistoryMotion>,
    calculator_motion: Option<Slide>,
    settings: Option<SettingsState>,
    focus: Id,
    /// Draw the focus rectangle: focus last moved by keyboard.
    focus_visible: bool,
    hover: Option<Id>,
    pressed: Option<Id>,
    selecting: bool,
    caret_on: bool,
    active: bool,
}

impl Page {
    pub fn new(history: Rc<RefCell<CalculationHistory>>) -> Self {
        Self {
            history,
            entry: TextEntry::default(),
            entry_scroll: 0.0,
            reveal_caret: true,
            entry_view: None,
            just_evaluated: false,
            answer: None,
            last_expression: None,
            equation: String::new(),
            error: None,
            copied: false,
            pinned: settings::is_pinned(),
            history_open: false,
            history_scroll: 0.0,
            history_focus: 0,
            history_motion: None,
            calculator_motion: None,
            settings: None,
            focus: Id::Entry,
            focus_visible: false,
            hover: None,
            pressed: None,
            selecting: false,
            caret_on: true,
            active: false,
        }
    }

    pub fn focus(&self) -> Id {
        self.focus
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    /// The interactive element under the pointer.
    pub fn hover(&self) -> Option<Id> {
        self.hover
    }

    /// A drag is extending the entry's selection.
    pub fn is_selecting(&self) -> bool {
        self.selecting
    }

    /// The caret blinks only while the entry has focus in the active window.
    pub fn wants_caret(&self) -> bool {
        self.active && self.focus == Id::Entry && self.settings.is_none()
    }

    pub fn set_active(&mut self, active: bool) {
        self.active = active;
        self.caret_on = true;
        if !active {
            self.hover = None;
            self.pressed = None;
            self.selecting = false;
        }
    }

    pub fn blink(&mut self) {
        self.caret_on = !self.caret_on;
    }

    /// Restarts the blink with the caret shown, as typing and clicking do.
    pub fn show_caret(&mut self) {
        self.caret_on = true;
    }

    /// The flyout opened: always on the calculator, caret at the end of the entry.
    pub fn on_shown(&mut self) {
        self.settings = None;
        self.calculator_motion = None;
        self.close_history(false);
        self.hover = None;
        self.pressed = None;
        self.focus_visible = false;
        self.focus_entry();
    }

    /// Swaps the calculator for the settings page.
    pub fn show_settings(&mut self, out: &mut Vec<Command>) {
        if self.settings.is_some() {
            return;
        }

        self.close_history(false);
        self.settings = Some(SettingsState {
            copy_result: settings::copy_result_on_evaluate(),
            startup: StartupView::Loading,
            version: format!("{DISPLAY_NAME} {}", startup::package_version().as_deref().unwrap_or("(unpackaged)")),
            motion: Slide::enter(PAGE_SLIDE),
        });
        // The first setting, not the back button, whose tooltip would pop up on focus.
        self.focus = Id::CopyResultToggle;
        out.push(Command::QueryStartup);
    }

    /// Returns from settings or history to the calculator; false when already there.
    pub fn go_back(&mut self, out: &mut Vec<Command>) -> bool {
        if self.settings.take().is_some() {
            self.calculator_motion = Some(Slide::enter(-PAGE_SLIDE));
        } else if self.close_history(true) {
            out.push(Command::Toggled { id: Id::HistoryButton, on: false });
        } else {
            return false;
        }

        self.focus_entry();
        true
    }

    pub fn toggle_history(&mut self, out: &mut Vec<Command>) {
        if self.settings.is_some() {
            return;
        }
        // Ctrl+H invokes the button too, so toggle from our state.
        if self.history_open {
            self.go_back(out);
        } else {
            self.open_history();
            out.push(Command::Toggled { id: Id::HistoryButton, on: true });
        }
    }

    /// The start-with-Windows state arrived from Windows. The switch flipped, and was announced,
    /// when the user asked; it's announced again only if Windows settled on something else. The
    /// first state after opening settings isn't a flip.
    pub fn startup_state(&mut self, state: Option<StartupTaskState>, out: &mut Vec<Command>) {
        if let Some(settings) = &mut self.settings {
            let before = (settings.startup != StartupView::Loading).then(|| settings.startup_on());
            settings.startup = StartupView::Known(state);
            let on = settings.startup_on();
            if before.is_some_and(|before| before != on) {
                out.push(Command::Toggled { id: Id::StartupToggle, on });
            }
        }
    }

    pub fn is_animating(&self, now: Instant) -> bool {
        self.history_motion.is_some_and(|m| !m.is_done(now))
            || self.calculator_motion.is_some_and(|m| !m.is_done(now))
            || self.settings.as_ref().is_some_and(|s| !s.motion.is_done(now))
    }

    /// Everything on screen now, in Tab order.
    pub fn elements(&self, text: &TextSystem) -> Vec<Element> {
        match &self.settings {
            Some(settings) => self.settings_elements(text, settings),
            None => self.calculator_elements(text),
        }
    }

    /// The interactive element at a point, if any.
    pub fn hit(&self, text: &TextSystem, x: f32, y: f32) -> Option<Element> {
        self.elements(text).into_iter().rev().find(|e| e.is_interactive() && e.rect.contains(x, y))
    }

    /// Tab and Shift+Tab: the next element in the Tab order, wrapping around.
    pub fn move_focus(&mut self, text: &TextSystem, forward: bool) {
        let order: Vec<Id> = self.elements(text).into_iter().filter(|e| e.focusable && e.enabled).map(|e| e.id).collect();
        if order.is_empty() {
            return;
        }

        let next = match order.iter().position(|&id| id == self.focus) {
            Some(i) if forward => (i + 1) % order.len(),
            Some(i) => (i + order.len() - 1) % order.len(),
            None => 0,
        };
        self.set_focus(order[next], true);
    }

    /// Moves focus to an element, as UI Automation's SetFocus and keyboard navigation do.
    pub fn set_focus(&mut self, id: Id, from_keyboard: bool) {
        self.focus = id;
        self.focus_visible = from_keyboard;
        if let Id::HistoryItem(index) = id {
            self.history_focus = usize::from(index);
            self.scroll_history_to(usize::from(index));
        }
        if id == Id::Entry {
            self.reveal_caret = true;
            self.caret_on = true;
        }
    }

    pub fn pointer_move(&mut self, text: &TextSystem, x: f32, y: f32) {
        if self.selecting {
            let position = self.entry_position_at(x, y);
            self.entry.set_caret(position, true);
            self.reveal_caret = true;
            return;
        }
        self.hover = self.hit(text, x, y).map(|e| e.id);
    }

    pub fn pointer_leave(&mut self) {
        self.hover = None;
    }

    /// A press; `double` for the second click of a double-click. Returns true when the window
    /// should capture the pointer until the release.
    pub fn pointer_down(&mut self, text: &TextSystem, x: f32, y: f32, shift: bool, double: bool) -> bool {
        let Some(element) = self.hit(text, x, y) else {
            return false;
        };

        if element.id == Id::Entry {
            let position = self.entry_position_at(x, y);
            if double {
                let word = self.entry.word_at(position);
                self.entry.select(word.start, word.end);
            } else {
                self.entry.set_caret(position, shift);
            }
            self.set_focus(Id::Entry, false);
            self.selecting = !double;
            return true;
        }

        self.pressed = Some(element.id);
        if element.focusable || matches!(element.role, Role::ListItem) {
            self.set_focus(element.id, false);
        }
        true
    }

    pub fn pointer_up(&mut self, text: &TextSystem, x: f32, y: f32, out: &mut Vec<Command>) {
        self.selecting = false;
        if let Some(pressed) = self.pressed.take()
            && self.hit(text, x, y).is_some_and(|e| e.id == pressed)
        {
            self.activate(pressed, out);
        }
    }

    /// Mouse wheel over the history list scrolls it; `notches` is positive away from the user.
    pub fn wheel(&mut self, text: &TextSystem, x: f32, y: f32, notches: f32) {
        let layout = CalculatorLayout::new(text, self.copied);
        if self.history_open && self.settings.is_none() && layout.history_list.contains(x, y) {
            let max = layout.history_max_scroll(self.history.borrow().entries().len());
            self.history_scroll = (self.history_scroll - notches * WHEEL_STEP).clamp(0.0, max);
            self.hover = self.hit(text, x, y).map(|e| e.id);
        }
    }

    /// A key press the window didn't handle itself; returns whether the page used it.
    pub fn key_down(&mut self, key: VIRTUAL_KEY, ctrl: bool, shift: bool, out: &mut Vec<Command>) -> bool {
        match self.focus {
            Id::Entry => self.entry_key(key, ctrl, shift, out),
            Id::HistoryItem(index) => self.history_key(key, usize::from(index), out),
            // Toggle switches flip on Space only; buttons take Enter too.
            Id::CopyResultToggle | Id::StartupToggle if key != VK_SPACE => false,
            id if matches!(key, VK_SPACE | VK_RETURN) => {
                self.activate(id, out);
                true
            }
            _ => false,
        }
    }

    /// A typed character, after keyboard translation.
    pub fn char_input(&mut self, c: char, out: &mut Vec<Command>) {
        if self.focus != Id::Entry || c.is_control() {
            return;
        }
        if c == '=' {
            self.evaluate(out);
            return;
        }

        // Typing a number right after a result replaces the result instead of appending to it.
        let at_end = !self.entry.has_selection() && self.entry.caret() == self.entry.len();
        if self.just_evaluated && at_end && starts_new_expression(c) {
            self.entry.set_text("", None);
        }
        self.on_user_edit();
        self.entry.insert(c.encode_utf8(&mut [0; 4]));
        self.reveal_caret = true;
    }

    /// Clicks, Enter and Space on a focused control, and UI Automation's Invoke and Toggle.
    pub fn activate(&mut self, id: Id, out: &mut Vec<Command>) {
        match id {
            Id::HistoryButton => self.toggle_history(out),
            Id::PinButton => {
                self.pinned = !self.pinned;
                settings::set_is_pinned(self.pinned);
                out.push(Command::Toggled { id, on: self.pinned });
                self.focus_entry();
            }
            Id::SettingsButton => self.show_settings(out),
            Id::Key(index) => self.press_key(&KEYS[usize::from(index)], out),
            Id::ClearHistory => {
                self.history.borrow_mut().clear();
                self.history_scroll = 0.0;
                self.history_focus = 0;
                self.set_focus(Id::HistoryButton, self.focus_visible);
            }
            Id::HistoryItem(index) => {
                let expression = self.history.borrow().entries().get(usize::from(index)).map(|e| e.expression.clone());
                if let Some(expression) = expression {
                    if self.close_history(true) {
                        out.push(Command::Toggled { id: Id::HistoryButton, on: false });
                    }
                    self.on_user_edit();
                    self.set_entry(&expression);
                    self.focus_entry();
                }
            }
            Id::BackButton => {
                self.go_back(out);
            }
            Id::CopyResultToggle => {
                if let Some(settings) = &mut self.settings {
                    settings.copy_result = !settings.copy_result;
                    settings::set_copy_result_on_evaluate(settings.copy_result);
                    out.push(Command::Toggled { id, on: settings.copy_result });
                }
            }
            Id::StartupToggle => {
                if let Some(settings) = self.settings.as_mut().filter(|s| s.startup_enabled()) {
                    // The switch shows the new state while Windows applies it, so announce it now,
                    // in reply to the key press; `startup_state` corrects it if Windows says no.
                    let on = !settings.startup_on();
                    settings.startup = StartupView::Changing(on);
                    out.push(Command::Toggled { id, on });
                    out.push(Command::SetStartup(on));
                }
            }
            _ => {}
        }
    }

    /// UI Automation's ValuePattern.SetValue on the entry.
    pub fn set_entry_value(&mut self, value: &str) {
        self.on_user_edit();
        self.set_entry(value);
    }

    pub fn entry_text(&self) -> String {
        self.entry.text()
    }

    pub fn paint(&mut self, canvas: &mut Canvas, now: Instant) {
        // Finished motions are dropped so the static layout takes over.
        if self.history_motion.is_some_and(|m| m.is_done(now)) {
            self.history_motion = None;
        }
        if self.calculator_motion.is_some_and(|m| m.is_done(now)) {
            self.calculator_motion = None;
        }

        if let Some(settings) = &self.settings {
            let (offset, opacity) = settings.motion.values(now);
            canvas.layer(offset, 0.0, opacity, |c| self.paint_settings(c));
        } else {
            let (offset, opacity) = self.calculator_motion.map_or((0.0, 1.0), |m| m.values(now));
            canvas.layer(offset, 0.0, opacity, |c| self.paint_calculator(c, now));
        }

        if self.focus_visible && self.active {
            let elements = self.elements(canvas.text);
            if let Some(focused) = elements.iter().find(|e| e.id == self.focus && e.role != Role::Edit) {
                widgets::focus_rect(canvas, focused.rect, CONTROL_RADIUS);
            }
        }
    }

    fn calculator_elements(&self, text: &TextSystem) -> Vec<Element> {
        let layout = CalculatorLayout::new(text, self.copied);
        let pin_label = if self.pinned { "Unpin" } else { "Pin open" };
        let mut elements = vec![
            Element::new(Id::Title, layout.title, Role::Text, DISPLAY_NAME),
            Element::new(Id::HistoryButton, layout.history_button, Role::ToggleButton { checked: self.history_open }, "History")
                .help("History (Ctrl+H)")
                .automation_id("HistoryButton")
                .focusable(),
            Element::new(Id::PinButton, layout.pin_button, Role::ToggleButton { checked: self.pinned }, pin_label)
                .help(pin_label)
                .automation_id("PinButton")
                .focusable(),
            Element::new(Id::SettingsButton, layout.settings_button, Role::Button, "Settings")
                .help("Settings")
                .automation_id("SettingsButton")
                .focusable(),
        ];

        if self.copied {
            elements.push(Element::new(Id::Copied, layout.copied, Role::Text, "Copied"));
        }
        if let Some(error) = &self.error {
            elements.push(Element::new(Id::Error, layout.equation, Role::Text, error.as_str()));
        } else if !self.equation.is_empty() {
            elements.push(Element::new(Id::Equation, layout.equation, Role::Text, self.equation.as_str()));
        }
        elements.push(Element::new(Id::Entry, layout.entry, Role::Edit, "Expression").automation_id("Entry").focusable());

        if !self.history_open {
            for (index, key) in KEYS.iter().enumerate() {
                let mut element = Element::new(Id::Key(index as u8), layout.key(key), Role::Button, key.name);
                if let Some(tooltip) = key.tooltip {
                    element = element.help(tooltip);
                }
                elements.push(element);
            }
            return elements;
        }

        let history = self.history.borrow();
        let entries = history.entries();
        elements.push(Element::new(Id::HistoryTitle, layout.history_title, Role::Text, "History"));
        elements.push(
            Element::new(Id::ClearHistory, layout.clear_history, Role::Button, "Clear history")
                .help("Clear history")
                .automation_id("ClearHistoryButton")
                .enabled(!entries.is_empty())
                .focusable(),
        );

        if entries.is_empty() {
            elements.push(Element::new(Id::HistoryEmpty, layout.history_list, Role::Text, "There's no history yet"));
            return elements;
        }

        elements.push(Element::new(Id::HistoryList, layout.history_list, Role::List, "History").automation_id("HistoryList"));
        let focused = self.history_focus.min(entries.len() - 1);
        for (index, entry) in entries.iter().enumerate() {
            let Some(rect) = layout.history_item(index, self.history_scroll).intersect(layout.history_list) else {
                continue;
            };
            let mut element = Element::new(Id::HistoryItem(index as u16), rect, Role::ListItem, entry.to_string()).parent(Id::HistoryList);
            // The list is a single Tab stop; arrows move within it.
            element.focusable = index == focused;
            elements.push(element);
        }
        elements
    }

    fn settings_elements(&self, text: &TextSystem, settings: &SettingsState) -> Vec<Element> {
        let layout = SettingsLayout::new(text, settings);
        vec![
            Element::new(Id::BackButton, layout.back, Role::Button, "Back").help("Back (Esc)").automation_id("BackButton").focusable(),
            Element::new(Id::SettingsTitle, layout.title, Role::Text, "Settings"),
            Element::new(Id::CalculatorHeader, layout.calculator_header, Role::Text, "Calculator"),
            Element::new(Id::CopyResultToggle, layout.copy_card.switch.inflate(4.0), Role::Switch { on: settings.copy_result }, COPY_RESULT_TITLE)
                .help(COPY_RESULT_DESCRIPTION)
                .automation_id("CopyResultToggle")
                .focusable(),
            Element::new(Id::GeneralHeader, layout.general_header, Role::Text, "General"),
            Element::new(Id::StartupToggle, layout.startup_card.switch.inflate(4.0), Role::Switch { on: settings.startup_on() }, STARTUP_TITLE)
                .help(settings.startup_description())
                .automation_id("StartupToggle")
                .enabled(settings.startup_enabled())
                .focusable(),
            Element::new(Id::Version, layout.version, Role::Text, settings.version.as_str()),
        ]
    }

    fn paint_calculator(&mut self, canvas: &mut Canvas, now: Instant) {
        let layout = CalculatorLayout::new(canvas.text, self.copied);
        let p = canvas.palette;

        canvas.text(DISPLAY_NAME, TextStyle::BODY_STRONG, layout.title, Align::Leading, p.text_primary);
        self.icon_button(canvas, Id::HistoryButton, layout.history_button, HISTORY_GLYPH, Some(self.history_open), true);
        let pin_glyph = if self.pinned { PINNED_GLYPH } else { PIN_GLYPH };
        self.icon_button(canvas, Id::PinButton, layout.pin_button, pin_glyph, Some(self.pinned), true);
        self.icon_button(canvas, Id::SettingsButton, layout.settings_button, SETTINGS_GLYPH, None, true);

        widgets::card(canvas, layout.card);
        if self.copied {
            let icon = Rect::new(layout.copied.x, layout.copied.y, 12.0, layout.copied.h);
            canvas.text(COPY_GLYPH, TextStyle::icon(12.0), icon, Align::Center, p.text_tertiary);
            let label = layout.copied.inset(16.0, 0.0, 0.0, 0.0);
            canvas.text("Copied", TextStyle::CAPTION, label, Align::Leading, p.text_tertiary);
        }
        match &self.error {
            Some(error) => canvas.text(error, TextStyle::BODY, layout.equation, Align::Trailing, p.critical),
            None => canvas.text(&self.equation, TextStyle::BODY, layout.equation, Align::Trailing, p.text_secondary),
        }
        self.paint_entry(canvas, layout.entry);

        let (pane_offset, pane_opacity, keypad_opacity) = match self.history_motion {
            Some(m) => (m.offset.value(now), m.pane_opacity.value(now), m.keypad_opacity.value(now)),
            None if self.history_open => (0.0, 1.0, 0.0),
            None => (0.0, 0.0, 1.0),
        };

        if keypad_opacity > 0.0 {
            canvas.layer(0.0, 0.0, keypad_opacity, |c| {
                for (index, key) in KEYS.iter().enumerate() {
                    let id = Id::Key(index as u8);
                    let kind = match key.kind {
                        KeyKind::Digit => ButtonKind::Standard,
                        KeyKind::Operator => ButtonKind::Secondary,
                        KeyKind::Equals => ButtonKind::Accent,
                    };
                    let style = match (key.kind, key.label) {
                        (_, BACKSPACE_GLYPH) => TextStyle::icon(16.0),
                        (KeyKind::Equals, _) => TextStyle::text(20.0, false),
                        _ => TextStyle::text(18.0, false),
                    };
                    widgets::button(c, layout.key(key), kind, self.state(id, true), key.label, style);
                }
            });
        }

        if pane_opacity > 0.0 {
            canvas.layer(0.0, pane_offset, pane_opacity, |c| self.paint_history(c, &layout));
        }
    }

    fn paint_entry(&mut self, canvas: &Canvas, rect: Rect) {
        let p = canvas.palette;
        let text = self.entry.text();
        let empty = self.entry.is_empty();
        let style = entry_style(self.entry.len());
        // An empty entry shows the placeholder "0".
        let shown = if empty { "0" } else { text.as_str() };
        let Ok(layout) = canvas.text.layout(shown, style, 100_000.0, rect.h, false) else {
            return;
        };
        let Ok(metrics) = graphics::metrics(&layout) else {
            return;
        };

        let width = metrics.widthIncludingTrailingWhitespace;
        let caret_x = if empty { width } else { position_x(&layout, self.entry.caret(), self.entry.len()) };
        let overflow = (width - rect.w).max(0.0);
        let mut scroll = self.entry_scroll.clamp(0.0, overflow);
        if self.reveal_caret {
            // Scroll just enough to bring the caret into view, like a text box.
            let left = overflow - scroll;
            if caret_x < left {
                scroll = overflow - caret_x;
            } else if caret_x > left + rect.w {
                scroll = overflow - (caret_x - rect.w);
            }
            scroll = scroll.clamp(0.0, overflow);
            self.reveal_caret = false;
        }
        self.entry_scroll = scroll;

        let x = rect.right() - width + scroll;
        let y = rect.y + (rect.h - metrics.height) / 2.0;
        let focused = self.wants_caret();
        canvas.clip(rect.inset(-2.0, 0.0, -2.0, 0.0), |c| {
            if empty {
                c.draw_layout(&layout, x, y, p.text_secondary);
            } else {
                c.draw_layout(&layout, x, y, p.text_primary);
                let selection = self.entry.selection();
                if focused && !selection.is_empty() {
                    let start = position_x(&layout, selection.start, self.entry.len());
                    let end = position_x(&layout, selection.end, self.entry.len());
                    let highlight = Rect::new(x + start, y, end - start, metrics.height);
                    c.fill(highlight, 0.0, p.selected_text_background);
                    c.clip(highlight, |c| c.draw_layout(&layout, x, y, Color::rgb(0xFFFFFF)));
                }
            }

            if focused && self.caret_on && !self.entry.has_selection() {
                c.fill(Rect::new(x + caret_x - 0.5, y + 2.0, 1.5, metrics.height - 4.0), 0.0, p.text_primary);
            }
        });

        self.entry_view = (!empty).then_some(EntryView { layout, x, y, height: metrics.height });
    }

    fn paint_history(&self, canvas: &mut Canvas, layout: &CalculatorLayout) {
        let p = canvas.palette;
        widgets::card(canvas, layout.history_pane);
        canvas.text("History", TextStyle::BODY_STRONG, layout.history_title, Align::Leading, p.text_primary);

        let history = self.history.borrow();
        let entries = history.entries();
        self.icon_button(canvas, Id::ClearHistory, layout.clear_history, DELETE_GLYPH, None, !entries.is_empty());

        if entries.is_empty() {
            let message = layout.history_list.inset(12.0, 12.0, 12.0, 12.0);
            canvas.text("There's no history yet", TextStyle::BODY, message, Align::Center, p.text_secondary);
            return;
        }

        canvas.clip(layout.history_list, |c| {
            for (index, entry) in entries.iter().enumerate() {
                let item = layout.history_item(index, self.history_scroll);
                if item.bottom() < layout.history_list.y || item.y > layout.history_list.bottom() {
                    continue;
                }

                let id = Id::HistoryItem(index as u16);
                let state = self.state(id, true);
                if state.pressed {
                    c.fill(item, CONTROL_RADIUS, p.subtle_fill_tertiary);
                } else if state.hover {
                    c.fill(item, CONTROL_RADIUS, p.subtle_fill_secondary);
                }

                let content = item.inset(12.0, 6.0, 12.0, 6.0);
                let expression = Rect::new(content.x, content.y, content.w, 20.0);
                let result = Rect::new(content.x, expression.bottom(), content.w, content.h - expression.h);
                c.text(&entry.expression, TextStyle::BODY, expression, Align::Trailing, p.text_secondary);
                c.text(&entry.result, TextStyle::text(20.0, true), result, Align::Trailing, p.text_primary);
            }
        });

        // A thin scroll indicator when the list overflows.
        let max = layout.history_max_scroll(entries.len());
        if max > 0.0 {
            let track = layout.history_list.inset(layout.history_list.w - 4.0, 4.0, 1.0, 4.0);
            let visible = layout.history_list.h / (layout.history_list.h + max);
            let thumb_height = (track.h * visible).max(24.0);
            let thumb_y = track.y + (track.h - thumb_height) * (self.history_scroll / max);
            canvas.fill(Rect::new(track.x, thumb_y, track.w, thumb_height), 1.5, p.text_tertiary.fade(0.6));
        }
    }

    fn paint_settings(&self, canvas: &mut Canvas) {
        let Some(settings) = &self.settings else {
            return;
        };
        let layout = SettingsLayout::new(canvas.text, settings);
        let p = canvas.palette;

        self.icon_button(canvas, Id::BackButton, layout.back, BACK_GLYPH, None, true);
        canvas.text("Settings", TextStyle::BODY_STRONG, layout.title, Align::Leading, p.text_primary);

        canvas.text("Calculator", TextStyle::BODY_STRONG, layout.calculator_header, Align::Leading, p.text_primary);
        self.paint_setting(canvas, &layout.copy_card, Id::CopyResultToggle, COPY_RESULT_TITLE, COPY_RESULT_DESCRIPTION, settings.copy_result, true);

        canvas.text("General", TextStyle::BODY_STRONG, layout.general_header, Align::Leading, p.text_primary);
        let description = settings.startup_description();
        self.paint_setting(canvas, &layout.startup_card, Id::StartupToggle, STARTUP_TITLE, description, settings.startup_on(), settings.startup_enabled());

        canvas.text(&settings.version, TextStyle::CAPTION, layout.version, Align::Leading, p.text_tertiary);
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_setting(&self, canvas: &Canvas, card: &SettingCard, id: Id, title: &str, description: &str, on: bool, enabled: bool) {
        let p = canvas.palette;
        widgets::card(canvas, card.rect);
        canvas.wrapped_text(title, TextStyle::BODY, card.title, p.text_primary);
        canvas.wrapped_text(description, TextStyle::CAPTION, card.description, p.text_secondary);
        widgets::toggle_switch(canvas, card.switch, on, self.state(id, enabled));
    }

    /// One of the flyout's 36×32 icon buttons; `checked` is `Some` for toggle buttons.
    fn icon_button(&self, canvas: &Canvas, id: Id, rect: Rect, glyph: &str, checked: Option<bool>, enabled: bool) {
        let kind = ButtonKind::Subtle { checked: checked.unwrap_or(false) };
        widgets::button(canvas, rect, kind, self.state(id, enabled), glyph, TextStyle::icon(14.0));
    }

    fn state(&self, id: Id, enabled: bool) -> State {
        let hover = self.hover == Some(id);
        State { hover, pressed: hover && self.pressed == Some(id), enabled }
    }

    fn entry_key(&mut self, key: VIRTUAL_KEY, ctrl: bool, shift: bool, out: &mut Vec<Command>) -> bool {
        self.reveal_caret = true;
        match key {
            VK_RETURN => self.evaluate(out),
            VK_UP => match self.last_expression.clone() {
                // Recall the last evaluated expression to tweak it.
                Some(last) => {
                    self.on_user_edit();
                    self.set_entry(&last);
                }
                None => return false,
            },
            VK_LEFT | VK_RIGHT => self.entry.step(key == VK_RIGHT, ctrl, shift),
            VK_HOME => self.entry.set_caret(0, shift),
            VK_END => self.entry.set_caret(self.entry.len(), shift),
            VK_BACK | VK_DELETE => {
                if ctrl && !self.entry.has_selection() {
                    self.entry.step(key == VK_DELETE, true, true);
                }
                let changed = if key == VK_BACK { self.entry.backspace() } else { self.entry.delete() };
                if changed {
                    self.on_user_edit();
                }
            }
            VK_A if ctrl => self.entry.select_all(),
            VK_C if ctrl => {
                if self.entry.has_selection() {
                    clipboard::set_text(&self.entry.selected_text());
                }
            }
            VK_X if ctrl => {
                if self.entry.has_selection() && clipboard::set_text(&self.entry.selected_text()) {
                    self.entry.insert("");
                    self.on_user_edit();
                }
            }
            VK_V if ctrl => {
                if let Some(text) = clipboard::text() {
                    self.paste(&text, out);
                }
            }
            _ => return false,
        }
        true
    }

    fn history_key(&mut self, key: VIRTUAL_KEY, index: usize, out: &mut Vec<Command>) -> bool {
        let count = self.history.borrow().entries().len();
        let target = match key {
            VK_UP => index.saturating_sub(1),
            VK_DOWN => (index + 1).min(count.saturating_sub(1)),
            VK_HOME => 0,
            VK_END => count.saturating_sub(1),
            VK_RETURN | VK_SPACE => {
                self.activate(Id::HistoryItem(index as u16), out);
                return true;
            }
            _ => return false,
        };
        self.set_focus(Id::HistoryItem(target as u16), true);
        true
    }

    fn press_key(&mut self, key: &Key, out: &mut Vec<Command>) {
        self.reveal_caret = true;
        self.caret_on = true;
        match key.action {
            KeyAction::Insert(value) => self.insert(value),
            KeyAction::Evaluate => self.evaluate(out),
            KeyAction::Clear => {
                self.on_user_edit();
                self.set_entry("");
                self.equation.clear();
            }
            KeyAction::Backspace => {
                if self.entry.backspace() {
                    self.on_user_edit();
                }
            }
        }
    }

    /// Inserts at the caret, replacing any selection, the way typing would.
    fn insert(&mut self, value: &str) {
        if self.just_evaluated && value.starts_with(starts_new_expression) {
            self.entry.set_text("", None);
        }
        self.on_user_edit();
        self.entry.insert(value);
    }

    /// Pasted text: line breaks go, and an "=" in it evaluates, as typing one does.
    fn paste(&mut self, text: &str, out: &mut Vec<Command>) {
        let evaluate = text.contains('=');
        let clean: String = text.chars().filter(|c| !matches!(c, '=' | '\r' | '\n')).collect();
        self.on_user_edit();
        self.entry.insert(&clean);
        if evaluate {
            self.evaluate(out);
        }
    }

    fn evaluate(&mut self, out: &mut Vec<Command>) {
        let expression = self.entry.text();
        if expression.trim().is_empty() {
            return;
        }

        match engine::evaluate(&expression, self.answer) {
            Err(error) => {
                self.error = Some(error.to_string());
                self.copied = false;
            }
            Ok(answer) => {
                let expression = expression.trim().to_owned();
                self.history.borrow_mut().add(&expression, &answer.text);
                self.answer = Some(answer.value);
                self.set_entry(&answer.text);
                self.just_evaluated = true;
                self.error = None;
                self.equation = format!("{expression} =");
                self.copied = settings::copy_result_on_evaluate() && clipboard::set_text(&answer.text);
                out.push(Command::SetTrayStatus(format!("{expression} = {}", answer.text)));
                self.last_expression = Some(expression);
            }
        }
    }

    /// The user changed the expression: the last result and any error no longer apply.
    fn on_user_edit(&mut self) {
        self.just_evaluated = false;
        self.error = None;
        self.copied = false;
    }

    /// Replaces the entry text without it counting as a user edit, caret at the end.
    fn set_entry(&mut self, text: &str) {
        self.entry.set_text(text, None);
        self.entry_scroll = 0.0;
        self.reveal_caret = true;
    }

    fn focus_entry(&mut self) {
        let end = self.entry.len();
        self.entry.set_caret(end, false);
        self.set_focus(Id::Entry, false);
    }

    fn open_history(&mut self) {
        let keypad_height = keypad_rect().h;
        let keypad_opacity = self.history_motion.map_or(1.0, |m| m.keypad_opacity.value(Instant::now()));
        self.history_open = true;
        self.history_scroll = 0.0;
        self.history_focus = 0;
        self.history_motion = Some(HistoryMotion {
            offset: Tween::new(keypad_height, 0.0, 300, Ease::ExponentialOut(7.0)),
            pane_opacity: Tween::new(0.0, 1.0, 150, Ease::Linear),
            keypad_opacity: Tween::new(keypad_opacity, 0.0, 150, Ease::Linear),
        });

        if !self.history.borrow().entries().is_empty() {
            self.set_focus(Id::HistoryItem(0), self.focus_visible);
        }
    }

    fn close_history(&mut self, animate: bool) -> bool {
        if !self.history_open {
            return false;
        }

        self.history_open = false;
        self.history_motion = animate.then(|| {
            let keypad_height = keypad_rect().h;
            HistoryMotion {
                offset: Tween::new(0.0, keypad_height, 180, Ease::ExponentialIn(4.0)),
                pane_opacity: Tween::new(1.0, 0.0, 120, Ease::Linear).delayed(60),
                keypad_opacity: Tween::new(0.0, 1.0, 180, Ease::Linear),
            }
        });

        if matches!(self.focus, Id::HistoryItem(_) | Id::ClearHistory) {
            self.focus_entry();
        }
        true
    }

    fn scroll_history_to(&mut self, index: usize) {
        let pitch = HISTORY_ITEM_HEIGHT + 2.0 * HISTORY_ITEM_MARGIN.1;
        let viewport = history_list_rect().h;
        let top = index as f32 * pitch;
        if top < self.history_scroll {
            self.history_scroll = top;
        } else if top + pitch > self.history_scroll + viewport {
            self.history_scroll = top + pitch - viewport;
        }
    }

    /// The text position under a point, for clicks in the entry.
    fn entry_position_at(&self, x: f32, y: f32) -> usize {
        let Some(view) = &self.entry_view else {
            return 0;
        };
        let (mut trailing, mut inside) = (BOOL(0), BOOL(0));
        let mut metrics = DWRITE_HIT_TEST_METRICS::default();
        let y = (y - view.y).clamp(0.0, view.height - 1.0);
        if unsafe { view.layout.HitTestPoint(x - view.x, y, &mut trailing, &mut inside, &mut metrics) }.is_err() {
            return self.entry.caret();
        }
        let position = metrics.textPosition as usize + if trailing.as_bool() { metrics.length as usize } else { 0 };
        position.min(self.entry.len())
    }
}

/// The x offset of a caret before `position` in a one-line layout of `length` characters.
fn position_x(layout: &IDWriteTextLayout, position: usize, length: usize) -> f32 {
    let (index, trailing) = if position >= length && length > 0 { (length - 1, true) } else { (position, false) };
    let (mut x, mut y) = (0.0, 0.0);
    let mut metrics = DWRITE_HIT_TEST_METRICS::default();
    let _ = unsafe { layout.HitTestTextPosition(index as u32, trailing, &mut x, &mut y, &mut metrics) };
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_new_expression_numbers_and_functions_start_over() {
        assert!(starts_new_expression('7'));
        assert!(starts_new_expression('.'));
        assert!(starts_new_expression('('));
        assert!(starts_new_expression('π'));
        assert!(starts_new_expression('s'));
        assert!(!starts_new_expression('+'));
        assert!(!starts_new_expression('×'));
        assert!(!starts_new_expression('^'));
    }

    #[test]
    fn entry_style_shrinks_long_expressions() {
        assert_eq!(entry_style(11).size, 40.0);
        assert_eq!(entry_style(12).size, 30.0);
        assert_eq!(entry_style(21).size, 22.0);
        assert_eq!(entry_style(22).size, 18.0);
    }

    #[test]
    fn keys_tile_the_keypad_grid() {
        let mut cells = [[false; 4]; 6];
        for key in &KEYS {
            for column in key.column..key.column + key.span {
                let cell = &mut cells[usize::from(key.row)][usize::from(column)];
                assert!(!*cell, "{} overlaps another key", key.name);
                *cell = true;
            }
        }
        assert!(cells.iter().flatten().all(|&filled| filled));
    }

    // Unpackaged, as tests run, every setting reads as off and saving one does nothing.
    fn page() -> Page {
        Page::new(Rc::new(RefCell::new(CalculationHistory::default())))
    }

    fn settings_page() -> Page {
        let mut page = page();
        page.show_settings(&mut Vec::new());
        page.startup_state(Some(StartupTaskState::Disabled), &mut Vec::new());
        page
    }

    /// The toggle state UI Automation reports for `id`.
    fn toggle_state(page: &Page, id: Id) -> Option<bool> {
        let text = TextSystem::new().unwrap();
        match page.elements(&text).into_iter().find(|e| e.id == id)?.role {
            Role::ToggleButton { checked } => Some(checked),
            Role::Switch { on } => Some(on),
            _ => None,
        }
    }

    #[test]
    fn activate_pin_button_reports_toggle_and_focuses_entry() {
        let mut page = page();
        page.set_focus(Id::PinButton, true);
        let mut out = Vec::new();
        page.activate(Id::PinButton, &mut out);
        assert_eq!(out, [Command::Toggled { id: Id::PinButton, on: true }]);
        assert_eq!(toggle_state(&page, Id::PinButton), Some(true));
        assert_eq!(page.focus(), Id::Entry);
    }

    #[test]
    fn activate_history_button_reports_toggle_each_way() {
        let mut page = page();
        let mut out = Vec::new();
        page.activate(Id::HistoryButton, &mut out);
        assert_eq!(out, [Command::Toggled { id: Id::HistoryButton, on: true }]);
        assert_eq!(toggle_state(&page, Id::HistoryButton), Some(true));

        out.clear();
        page.activate(Id::HistoryButton, &mut out);
        assert_eq!(out, [Command::Toggled { id: Id::HistoryButton, on: false }]);
        assert_eq!(toggle_state(&page, Id::HistoryButton), Some(false));
    }

    #[test]
    fn go_back_from_history_reports_history_toggle() {
        let mut page = page();
        page.toggle_history(&mut Vec::new());
        let mut out = Vec::new();
        assert!(page.go_back(&mut out));
        assert_eq!(out, [Command::Toggled { id: Id::HistoryButton, on: false }]);
    }

    #[test]
    fn go_back_from_settings_reports_nothing() {
        let mut page = settings_page();
        let mut out = Vec::new();
        assert!(page.go_back(&mut out));
        assert!(out.is_empty());
    }

    #[test]
    fn activate_history_item_reports_history_toggle() {
        let mut page = page();
        page.history.borrow_mut().add("1+1", "2");
        page.toggle_history(&mut Vec::new());
        assert_eq!(page.focus(), Id::HistoryItem(0));

        let mut out = Vec::new();
        page.activate(Id::HistoryItem(0), &mut out);
        assert_eq!(out, [Command::Toggled { id: Id::HistoryButton, on: false }]);
        assert_eq!(page.entry_text(), "1+1");
    }

    #[test]
    fn activate_copy_result_toggle_reports_toggle_without_focus() {
        let mut page = settings_page();
        page.set_focus(Id::BackButton, true);
        let mut out = Vec::new();
        page.activate(Id::CopyResultToggle, &mut out);
        assert_eq!(out, [Command::Toggled { id: Id::CopyResultToggle, on: true }]);
        assert_eq!(toggle_state(&page, Id::CopyResultToggle), Some(true));
    }

    #[test]
    fn activate_startup_toggle_reports_toggle_with_request() {
        let mut page = settings_page();
        let mut out = Vec::new();
        page.activate(Id::StartupToggle, &mut out);
        assert_eq!(out, [Command::Toggled { id: Id::StartupToggle, on: true }, Command::SetStartup(true)]);
        assert_eq!(toggle_state(&page, Id::StartupToggle), Some(true));
    }

    #[test]
    fn startup_state_confirmed_change_reports_nothing() {
        let mut page = settings_page();
        page.activate(Id::StartupToggle, &mut Vec::new());
        let mut out = Vec::new();
        page.startup_state(Some(StartupTaskState::Enabled), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn startup_state_refused_change_reports_toggle_back() {
        let mut page = settings_page();
        page.activate(Id::StartupToggle, &mut Vec::new());
        let mut out = Vec::new();
        page.startup_state(Some(StartupTaskState::DisabledByUser), &mut out);
        assert_eq!(out, [Command::Toggled { id: Id::StartupToggle, on: false }]);
        assert_eq!(toggle_state(&page, Id::StartupToggle), Some(false));
    }

    #[test]
    fn startup_state_first_state_reports_nothing() {
        let mut page = page();
        page.show_settings(&mut Vec::new());
        let mut out = Vec::new();
        page.startup_state(Some(StartupTaskState::Enabled), &mut out);
        assert!(out.is_empty());
        assert_eq!(toggle_state(&page, Id::StartupToggle), Some(true));
    }
}
