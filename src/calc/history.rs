//! The equations evaluated this session, newest first. Kept in memory only, so it outlives the
//! flyout closing but not the app exiting, like Windows Calculator's history.

use std::fmt;

/// One evaluated equation: the expression as entered and its formatted result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub expression: String,
    pub result: String,
}

/// What Narrator reads for the entry in the history list.
impl fmt::Display for HistoryEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} = {}", self.expression, self.result)
    }
}

#[derive(Debug, Default)]
pub struct CalculationHistory {
    entries: Vec<HistoryEntry>,
}

impl CalculationHistory {
    /// The most entries kept; the oldest drop off past this.
    pub const CAPACITY: usize = 50;

    /// Newest first.
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// Records an equation. Repeating the newest one again doesn't add a duplicate.
    pub fn add(&mut self, expression: &str, result: &str) {
        let entry = HistoryEntry { expression: expression.trim().to_owned(), result: result.to_owned() };
        if self.entries.first() == Some(&entry) {
            return;
        }

        self.entries.insert(0, entry);
        self.entries.truncate(Self::CAPACITY);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(expression: &str, result: &str) -> HistoryEntry {
        HistoryEntry { expression: expression.to_owned(), result: result.to_owned() }
    }

    #[test]
    fn add_new_entries_are_newest_first() {
        let mut history = CalculationHistory::default();

        history.add("1+1", "2");
        history.add("2*3", "6");

        assert_eq!(history.entries(), [entry("2*3", "6"), entry("1+1", "2")]);
    }

    #[test]
    fn add_trims_expression() {
        let mut history = CalculationHistory::default();

        history.add("  1+1 ", "2");

        assert_eq!(history.entries()[0].expression, "1+1");
    }

    #[test]
    fn add_same_as_newest_is_not_duplicated() {
        let mut history = CalculationHistory::default();

        history.add("1+1", "2");
        history.add("1+1", "2");

        assert_eq!(history.entries().len(), 1);
    }

    #[test]
    fn add_same_as_older_is_added_again() {
        let mut history = CalculationHistory::default();

        history.add("1+1", "2");
        history.add("2*3", "6");
        history.add("1+1", "2");

        assert_eq!(history.entries().len(), 3);
        assert_eq!(history.entries()[0].expression, "1+1");
    }

    #[test]
    fn add_past_capacity_drops_oldest() {
        let mut history = CalculationHistory::default();

        for i in 0..=CalculationHistory::CAPACITY {
            history.add(&format!("{i}+0"), &i.to_string());
        }

        let entries = history.entries();
        assert_eq!(entries.len(), CalculationHistory::CAPACITY);
        assert_eq!(entries[0].expression, format!("{}+0", CalculationHistory::CAPACITY));
        assert_eq!(entries[entries.len() - 1].expression, "1+0");
    }

    #[test]
    fn history_entry_display_reads_as_equation() {
        assert_eq!(entry("2*3", "6").to_string(), "2*3 = 6");
    }

    #[test]
    fn clear_removes_everything() {
        let mut history = CalculationHistory::default();
        history.add("1+1", "2");

        history.clear();

        assert!(history.entries().is_empty());
    }
}
