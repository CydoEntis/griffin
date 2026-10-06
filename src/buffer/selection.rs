//! Selection: the text between `Buffer::anchor` and the cursor. Edits made while a
//! selection exists replace it, and the replacement undoes as one step.

use std::ops::Range;

use super::Buffer;
use super::edit::Change;
use super::movement::Motion;

impl Buffer {
    /// The selected char range, or `None` when nothing (or nothing non-empty) is
    /// selected.
    pub fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.anchor?;
        let len = self.rope.len_chars();
        let (a, b) = (anchor.min(len), self.cursor.min(len));
        let range = a.min(b)..a.max(b);
        (!range.is_empty()).then_some(range)
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection()
            .map(|range| self.rope.slice(range).to_string())
    }

    /// Moves the cursor like `move_cursor`, keeping (or dropping, on the first
    /// call) an anchor where the cursor was, so the selection grows or shrinks.
    pub fn select(&mut self, motion: Motion, page_height: usize, tab_width: usize) {
        let anchor = self.anchor.unwrap_or(self.cursor);
        self.move_cursor(motion, page_height, tab_width);
        self.anchor = Some(anchor);
    }

    pub fn select_all(&mut self) {
        self.history.seal();
        self.anchor = Some(0);
        self.cursor = self.rope.len_chars();
        self.goal_col = None;
    }

    /// Removes the selected text, leaving the cursor where it began. Returns
    /// whether there was a selection.
    pub fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection() else {
            self.anchor = None;
            return false;
        };
        let removed = self.rope.slice(range.clone()).to_string();
        self.apply(Change {
            at: range.start,
            removed,
            inserted: String::new(),
        });
        true
    }

    /// Runs `edit` after deleting the selection, all as one undo step. Without a
    /// selection `edit` runs alone and groups as it normally would.
    pub(super) fn replacing_selection(&mut self, edit: impl FnOnce(&mut Self)) {
        if self.selection().is_none() {
            self.anchor = None;
            return edit(self);
        }
        self.begin_group();
        self.delete_selection();
        edit(self);
        self.end_group();
    }

    /// Inserts pasted text (clipboard or bracketed paste) in place of any
    /// selection, as one undo step. Terminals and Windows send CRLF or a lone CR for
    /// line breaks; the rope only ever holds LF.
    pub fn paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        self.begin_group();
        self.delete_selection();
        self.insert(&text);
        self.end_group();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    /// A buffer holding `text` with the cursor at `|` and, if present, the anchor
    /// at `^`.
    fn buf(text: &str) -> Buffer {
        let (mut plain, mut cursor, mut anchor) = (String::new(), None, None);
        for ch in text.chars() {
            let at = plain.chars().count();
            match ch {
                '|' => cursor = Some(at),
                '^' => anchor = Some(at),
                _ => plain.push(ch),
            }
        }
        Buffer {
            rope: Rope::from_str(&plain),
            cursor: cursor.expect("cursor marker"),
            anchor,
            ..Buffer::empty()
        }
    }

    /// The text with `|` at the cursor and `^` at the anchor (if any).
    fn show(b: &Buffer) -> String {
        let mut out = String::new();
        for (i, ch) in b.rope.chars().chain(std::iter::once('\0')).enumerate() {
            if b.anchor == Some(i) {
                out.push('^');
            }
            if b.cursor == i {
                out.push('|');
            }
            if ch != '\0' {
                out.push(ch);
            }
        }
        out
    }

    fn sel(b: &mut Buffer, motion: Motion) {
        b.select(motion, 10, 4);
    }

    #[test]
    fn markers_round_trip() {
        for text in ["ab|cd", "a^b|cd", "a|bc^d", "|", "^|x"] {
            assert_eq!(show(&buf(text)), text);
        }
    }

    #[test]
    fn shift_motion_extends_from_the_anchor() {
        let mut b = buf("|hello world");
        for _ in 0..5 {
            sel(&mut b, Motion::Right);
        }
        assert_eq!(show(&b), "^hello| world");
        assert_eq!(b.selected_text().as_deref(), Some("hello"));
        sel(&mut b, Motion::WordRight);
        assert_eq!(b.selected_text().as_deref(), Some("hello world"));
        // Back past the anchor: the selection flips to the other side.
        let mut b = buf("ab|cd");
        sel(&mut b, Motion::Right);
        sel(&mut b, Motion::Left);
        sel(&mut b, Motion::Left);
        assert_eq!(show(&b), "a|b^cd");
        assert_eq!(b.selection(), Some(1..2));
    }

    #[test]
    fn every_motion_selects() {
        // Mid-buffer, so every motion has somewhere to go.
        let text = "one two\nthre|e four\nfive";
        for &motion in Motion::ALL {
            let mut moved = buf(text);
            let mut selected = buf(text);
            moved.move_cursor(motion, 1, 4);
            selected.select(motion, 1, 4);
            assert_eq!(selected.cursor, moved.cursor, "{motion:?}");
            assert_eq!(selected.anchor, Some(12), "{motion:?}");
            assert!(selected.selection().is_some(), "{motion:?}");
        }
    }

    #[test]
    fn shift_up_down_keep_the_goal_column() {
        let mut b = buf("abcdef\nx\nabc|def");
        sel(&mut b, Motion::Up);
        sel(&mut b, Motion::Up);
        assert_eq!(show(&b), "abc|def\nx\nabc^def");
    }

    #[test]
    fn a_plain_motion_clears_the_selection() {
        let mut b = buf("^ab|cd");
        b.move_cursor(Motion::Right, 10, 4);
        assert_eq!(show(&b), "abc|d");
        assert_eq!(b.selection(), None);
    }

    #[test]
    fn select_all_covers_the_buffer() {
        let mut b = buf("one\nt|wo");
        b.select_all();
        assert_eq!(show(&b), "^one\ntwo|");
        assert_eq!(b.selected_text().as_deref(), Some("one\ntwo"));
        let mut empty = buf("|");
        empty.select_all();
        assert_eq!(
            empty.selection(),
            None,
            "an empty selection is no selection"
        );
    }

    #[test]
    fn typing_replaces_the_selection_as_one_step() {
        let mut b = buf("^hello| world");
        for ch in ["b", "y", "e"] {
            b.type_text(ch);
        }
        assert_eq!(show(&b), "bye| world");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "hello world");
        assert!(!b.undo(), "the replacement was a single step");
    }

    #[test]
    fn typing_on_after_a_replacement_stays_in_its_step() {
        let mut b = buf("^hello| world");
        b.type_text("b");
        assert_eq!(b.selection(), None);
        b.type_text("y");
        b.newline();
        b.type_text("z");
        assert_eq!(b.rope.to_string(), "by\nz world");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "by\n world");
        assert!(b.undo());
        assert_eq!(show(&b), "hello| world");
        // A move in between ends the run as usual.
        let mut b = buf("^ab|c");
        b.type_text("x");
        b.move_cursor(Motion::Right, 10, 4);
        b.move_cursor(Motion::Left, 10, 4);
        b.type_text("y");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "xc");
    }

    #[test]
    fn enter_and_tab_replace_the_selection() {
        let mut b = buf("  a^bc|d");
        b.newline();
        assert_eq!(show(&b), "  a\n  |d");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "  abcd");
        assert!(!b.undo());

        let mut b = buf("x|yz^");
        b.tab(4, true);
        assert_eq!(show(&b), "x   |");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "xyz");
        assert!(!b.undo());
    }

    #[test]
    fn backspace_and_delete_remove_only_the_selection() {
        for edit in [Buffer::backspace as fn(&mut Buffer), Buffer::delete] {
            let mut b = buf("a^bc|d");
            edit(&mut b);
            assert_eq!(show(&b), "a|d");
            assert!(b.undo());
            assert_eq!(b.rope.to_string(), "abcd");
            assert!(!b.undo());
        }
    }

    #[test]
    fn edits_and_undo_clear_the_selection() {
        let mut b = buf("^ab|");
        b.type_text("x");
        assert_eq!(b.anchor, None);
        let mut b = buf("|ab");
        b.type_text("x");
        b.select_all();
        assert!(b.undo());
        assert_eq!(b.anchor, None);
    }

    #[test]
    fn paste_inserts_as_one_step_and_normalises_line_breaks() {
        let mut b = buf("ab|");
        b.type_text("c");
        b.paste("one\r\ntwo\rthree\n");
        assert_eq!(show(&b), "abcone\ntwo\nthree\n|");
        assert!(b.undo());
        assert_eq!(show(&b), "abc|");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "ab");
    }

    #[test]
    fn paste_replaces_the_selection() {
        let mut b = buf("x^abc|y");
        b.paste("Z");
        assert_eq!(show(&b), "xZ|y");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "xabcy");
        assert!(!b.undo());
    }

    #[test]
    fn delete_selection_without_one_does_nothing() {
        let mut b = buf("a^|b");
        assert!(!b.delete_selection());
        assert_eq!(b.rope.to_string(), "ab");
        assert!(!b.dirty);
    }
}
