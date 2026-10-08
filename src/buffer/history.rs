//! Undo and redo. `Buffer::apply` records every `Change` here; undo applies the
//! inverses without recording them, so they never become history of their own.

use super::Buffer;
use super::edit::Change;

/// How an edit joins the open group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// A typed character: joins an open run of typing at the adjacent position.
    Typing,
    /// Enter: ends an open run of typing as part of it, so a typed line and its
    /// line break undo together.
    Newline,
    /// Anything else: a group of its own.
    Other,
}

/// What one undo step reverts: changes in the order they were applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    changes: Vec<Change>,
    /// Where the cursor was before the first change, so undo puts it back there.
    cursor_before: usize,
    /// The selection's anchor then, for the steps that bring it back on undo.
    anchor_before: Option<usize>,
}

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Group>,
    redo: Vec<Group>,
    /// The last undo group is a run of typing that the next typed char may join.
    typing_open: bool,
    /// Depth of `begin_group` calls; while above zero every change joins one group.
    explicit_depth: usize,
    /// Set by the outermost `begin_group`: the next change starts a fresh group.
    explicit_fresh: bool,
    /// Closers typing stepped over since the last typed change; the run goes on
    /// past them.
    stepped: usize,
}

impl Change {
    /// The change that undoes this one.
    pub fn inverse(&self) -> Change {
        Change {
            at: self.at,
            removed: self.inserted.clone(),
            inserted: self.removed.clone(),
        }
    }

    /// Whether `next` is typing that continues straight on from this change, after
    /// `stepped` closers typed over in between. Typing inside the change counts
    /// too: an auto-paired `()` leaves the cursor between its brackets, and what is
    /// typed there belongs to the same run.
    fn continued_by(&self, next: &Change, stepped: usize) -> bool {
        let len = self.inserted.chars().count();
        self.removed.is_empty()
            && next.removed.is_empty()
            && next.at > self.at
            && next.at <= self.at + len + stepped
    }
}

impl History {
    /// Records a change applied with the cursor at `cursor_before`. Any new edit
    /// makes the redo stack meaningless, so it is cleared.
    pub fn record(&mut self, change: Change, kind: EditKind, cursor_before: usize) {
        self.redo.clear();
        if self.explicit_depth > 0 {
            match self.undo.last_mut() {
                Some(group) if !self.explicit_fresh => group.changes.push(change),
                _ => self.push_group(change, cursor_before),
            }
            self.explicit_fresh = false;
            return;
        }
        let joins = kind != EditKind::Other
            && self.typing_open
            && self
                .undo
                .last()
                .and_then(|group| group.changes.last())
                .is_some_and(|last| last.continued_by(&change, self.stepped));
        if joins {
            // `typing_open` means the last group exists.
            if let Some(group) = self.undo.last_mut() {
                group.changes.push(change);
            }
        } else {
            self.push_group(change, cursor_before);
        }
        self.typing_open = kind == EditKind::Typing;
        self.stepped = 0;
    }

    /// Typing stepped over a closer: the run of typing stays open across it.
    pub fn step_over(&mut self) {
        if self.typing_open {
            self.stepped += 1;
        }
    }

    fn push_group(&mut self, change: Change, cursor_before: usize) {
        self.undo.push(Group {
            changes: vec![change],
            cursor_before,
            anchor_before: None,
        });
    }

    /// Has undoing the last group select from `anchor` to its cursor again, as
    /// wrapping a selection does: it changed the selection's text, not just its place.
    pub fn select_on_undo(&mut self, anchor: usize) {
        if let Some(group) = self.undo.last_mut() {
            group.anchor_before = Some(anchor);
        }
    }

    /// Ends any open run of typing, so the next edit starts its own group.
    pub fn seal(&mut self) {
        self.typing_open = false;
        self.stepped = 0;
    }

    /// Lets the next typed char join the last group as if it were a run of typing;
    /// typing over a selection keeps going in the group that replaced it.
    pub fn continue_typing(&mut self) {
        self.typing_open = !self.undo.is_empty();
    }

    #[cfg(test)]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    #[cfg(test)]
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

impl Buffer {
    /// Reverts the last group and puts the cursor where it was before that group.
    /// Returns whether there was anything to undo.
    pub fn undo(&mut self) -> bool {
        self.history.seal();
        let Some(group) = self.history.undo.pop() else {
            return false;
        };
        for change in group.changes.iter().rev() {
            self.apply_unrecorded(&change.inverse());
        }
        self.cursor = group.cursor_before;
        self.anchor = group.anchor_before;
        self.history.redo.push(group);
        true
    }

    /// Reapplies the last undone group; the cursor ends after its last change.
    /// Returns whether there was anything to redo.
    pub fn redo(&mut self) -> bool {
        self.history.seal();
        let Some(group) = self.history.redo.pop() else {
            return false;
        };
        for change in &group.changes {
            self.apply_unrecorded(change);
        }
        self.history.undo.push(group);
        true
    }

    /// Every change from here to the matching `end_group` undoes as one step (paste,
    /// replace selection). Calls nest; only the outermost pair makes the group.
    pub fn begin_group(&mut self) {
        if self.history.explicit_depth == 0 {
            self.history.seal();
            self.history.explicit_fresh = true;
        }
        self.history.explicit_depth += 1;
    }

    pub fn end_group(&mut self) {
        self.history.explicit_depth = self.history.explicit_depth.saturating_sub(1);
        if self.history.explicit_depth == 0 {
            self.history.explicit_fresh = false;
            self.history.seal();
        }
    }

    /// Closes any open run of typing; the app calls this for every non-typing action.
    pub fn seal_undo_group(&mut self) {
        self.history.seal();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::movement::Motion;
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

    fn type_str(b: &mut Buffer, text: &str) {
        for ch in text.chars() {
            b.type_text(ch.encode_utf8(&mut [0; 4]));
        }
    }

    fn step(b: &mut Buffer, motion: Motion) {
        b.move_cursor(motion, 10, 4);
    }

    #[test]
    fn undo_reverts_and_redo_reapplies_every_kind_of_change() {
        let mut b = buf("ab|cd");
        b.apply(Change {
            at: 1,
            removed: "bc".into(),
            inserted: "XYZ".into(),
        });
        assert_eq!(show(&b), "aXYZ|d");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "abcd");
        assert!(b.redo());
        assert_eq!(b.rope.to_string(), "aXYZd");
        assert!(!b.redo(), "nothing left to redo");
        assert!(b.undo());
        assert!(!b.undo(), "nothing left to undo");
        assert_eq!(b.rope.to_string(), "abcd");
    }

    #[test]
    fn undo_on_an_untouched_buffer_does_nothing() {
        let mut b = buf("ab|");
        assert!(!b.undo());
        assert!(!b.redo());
        assert_eq!(show(&b), "ab|");
        assert!(!b.dirty);
    }

    #[test]
    fn each_non_typing_edit_is_its_own_step() {
        let mut b = buf("abc|");
        b.backspace();
        b.backspace();
        b.delete();
        b.tab(4, true);
        assert_eq!(b.rope.to_string(), "a   ");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "a");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "ab");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "abc");
        // The delete at the document end changed nothing, so it isn't a step.
        assert!(!b.undo());
    }

    #[test]
    fn a_new_edit_after_undo_clears_redo() {
        let mut b = buf("|");
        type_str(&mut b, "ab");
        b.newline();
        type_str(&mut b, "cd");
        assert!(b.undo());
        assert!(b.history.can_redo());
        type_str(&mut b, "x");
        assert!(!b.history.can_redo());
        assert!(!b.redo());
        assert_eq!(b.rope.to_string(), "ab\nx");
    }

    #[test]
    fn typing_a_run_undoes_as_one_step() {
        let mut b = buf("|");
        type_str(&mut b, "hello world");
        assert!(b.undo());
        assert_eq!(show(&b), "|");
        assert!(!b.history.can_undo());
        assert!(b.redo());
        assert_eq!(show(&b), "hello world|");
    }

    #[test]
    fn enter_ends_the_typing_run_it_follows() {
        let mut b = buf("|");
        type_str(&mut b, "abc");
        b.newline();
        type_str(&mut b, "def");
        assert!(b.undo());
        assert_eq!(show(&b), "abc\n|");
        assert!(b.undo());
        assert_eq!(show(&b), "|");
        assert!(b.redo());
        assert!(b.redo());
        assert_eq!(show(&b), "abc\ndef|");
    }

    #[test]
    fn enter_without_typing_before_it_is_its_own_step() {
        let mut b = buf("ab|");
        b.newline();
        b.newline();
        assert!(b.undo());
        assert_eq!(show(&b), "ab\n|");
    }

    #[test]
    fn a_cursor_move_closes_the_run() {
        let mut b = buf("|");
        type_str(&mut b, "ab");
        // Away and back: the next char is adjacent again but still a new step.
        step(&mut b, Motion::Left);
        step(&mut b, Motion::Right);
        type_str(&mut b, "cd");
        assert!(b.undo());
        assert_eq!(show(&b), "ab|");
        assert!(b.undo());
        assert_eq!(show(&b), "|");
    }

    #[test]
    fn backspace_after_typing_closes_the_run() {
        let mut b = buf("|");
        type_str(&mut b, "abc");
        b.backspace();
        type_str(&mut b, "d");
        assert_eq!(b.rope.to_string(), "abd");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "ab");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "abc");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "");
    }

    #[test]
    fn any_non_typing_action_closes_the_run() {
        let mut b = buf("|");
        type_str(&mut b, "ab");
        b.seal_undo_group();
        type_str(&mut b, "cd");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "ab");

        let mut b = buf("|");
        type_str(&mut b, "ab");
        b.tab(4, false);
        type_str(&mut b, "cd");
        assert!(b.undo());
        assert_eq!(b.rope.to_string(), "ab\t");
    }

    #[test]
    fn undo_puts_the_cursor_where_the_change_happened() {
        let mut b = buf("hello |world");
        type_str(&mut b, "big ");
        step(&mut b, Motion::DocEnd);
        assert!(b.undo());
        assert_eq!(show(&b), "hello |world");
        step(&mut b, Motion::DocStart);
        assert!(b.redo());
        assert_eq!(show(&b), "hello big |world");

        // Undoing a deletion leaves the cursor where it was before deleting.
        let mut b = buf("ab|cd");
        b.backspace();
        step(&mut b, Motion::DocStart);
        assert!(b.undo());
        assert_eq!(show(&b), "ab|cd");
        let mut b = buf("ab|cd");
        b.delete();
        step(&mut b, Motion::DocEnd);
        assert!(b.undo());
        assert_eq!(show(&b), "ab|cd");
        assert!(b.redo());
        assert_eq!(show(&b), "ab|d");
    }

    #[test]
    fn undo_and_redo_set_dirty() {
        let mut b = buf("|");
        type_str(&mut b, "x");
        b.dirty = false;
        assert!(b.undo());
        assert!(b.dirty);
        b.dirty = false;
        assert!(b.redo());
        assert!(b.dirty);
    }

    #[test]
    fn begin_and_end_group_make_one_step() {
        let mut b = buf("|");
        type_str(&mut b, "a");
        b.begin_group();
        b.insert("bc");
        b.begin_group();
        b.newline();
        b.end_group();
        b.backspace();
        b.end_group();
        type_str(&mut b, "d");
        assert_eq!(b.rope.to_string(), "abcd");
        assert!(b.undo());
        assert_eq!(show(&b), "abc|");
        assert!(b.undo());
        assert_eq!(show(&b), "a|");
        assert!(b.undo());
        assert_eq!(show(&b), "|");
    }

    #[test]
    fn an_empty_explicit_group_is_not_a_step() {
        let mut b = buf("|");
        type_str(&mut b, "a");
        b.begin_group();
        b.end_group();
        assert!(b.undo());
        assert!(!b.undo());
    }
}
