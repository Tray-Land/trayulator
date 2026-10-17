//! The expression entry as a model: text, caret and selection, with the editing operations a
//! Windows text box offers. Positions count chars; input is limited to the Basic Multilingual
//! Plane, so they are also the UTF-16 positions DirectWrite hit-tests in.

use std::ops::Range;

#[derive(Clone, Debug, Default)]
pub struct TextEntry {
    chars: Vec<char>,
    caret: usize,
    /// The other end of the selection; equal to `caret` when nothing is selected.
    anchor: usize,
}

impl TextEntry {
    pub fn text(&self) -> String {
        self.chars.iter().collect()
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chars.is_empty()
    }

    pub fn caret(&self) -> usize {
        self.caret
    }

    pub fn selection(&self) -> Range<usize> {
        self.caret.min(self.anchor)..self.caret.max(self.anchor)
    }

    pub fn has_selection(&self) -> bool {
        self.caret != self.anchor
    }

    pub fn selected_text(&self) -> String {
        self.chars[self.selection()].iter().collect()
    }

    /// Replaces the text, with the caret at `caret` (or the end) and nothing selected.
    pub fn set_text(&mut self, text: &str, caret: Option<usize>) {
        self.chars = text.chars().filter(|&c| accepts(c)).collect();
        let caret = caret.unwrap_or(self.chars.len()).min(self.chars.len());
        self.caret = caret;
        self.anchor = caret;
    }

    /// Moves the caret; `extend` keeps the anchor, growing or shrinking the selection.
    pub fn set_caret(&mut self, position: usize, extend: bool) {
        self.caret = position.min(self.chars.len());
        if !extend {
            self.anchor = self.caret;
        }
    }

    /// Selects from `anchor` to `caret`; the caret end moves when the selection is extended.
    pub fn select(&mut self, anchor: usize, caret: usize) {
        self.anchor = anchor.min(self.chars.len());
        self.caret = caret.min(self.chars.len());
    }

    pub fn select_all(&mut self) {
        self.select(0, self.chars.len());
    }

    /// Inserts at the caret, replacing any selection, the way typing does.
    pub fn insert(&mut self, text: &str) {
        let range = self.selection();
        let inserted: Vec<char> = text.chars().filter(|&c| accepts(c)).collect();
        let caret = range.start + inserted.len();
        self.chars.splice(range, inserted);
        self.set_caret(caret, false);
    }

    /// Deletes the selection, or the character before the caret. False when nothing changed.
    pub fn backspace(&mut self) -> bool {
        if !self.has_selection() {
            if self.caret == 0 {
                return false;
            }
            self.anchor = self.caret - 1;
        }
        self.insert("");
        true
    }

    /// Deletes the selection, or the character after the caret. False when nothing changed.
    pub fn delete(&mut self) -> bool {
        if !self.has_selection() {
            if self.caret == self.chars.len() {
                return false;
            }
            self.anchor = self.caret + 1;
        }
        self.insert("");
        true
    }

    /// Left (`forward` false) or right by a character or a word. Without `extend`, an existing
    /// selection collapses to the side moved towards, as in any Windows text box.
    pub fn step(&mut self, forward: bool, word: bool, extend: bool) {
        if self.has_selection() && !extend && !word {
            let range = self.selection();
            self.set_caret(if forward { range.end } else { range.start }, false);
            return;
        }

        let target = match (forward, word) {
            (true, true) => self.next_word_start(self.caret),
            (false, true) => self.previous_word_start(self.caret),
            (true, false) => (self.caret + 1).min(self.chars.len()),
            (false, false) => self.caret.saturating_sub(1),
        };
        self.set_caret(target, extend);
    }

    /// The run of same-class characters around `position` (what a double-click selects).
    pub fn word_at(&self, position: usize) -> Range<usize> {
        if self.chars.is_empty() {
            return 0..0;
        }

        let position = position.min(self.chars.len() - 1);
        let class = class(self.chars[position]);
        let mut start = position;
        while start > 0 && class_at(&self.chars, start - 1) == class {
            start -= 1;
        }
        let mut end = position + 1;
        while end < self.chars.len() && class_at(&self.chars, end) == class {
            end += 1;
        }
        start..end
    }

    fn next_word_start(&self, from: usize) -> usize {
        let mut i = from;
        if i < self.chars.len() {
            let class = class(self.chars[i]);
            while i < self.chars.len() && class_at(&self.chars, i) == class {
                i += 1;
            }
        }
        while i < self.chars.len() && self.chars[i].is_whitespace() {
            i += 1;
        }
        i
    }

    fn previous_word_start(&self, from: usize) -> usize {
        let mut i = from;
        while i > 0 && self.chars[i - 1].is_whitespace() {
            i -= 1;
        }
        if i > 0 {
            let class = class(self.chars[i - 1]);
            while i > 0 && class_at(&self.chars, i - 1) == class {
                i -= 1;
            }
        }
        i
    }
}

/// Printable characters only, and none that need a surrogate pair.
fn accepts(c: char) -> bool {
    !c.is_control() && u32::from(c) <= 0xFFFF
}

/// Word classes: spaces, number-or-name characters, and operators.
fn class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '.' || c == '_' {
        1
    } else {
        2
    }
}

fn class_at(chars: &[char], i: usize) -> u8 {
    class(chars[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str) -> TextEntry {
        let mut entry = TextEntry::default();
        entry.set_text(text, None);
        entry
    }

    #[test]
    fn set_text_puts_caret_at_end() {
        let entry = entry("12+3");

        assert_eq!(entry.caret(), 4);
        assert!(!entry.has_selection());
    }

    #[test]
    fn insert_replaces_selection() {
        let mut entry = entry("12+34");
        entry.select(1, 4);

        entry.insert("×");

        assert_eq!(entry.text(), "1×4");
        assert_eq!(entry.caret(), 2);
    }

    #[test]
    fn insert_drops_line_breaks_and_controls() {
        let mut entry = entry("");

        entry.insert("1\r\n+\t2");

        assert_eq!(entry.text(), "1+2");
    }

    #[test]
    fn backspace_at_start_does_nothing() {
        let mut entry = entry("12");
        entry.set_caret(0, false);

        assert!(!entry.backspace());
        assert_eq!(entry.text(), "12");
    }

    #[test]
    fn backspace_deletes_before_caret() {
        let mut entry = entry("123");
        entry.set_caret(2, false);

        assert!(entry.backspace());
        assert_eq!(entry.text(), "13");
        assert_eq!(entry.caret(), 1);
    }

    #[test]
    fn delete_removes_selection() {
        let mut entry = entry("1234");
        entry.select(3, 1);

        assert!(entry.delete());
        assert_eq!(entry.text(), "14");
        assert_eq!(entry.caret(), 1);
    }

    #[test]
    fn step_without_extend_collapses_selection() {
        let mut entry = entry("12345");
        entry.select(1, 4);

        entry.step(false, false, false);

        assert_eq!(entry.caret(), 1);
        assert!(!entry.has_selection());
    }

    #[test]
    fn step_with_extend_grows_selection() {
        let mut entry = entry("12345");

        entry.step(false, false, true);
        entry.step(false, false, true);

        assert_eq!(entry.selection(), 3..5);
        assert_eq!(entry.selected_text(), "45");
    }

    #[test]
    fn step_by_word_skips_number_and_operator_runs() {
        let mut entry = entry("12.5 + sqrt(4)");
        entry.set_caret(0, false);

        entry.step(true, true, false);
        assert_eq!(entry.caret(), 5);
        entry.step(true, true, false);
        assert_eq!(entry.caret(), 7);
        entry.step(false, true, false);
        assert_eq!(entry.caret(), 5);
        entry.step(false, true, false);
        assert_eq!(entry.caret(), 0);
    }

    #[test]
    fn word_at_selects_the_number_under_the_caret() {
        let entry = entry("12.5*300");

        assert_eq!(entry.word_at(1), 0..4);
        assert_eq!(entry.word_at(4), 4..5);
        assert_eq!(entry.word_at(99), 5..8);
    }
}
