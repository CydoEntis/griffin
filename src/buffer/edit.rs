//! Text edits. Every change to the rope goes through `Buffer::apply`, which records
//! each `Change` for undo in one place.

use std::ops::Range;

use super::Buffer;
use super::history::EditKind;
use super::movement::display_col;
use crate::highlight::input_edit;

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
        if let Some(highlighter) = &mut self.highlighter {
            let edit = input_edit(&self.rope, change.at, removed_len, &change.inserted);
            highlighter.edit(&edit);
        }
        self.move_breakpoints(change.at, removed_len, &change.inserted);
        self.rope.remove(change.at..change.at + removed_len);
        self.rope.insert(change.at, &change.inserted);
        self.revision += 1;
        self.cursor = change.at + change.inserted.chars().count();
        self.goal_col = None;
        // Positions after the change have shifted, so an old anchor means nothing.
        self.anchor = None;
        self.dirty = true;
    }

    /// Moves the breakpoints with their lines across replacing `removed_len`
    /// chars at `at` with `inserted`; called before the rope changes, as it
    /// reads the old line breaks. A line whose break the change removes joins the
    /// line the change starts on, and its breakpoint goes with it.
    fn move_breakpoints(&mut self, at: usize, removed_len: usize, inserted: &str) {
        if self.breakpoints.is_empty() {
            return;
        }
        let first = self.rope.char_to_line(at);
        let removed = self.rope.char_to_line(at + removed_len) - first;
        // Counted the way the rope counts line breaks, which isn't only `\n`.
        let added = ropey::Rope::from_str(inserted).len_lines() - 1;
        // Text put in at a line's very start pushes that whole line down, as
        // Enter there does, so its breakpoint follows it.
        let pushed = removed_len == 0 && at == self.rope.line_to_char(first);
        self.breakpoints = std::mem::take(&mut self.breakpoints)
            .into_iter()
            .map(|line| {
                if line < first || (line == first && !pushed) {
                    line
                } else if line > first + removed || line == first {
                    line - removed + added
                } else {
                    first
                }
            })
            .collect();
    }

    /// Typed text at the cursor, replacing any selection; a run of it undoes as
    /// one step, together with the selection it replaced.
    pub fn type_text(&mut self, text: &str) {
        let replacing = self.selection().is_some();
        self.replacing_selection(|b| b.insert_as(text, EditKind::Typing));
        if replacing && !text.is_empty() {
            self.history.continue_typing();
        }
    }

    /// A typed character, with bracket pairing when `auto_pairs` is on: an opener
    /// brings its closer along when nothing but whitespace or a closer follows,
    /// and a closer typed in front of the same closer steps over it.
    pub fn type_char(&mut self, ch: char, auto_pairs: bool) {
        let next = (self.cursor < self.rope.len_chars()).then(|| self.rope.char(self.cursor));
        if auto_pairs && self.selection().is_none() {
            if CLOSERS.contains(&ch) && next == Some(ch) {
                // A move, but part of the typing: what's typed next joins its run.
                self.cursor += 1;
                self.goal_col = None;
                self.history.step_over();
                return;
            }
            if let Some(closer) = closer_of(ch)
                && next.is_none_or(|c| c.is_whitespace() || CLOSERS.contains(&c))
            {
                // One change for the pair, so the typing after it joins the same
                // undo step even though the cursor sits inside it.
                let at = self.cursor;
                self.insert_as(&format!("{ch}{closer}"), EditKind::Typing);
                self.cursor = at + 1;
                return;
            }
        }
        self.type_text(ch.encode_utf8(&mut [0; 4]));
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
    /// indent doesn't grow it). Replaces any selection.
    pub fn newline(&mut self) {
        self.replacing_selection(Self::newline_at_cursor);
    }

    fn newline_at_cursor(&mut self) {
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
    /// break, joining the lines. Nothing at the start of the document. With a
    /// selection, deletes just the selection.
    pub fn backspace(&mut self) {
        if self.delete_selection() || self.cursor == 0 {
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
    /// joining the lines. Nothing at the end of the document. With a selection,
    /// deletes just the selection.
    pub fn delete(&mut self) {
        if self.delete_selection() || self.cursor >= self.rope.len_chars() {
            return;
        }
        self.apply(Change {
            at: self.cursor,
            removed: self.rope.char(self.cursor).to_string(),
            inserted: String::new(),
        });
    }

    /// Replaces each range with its text as one undo step (find and replace). The
    /// ranges are in the current text, sorted and not overlapping; the last is
    /// applied first so the earlier ones still point at their text. The cursor
    /// ends after the first replacement.
    pub fn replace_ranges(&mut self, edits: &[(Range<usize>, String)]) {
        self.begin_group();
        for (range, inserted) in edits.iter().rev() {
            let removed = self.rope.slice(range.clone()).to_string();
            self.apply(Change {
                at: range.start,
                removed,
                inserted: inserted.clone(),
            });
        }
        self.end_group();
    }

    /// `replace_ranges` for edits that rewrite text around the cursor rather than
    /// at it (a formatter's): the cursor stays on the text it was on, moved by
    /// the edits before it, or to the start of an edit that swallowed it. No
    /// edits, no undo step.
    pub fn apply_edits(&mut self, edits: &[(Range<usize>, String)]) {
        if edits.is_empty() {
            return;
        }
        let cursor = self.cursor;
        let mut kept = cursor.cast_signed();
        for (range, inserted) in edits {
            let grown = inserted.chars().count().cast_signed() - range.len().cast_signed();
            if range.end <= cursor && !(range.is_empty() && range.start == cursor) {
                kept += grown;
            } else if range.start < cursor {
                kept += range.start.cast_signed() - cursor.cast_signed();
                break;
            } else {
                break;
            }
        }
        self.replace_ranges(edits);
        self.cursor = kept.max(0).cast_unsigned().min(self.rope.len_chars());
    }

    /// Spaces up to the next tab stop, or a literal tab, replacing any selection.
    pub fn tab(&mut self, tab_width: usize, insert_spaces: bool) {
        self.replacing_selection(|b| b.tab_at_cursor(tab_width, insert_spaces));
    }

    fn tab_at_cursor(&mut self, tab_width: usize, insert_spaces: bool) {
        if !insert_spaces {
            return self.insert("\t");
        }
        let tab_width = tab_width.max(1);
        let (line, col) = self.cursor_line_col();
        let col = display_col(self.rope.line(line), col, tab_width);
        self.insert(&" ".repeat(tab_width - col % tab_width));
    }
}

/// The closing brackets `auto_pairs` inserts and steps over.
const CLOSERS: [char; 3] = [')', ']', '}'];

/// The closer `auto_pairs` inserts after an opening bracket.
fn closer_of(opener: char) -> Option<char> {
    match opener {
        '(' => Some(')'),
        '[' => Some(']'),
        '{' => Some('}'),
        _ => None,
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

    /// Types `text` one char at a time, as the keyboard does.
    fn type_all(b: &mut Buffer, text: &str, auto_pairs: bool) {
        for ch in text.chars() {
            b.type_char(ch, auto_pairs);
        }
    }

    #[test]
    fn an_opener_brings_its_closer_before_whitespace_or_a_closer() {
        for (before, typed, after) in [
            ("|", '(', "(|)"),
            ("a| b", '[', "a[|] b"),
            ("x|\ny", '{', "x{|}\ny"),
            ("f(|)", '(', "f((|))"),
            ("[|]", '{', "[{|}]"),
            ("a|\tb", '(', "a(|)\tb"),
        ] {
            let mut b = buf(before);
            b.type_char(typed, true);
            assert_eq!(show(&b), after, "typing {typed:?} in {before:?}");
        }
    }

    #[test]
    fn an_opener_before_other_text_comes_alone() {
        for (before, typed, after) in [
            ("|foo", '(', "(|foo"),
            ("|.x", '[', "[|.x"),
            ("|\"s\"", '{', "{|\"s\""),
        ] {
            let mut b = buf(before);
            b.type_char(typed, true);
            assert_eq!(show(&b), after, "typing {typed:?} in {before:?}");
        }
    }

    #[test]
    fn quotes_and_other_chars_never_pair() {
        let mut b = buf("|");
        type_all(&mut b, "\"'`<", true);
        assert_eq!(show(&b), "\"'`<|");
    }

    #[test]
    fn a_closer_steps_over_the_same_closer() {
        let mut b = buf("|");
        type_all(&mut b, "foo(bar)", true);
        assert_eq!(show(&b), "foo(bar)|");

        // Whether or not Glyph inserted it.
        let mut b = buf("a|]");
        b.type_char(']', true);
        assert_eq!(show(&b), "a]|");
    }

    #[test]
    fn a_closer_before_a_different_char_is_inserted() {
        let mut b = buf("a|)");
        b.type_char(']', true);
        assert_eq!(show(&b), "a]|)");
    }

    #[test]
    fn nested_brackets_type_through() {
        let mut b = buf("|");
        type_all(&mut b, "f(g[{x}])", true);
        assert_eq!(show(&b), "f(g[{x}])|");
    }

    #[test]
    fn one_undo_removes_the_run_with_its_closer() {
        let mut b = buf("x |");
        b.seal_undo_group();
        type_all(&mut b, "foo(bar", true);
        assert_eq!(show(&b), "x foo(bar|)");
        b.undo();
        assert_eq!(show(&b), "x |");
        assert!(
            !b.history.can_undo(),
            "the closer should not be a step of its own"
        );
    }

    #[test]
    fn typing_on_past_a_closer_stays_one_undo_step() {
        let mut b = buf("x |");
        b.seal_undo_group();
        type_all(&mut b, "foo(bar);", true);
        assert_eq!(show(&b), "x foo(bar);|");
        b.undo();
        assert_eq!(show(&b), "x |");
        assert!(!b.history.can_undo(), "stepping over `)` split the run");

        // Over a closer Glyph didn't insert, too.
        let mut b = buf("(|)");
        type_all(&mut b, "a);", true);
        assert_eq!(show(&b), "(a);|");
        b.undo();
        assert_eq!(show(&b), "(|)");
    }

    #[test]
    fn without_auto_pairs_typing_is_plain() {
        let mut b = buf("|)");
        type_all(&mut b, "f(x)", false);
        assert_eq!(show(&b), "f(x)|)");
    }

    #[test]
    fn a_paste_never_pairs() {
        let mut b = buf("|");
        b.paste("(");
        assert_eq!(show(&b), "(|");
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
    fn apply_edits_keeps_the_cursor_on_its_text_and_undoes_in_one_step() {
        let mut b = buf("fn  a( ){\n x|=1;}\n");
        b.apply_edits(&[
            (2..4, " ".into()),
            (5..8, "() ".into()),
            (10..11, "    ".into()),
            (12..13, " = ".into()),
        ]);
        assert_eq!(show(&b), "fn a() {\n    x| = 1;}\n");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "fn  a( ){\n x=1;}\n");
        assert!(!b.undo(), "one undo step");

        // A cursor inside a replaced range goes to its start; an insert right
        // at the cursor goes after it.
        let mut b = buf("ab|cd");
        b.apply_edits(&[(1..3, "X".into())]);
        assert_eq!(show(&b), "a|Xd");
        let mut b = buf("ab|cd");
        b.apply_edits(&[(2..2, "--".into())]);
        assert_eq!(show(&b), "ab|--cd");

        let mut b = buf("a|b");
        b.apply_edits(&[]);
        assert!(!b.dirty);
        assert!(!b.undo());
    }

    #[test]
    fn replace_ranges_is_one_undo_step() {
        let mut b = buf("foo |bar foo
foo");
        b.type_text("x");
        b.seal_undo_group();
        b.replace_ranges(&[
            (0..3, "quux".to_string()),
            (9..12, "q".to_string()),
            (13..16, String::new()),
        ]);
        assert_eq!(
            show(&b),
            "quux| xbar q
"
        );
        assert!(b.undo());
        assert_eq!(
            show(&b),
            "foo x|bar foo
foo"
        );
        assert!(b.undo());
        assert_eq!(
            show(&b),
            "foo |bar foo
foo"
        );
        assert!(b.redo());
        assert!(b.redo());
        assert_eq!(
            b.rope.to_string(),
            "quux xbar q
"
        );
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

    fn breakpoints(b: &Buffer) -> Vec<usize> {
        b.breakpoints.iter().copied().collect()
    }

    #[test]
    fn breakpoints_move_with_lines_inserted_above_them() {
        let mut b = buf("a|\nb\nc\n");
        b.toggle_breakpoint(0);
        b.toggle_breakpoint(2);
        b.insert("\nnew\n");
        // Line 0 keeps its breakpoint: the text went in after its start.
        assert_eq!(breakpoints(&b), [0, 4]);
        assert_eq!(b.rope.line(4).to_string(), "c\n");
    }

    #[test]
    fn enter_at_a_line_start_pushes_its_breakpoint_down() {
        let mut b = buf("a\n|b\n");
        b.toggle_breakpoint(1);
        b.newline();
        assert_eq!(breakpoints(&b), [2]);
        assert_eq!(b.rope.line(2).to_string(), "b\n");
    }

    #[test]
    fn breakpoints_move_up_with_lines_removed_above_them() {
        let mut b = buf("|a\nb\nc\nd\n");
        b.toggle_breakpoint(3);
        b.apply(Change {
            at: 0,
            removed: "a\nb\n".into(),
            inserted: String::new(),
        });
        assert_eq!(breakpoints(&b), [1]);
        assert_eq!(b.rope.line(1).to_string(), "d\n");
    }

    #[test]
    fn joining_a_line_onto_the_one_above_takes_its_breakpoint_along() {
        let mut b = buf("a\n|b\nc\n");
        b.toggle_breakpoint(1);
        b.toggle_breakpoint(2);
        b.backspace();
        assert_eq!(breakpoints(&b), [0, 1]);
    }

    #[test]
    fn undo_moves_breakpoints_back() {
        let mut b = buf("|a\nb\n");
        b.toggle_breakpoint(1);
        b.insert("x\ny\n");
        assert_eq!(breakpoints(&b), [3]);
        b.history.seal();
        assert!(b.undo());
        assert_eq!(breakpoints(&b), [1]);
    }

    #[test]
    fn toggling_twice_clears_and_past_the_end_does_nothing() {
        let mut b = buf("|a\nb");
        b.toggle_breakpoint(1);
        assert_eq!(breakpoints(&b), [1]);
        b.toggle_breakpoint(1);
        b.toggle_breakpoint(5);
        assert!(b.breakpoints.is_empty());
    }
}
