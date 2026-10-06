//! Text edits. Every change to the rope goes through `Buffer::apply`, which records
//! each `Change` for undo in one place.

use super::Buffer;
use super::history::EditKind;
use super::movement::display_col;

/// Replace `removed` at char index `at` with `inserted`. An insertion has an empty
/// `removed`; a deletion an empty `inserted`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub at: usize,
    pub removed: String,
    pub inserted: String,
}

impl Buffer {
    /// The only path that mutates the rope (besides undo/redo replaying history).
    /// Records the change for undo, leaves the cursor after the inserted text and
    /// marks the buffer dirty.
    pub fn apply(&mut self, change: Change) {
        self.apply_as(change, EditKind::Other);
    }

    fn apply_as(&mut self, change: Change, kind: EditKind) {
        let cursor_before = self.cursor;
        self.apply_unrecorded(&change);
        self.history.record(change, kind, cursor_before);
    }

    /// Mutates the rope without touching history; undo and redo use it so their
    /// replays aren't recorded as new edits.
    pub(super) fn apply_unrecorded(&mut self, change: &Change) {
        let removed_len = change.removed.chars().count();
        debug_assert_eq!(
            self.rope
                .slice(change.at..change.at + removed_len)
                .to_string(),
            change.removed,
            "a Change must describe the text it removes"
        );
        self.rope.remove(change.at..change.at + removed_len);
        self.rope.insert(change.at, &change.inserted);
        self.cursor = change.at + change.inserted.chars().count();
        self.goal_col = None;
        self.dirty = true;
    }

    /// Typed text at the cursor; a run of it undoes as one step.
    pub fn type_text(&mut self, text: &str) {
        self.insert_as(text, EditKind::Typing);
    }

    /// Inserts `text` at the cursor.
    pub fn insert(&mut self, text: &str) {
        self.insert_as(text, EditKind::Other);
    }

    fn insert_as(&mut self, text: &str, kind: EditKind) {
        if text.is_empty() {
            return;
        }
        let change = Change {
            at: self.cursor,
            removed: String::new(),
            inserted: text.to_string(),
        };
        self.apply_as(change, kind);
    }

    /// Splits the line at the cursor; the new line starts with the current line's
    /// leading whitespace (only what lies before the cursor, so Enter inside the
    /// indent doesn't grow it).
    pub fn newline(&mut self) {
        let line = self.rope.char_to_line(self.cursor);
        let start = self.rope.line_to_char(line);
        let indent: String = self
            .rope
            .slice(start..self.cursor)
            .chars()
            .take_while(|&c| c == ' ' || c == '\t')
            .collect();
        self.insert_as(&format!("\n{indent}"), EditKind::Newline);
    }

    /// Deletes the char before the cursor; at column 0 that is the previous line
    /// break, joining the lines. Nothing at the start of the document.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let at = self.cursor - 1;
        self.apply(Change {
            at,
            removed: self.rope.char(at).to_string(),
            inserted: String::new(),
        });
    }

    /// Deletes the char after the cursor; at line end that is the line break,
    /// joining the lines. Nothing at the end of the document.
    pub fn delete(&mut self) {
        if self.cursor >= self.rope.len_chars() {
            return;
        }
        self.apply(Change {
            at: self.cursor,
            removed: self.rope.char(self.cursor).to_string(),
            inserted: String::new(),
        });
    }

    /// Spaces up to the next tab stop, or a literal tab.
    pub fn tab(&mut self, tab_width: usize, insert_spaces: bool) {
        if !insert_spaces {
            return self.insert("\t");
        }
        let tab_width = tab_width.max(1);
        let (line, col) = self.cursor_line_col();
        let col = display_col(self.rope.line(line), col, tab_width);
        self.insert(&" ".repeat(tab_width - col % tab_width));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    /// A buffer holding `text` with the cursor at `|`.
    fn buf(text: &str) -> Buffer {
        let cursor = text.chars().position(|c| c == '|').expect("cursor marker");
        Buffer {
            rope: Rope::from_str(&text.replacen('|', "", 1)),
            cursor,
            ..Buffer::empty()
        }
    }

    /// The text with `|` where the cursor is.
    fn show(buf: &Buffer) -> String {
        let mut text = buf.rope.to_string();
        let byte = buf.rope.char_to_byte(buf.cursor);
        text.insert(byte, '|');
        text
    }

    #[test]
    fn apply_replaces_and_moves_the_cursor() {
        let mut b = buf("|hello world");
        b.goal_col = Some(3);
        b.apply(Change {
            at: 6,
            removed: "world".into(),
            inserted: "rope".into(),
        });
        assert_eq!(show(&b), "hello rope|");
        assert!(b.dirty);
        assert_eq!(b.goal_col, None);
    }

    #[test]
    fn typing_inserts_and_advances() {
        let mut b = buf("a|c");
        b.insert("b");
        assert_eq!(show(&b), "ab|c");
        b.insert("日本");
        assert_eq!(show(&b), "ab日本|c");
    }

    #[test]
    fn enter_splits_the_line() {
        let mut b = buf("ab|cd");
        b.newline();
        assert_eq!(show(&b), "ab\n|cd");
    }

    #[test]
    fn enter_copies_leading_whitespace() {
        let mut b = buf("    if x {|\n");
        b.newline();
        assert_eq!(show(&b), "    if x {\n    |\n");

        let mut b = buf("\t  y = 1;|");
        b.newline();
        assert_eq!(show(&b), "\t  y = 1;\n\t  |");
    }

    #[test]
    fn enter_on_a_later_line_uses_that_lines_indent() {
        let mut b = buf("fn x() {\n  a|");
        b.newline();
        assert_eq!(show(&b), "fn x() {\n  a\n  |");
    }

    #[test]
    fn enter_inside_the_indent_copies_only_what_is_before_the_cursor() {
        let mut b = buf("  |  x");
        b.newline();
        assert_eq!(show(&b), "  \n  |  x");
    }

    #[test]
    fn backspace_deletes_the_char_before() {
        let mut b = buf("ab|c");
        b.backspace();
        assert_eq!(show(&b), "a|c");
    }

    #[test]
    fn backspace_at_column_zero_joins_lines() {
        let mut b = buf("ab\n|cd");
        b.backspace();
        assert_eq!(show(&b), "ab|cd");
    }

    #[test]
    fn backspace_at_document_start_does_nothing() {
        let mut b = buf("|ab");
        b.backspace();
        assert_eq!(show(&b), "|ab");
        assert!(!b.dirty);
    }

    #[test]
    fn delete_deletes_the_char_after() {
        let mut b = buf("a|bc");
        b.delete();
        assert_eq!(show(&b), "a|c");
    }

    #[test]
    fn delete_at_line_end_joins_lines() {
        let mut b = buf("ab|\ncd");
        b.delete();
        assert_eq!(show(&b), "ab|cd");
    }

    #[test]
    fn delete_at_document_end_does_nothing() {
        let mut b = buf("ab|");
        b.delete();
        assert_eq!(show(&b), "ab|");
        assert!(!b.dirty);
    }

    #[test]
    fn tab_inserts_spaces_to_the_next_stop() {
        let mut b = buf("|");
        b.tab(4, true);
        assert_eq!(show(&b), "    |");

        let mut b = buf("ab|");
        b.tab(4, true);
        assert_eq!(show(&b), "ab  |");

        let mut b = buf("abcd|");
        b.tab(4, true);
        assert_eq!(show(&b), "abcd    |");

        // A tab before the cursor already reaches column 8.
        let mut b = buf("\t\tx|");
        b.tab(8, true);
        assert_eq!(show(&b), "\t\tx       |");
    }

    #[test]
    fn tab_inserts_a_tab_when_spaces_are_off() {
        let mut b = buf("ab|");
        b.tab(4, false);
        assert_eq!(show(&b), "ab\t|");
    }

    #[test]
    fn every_edit_sets_dirty() {
        let edits: [fn(&mut Buffer); 5] = [
            |b| b.insert("x"),
            |b| b.newline(),
            |b| b.backspace(),
            |b| b.delete(),
            |b| b.tab(4, true),
        ];
        for edit in edits {
            let mut b = buf("a|b");
            assert!(!b.dirty);
            edit(&mut b);
            assert!(b.dirty, "{}", show(&b));
        }
    }
}
