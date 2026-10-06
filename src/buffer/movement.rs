//! Cursor motions. Positions are char indices into the rope; columns that must line
//! up on screen (the goal column for Up/Down) are display columns, so a tab or a
//! wide character counts for the cells it fills.

use ropey::RopeSlice;
use unicode_width::UnicodeWidthChar;

use super::Buffer;

/// One way the cursor can move. Shared by plain movement (#5) and selection (#9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    LineStart,
    LineEnd,
    PageUp,
    PageDown,
    WordLeft,
    WordRight,
    DocStart,
    DocEnd,
}

impl Motion {
    #[cfg(test)]
    pub const ALL: &[Motion] = &[
        Motion::Left,
        Motion::Right,
        Motion::Up,
        Motion::Down,
        Motion::LineStart,
        Motion::LineEnd,
        Motion::PageUp,
        Motion::PageDown,
        Motion::WordLeft,
        Motion::WordRight,
        Motion::DocStart,
        Motion::DocEnd,
    ];
}

/// Cells `ch` fills when it starts at display column `col`. Mirrors how the view
/// draws a line: tabs reach the next stop, control characters show as one `?`.
pub fn char_width(ch: char, col: usize, tab_width: usize) -> usize {
    let tab_width = tab_width.max(1);
    if ch == '\t' {
        tab_width - col % tab_width
    } else if ch.is_control() {
        1
    } else {
        ch.width().unwrap_or(0)
    }
}

/// Display column of char `char_col` in `line`.
pub fn display_col(line: RopeSlice, char_col: usize, tab_width: usize) -> usize {
    line.chars()
        .take(char_col)
        .fold(0, |col, ch| col + char_width(ch, col, tab_width))
}

/// The char column in `line` closest to display column `goal` without passing it.
/// A goal inside a tab or wide character lands at its start, never in the middle.
pub fn char_col_at(line: RopeSlice, goal: usize, tab_width: usize) -> usize {
    let mut col = 0;
    for (i, ch) in line.chars().enumerate() {
        if ch == '\n' {
            return i;
        }
        let next = col + char_width(ch, col, tab_width);
        if next > goal {
            return i;
        }
        col = next;
    }
    line.len_chars()
}

/// Chars in `line` before its line break.
fn line_len(line: RopeSlice) -> usize {
    let len = line.len_chars();
    if len > 0 && line.char(len - 1) == '\n' {
        len - 1
    } else {
        len
    }
}

/// Word motions stop where one of these classes changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CharClass {
    Space,
    Word,
    Punct,
}

fn class(ch: char) -> CharClass {
    if ch.is_whitespace() {
        CharClass::Space
    } else if ch.is_alphanumeric() || ch == '_' {
        CharClass::Word
    } else {
        CharClass::Punct
    }
}

impl Buffer {
    /// Line index and char column of the cursor.
    pub fn cursor_line_col(&self) -> (usize, usize) {
        let line = self.rope.char_to_line(self.cursor);
        (line, self.cursor - self.rope.line_to_char(line))
    }

    /// Moves the cursor. `page_height` is how many lines PageUp/PageDown move;
    /// `tab_width` makes Up/Down keep the on-screen column across tabs.
    pub fn move_cursor(&mut self, motion: Motion, page_height: usize, tab_width: usize) {
        // Typing after a move is a new undo step even if it lands back where it was.
        self.history.seal();
        let len = self.rope.len_chars();
        let cursor = self.cursor.min(len);
        let page = isize::try_from(page_height.max(1)).unwrap_or(isize::MAX);
        // Vertical motions keep the goal column; every other motion clears it.
        match motion {
            Motion::Up => return self.move_lines(-1, tab_width),
            Motion::Down => return self.move_lines(1, tab_width),
            Motion::PageUp => return self.move_lines(-page, tab_width),
            Motion::PageDown => return self.move_lines(page, tab_width),
            Motion::Left => self.cursor = cursor.saturating_sub(1),
            Motion::Right => self.cursor = (cursor + 1).min(len),
            Motion::LineStart => {
                self.cursor = self.rope.line_to_char(self.rope.char_to_line(cursor));
            }
            Motion::LineEnd => {
                let line = self.rope.char_to_line(cursor);
                self.cursor = self.rope.line_to_char(line) + line_len(self.rope.line(line));
            }
            Motion::WordLeft => self.cursor = self.word_left(cursor),
            Motion::WordRight => self.cursor = self.word_right(cursor),
            Motion::DocStart => self.cursor = 0,
            Motion::DocEnd => self.cursor = len,
        }
        self.goal_col = None;
    }

    /// Moves `lines` up (negative) or down, aiming for the goal column. Past the
    /// first or last line the cursor goes to the start or end of the document.
    fn move_lines(&mut self, lines: isize, tab_width: usize) {
        let (line, col) = self.cursor_line_col();
        let goal = *self
            .goal_col
            .get_or_insert_with(|| display_col(self.rope.line(line), col, tab_width));
        let last = self.rope.len_lines() - 1;
        let target = line.checked_add_signed(lines);
        match target {
            Some(target) if target <= last => {
                let slice = self.rope.line(target);
                self.cursor = self.rope.line_to_char(target) + char_col_at(slice, goal, tab_width);
            }
            None => self.cursor = 0,
            Some(_) => self.cursor = self.rope.len_chars(),
        }
    }

    /// Start of the word (or punctuation run) before `pos`, skipping spaces and
    /// line breaks first.
    fn word_left(&self, mut pos: usize) -> usize {
        while pos > 0 && class(self.rope.char(pos - 1)) == CharClass::Space {
            pos -= 1;
        }
        if pos == 0 {
            return 0;
        }
        let run = class(self.rope.char(pos - 1));
        while pos > 0 && class(self.rope.char(pos - 1)) == run {
            pos -= 1;
        }
        pos
    }

    /// End of the word (or punctuation run) after `pos`, skipping spaces and line
    /// breaks first.
    fn word_right(&self, mut pos: usize) -> usize {
        let len = self.rope.len_chars();
        while pos < len && class(self.rope.char(pos)) == CharClass::Space {
            pos += 1;
        }
        if pos == len {
            return len;
        }
        let run = class(self.rope.char(pos));
        while pos < len && class(self.rope.char(pos)) == run {
            pos += 1;
        }
        pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    const TAB: usize = 4;

    fn buf(text: &str) -> Buffer {
        Buffer {
            rope: Rope::from_str(text),
            ..Buffer::empty()
        }
    }

    fn at(text: &str, cursor: usize) -> Buffer {
        Buffer {
            cursor,
            ..buf(text)
        }
    }

    fn moved(b: &mut Buffer, motion: Motion) -> (usize, usize) {
        b.move_cursor(motion, 10, TAB);
        b.cursor_line_col()
    }

    #[test]
    fn left_and_right_step_one_char_and_cross_lines() {
        let mut b = at("ab\ncd", 0);
        assert_eq!(moved(&mut b, Motion::Left), (0, 0));
        assert_eq!(moved(&mut b, Motion::Right), (0, 1));
        assert_eq!(moved(&mut b, Motion::Right), (0, 2));
        assert_eq!(moved(&mut b, Motion::Right), (1, 0));
        assert_eq!(moved(&mut b, Motion::Left), (0, 2));
        let mut b = at("ab\ncd", 5);
        assert_eq!(moved(&mut b, Motion::Right), (1, 2));
    }

    #[test]
    fn right_steps_over_a_wide_char_or_tab_whole() {
        let mut b = at("日本\tx", 0);
        assert_eq!(moved(&mut b, Motion::Right), (0, 1));
        assert_eq!(moved(&mut b, Motion::Right), (0, 2));
        assert_eq!(moved(&mut b, Motion::Right), (0, 3));
        assert_eq!(b.rope.char(b.cursor), 'x');
    }

    #[test]
    fn home_and_end() {
        let mut b = at("one\ntwo three\nx", 6);
        assert_eq!(moved(&mut b, Motion::LineEnd), (1, 9));
        assert_eq!(moved(&mut b, Motion::LineStart), (1, 0));
        let mut b = at("one\nlast", 5);
        assert_eq!(moved(&mut b, Motion::LineEnd), (1, 4));
    }

    #[test]
    fn up_and_down_keep_the_goal_column_across_shorter_lines() {
        let mut b = at("abcdefgh\nab\n\nabcdefgh", 6);
        assert_eq!(moved(&mut b, Motion::Down), (1, 2));
        assert_eq!(moved(&mut b, Motion::Down), (2, 0));
        assert_eq!(moved(&mut b, Motion::Down), (3, 6));
        assert_eq!(moved(&mut b, Motion::Up), (2, 0));
        assert_eq!(moved(&mut b, Motion::Up), (1, 2));
        assert_eq!(moved(&mut b, Motion::Up), (0, 6));
    }

    #[test]
    fn horizontal_movement_resets_the_goal_column() {
        let mut b = at("abcdefgh\nab\nabcdefgh", 6);
        moved(&mut b, Motion::Down);
        moved(&mut b, Motion::Left);
        assert_eq!(moved(&mut b, Motion::Down), (2, 1));
    }

    #[test]
    fn up_on_the_first_line_and_down_on_the_last_go_to_the_ends() {
        let mut b = at("abc\ndef", 1);
        assert_eq!(moved(&mut b, Motion::Up), (0, 0));
        let mut b = at("abc\ndef", 5);
        assert_eq!(moved(&mut b, Motion::Down), (1, 3));
    }

    #[test]
    fn vertical_movement_over_tabs_lands_on_char_boundaries() {
        // Column 2 on line 0 falls inside the tab on line 1: land before the tab.
        let mut b = at("abcdefgh\n\tx\nabcdefgh", 2);
        assert_eq!(moved(&mut b, Motion::Down), (1, 0));
        // The goal column survives: line 2 gets column 2 again.
        assert_eq!(moved(&mut b, Motion::Down), (2, 2));
        // Column 5 on line 2 is just after the tab's 4 cells, so on `x`'s end.
        let mut b = at("abcdefgh\n\tx\nabcdefgh", 5);
        assert_eq!(moved(&mut b, Motion::Down), (1, 2));
        let mut b = at("abcdefgh\n\tx\nabcdefgh", 4);
        assert_eq!(moved(&mut b, Motion::Down), (1, 1));
    }

    #[test]
    fn vertical_movement_over_wide_chars_lands_on_char_boundaries() {
        // `日本語` fills columns 0-5; column 3 is the right half of `本`.
        let mut b = at("abcdef\n日本語\nabcdef", 3);
        assert_eq!(moved(&mut b, Motion::Down), (1, 1));
        assert_eq!(moved(&mut b, Motion::Down), (2, 3));
        let mut b = at("abcdef\n日本語\nabcdef", 4);
        assert_eq!(moved(&mut b, Motion::Down), (1, 2));
        // Going up from the wide line keeps its display column, not its char column.
        let mut b = at("abcdef\n日本語\nabcdef", 9);
        assert_eq!(moved(&mut b, Motion::Up), (0, 4));
    }

    #[test]
    fn page_up_and_down_move_by_the_page_height() {
        let text: String = (0..30).map(|n| format!("line {n}\n")).collect();
        let mut b = at(&text, 2);
        assert_eq!(moved(&mut b, Motion::PageDown), (10, 2));
        assert_eq!(moved(&mut b, Motion::PageDown), (20, 2));
        assert_eq!(moved(&mut b, Motion::PageDown), (30, 0));
        assert_eq!(moved(&mut b, Motion::PageUp), (20, 2));
        assert_eq!(moved(&mut b, Motion::PageUp), (10, 2));
        assert_eq!(moved(&mut b, Motion::PageUp), (0, 2));
        assert_eq!(moved(&mut b, Motion::PageUp), (0, 0));
    }

    #[test]
    fn page_height_zero_still_moves() {
        let mut b = at("a\nb\nc", 0);
        b.move_cursor(Motion::PageDown, 0, TAB);
        assert_eq!(b.cursor_line_col(), (1, 0));
    }

    #[test]
    fn word_right_stops_at_word_ends() {
        let text = "let foo_bar = baz(1);\n  next";
        let mut b = at(text, 0);
        let mut stops = Vec::new();
        for _ in 0..9 {
            b.move_cursor(Motion::WordRight, 10, TAB);
            stops.push(b.cursor);
        }
        // let | foo_bar | = | baz | ( | 1 | ); | next | end
        assert_eq!(stops, [3, 11, 13, 17, 18, 19, 21, 28, 28]);
    }

    #[test]
    fn word_left_stops_at_word_starts() {
        let text = "let foo_bar = baz(1);\n  next";
        let mut b = at(text, text.chars().count());
        let mut stops = Vec::new();
        for _ in 0..9 {
            b.move_cursor(Motion::WordLeft, 10, TAB);
            stops.push(b.cursor);
        }
        assert_eq!(stops, [24, 19, 18, 17, 14, 12, 4, 0, 0]);
    }

    #[test]
    fn words_include_non_ascii_letters() {
        let mut b = at("héllo wörld", 0);
        b.move_cursor(Motion::WordRight, 10, TAB);
        assert_eq!(b.cursor, 5);
        b.move_cursor(Motion::WordRight, 10, TAB);
        assert_eq!(b.cursor, 11);
    }

    #[test]
    fn doc_start_and_end() {
        let mut b = at("one\ntwo\nthree", 5);
        assert_eq!(moved(&mut b, Motion::DocEnd), (2, 5));
        assert_eq!(moved(&mut b, Motion::DocStart), (0, 0));
    }

    #[test]
    fn empty_buffer_never_moves_or_panics() {
        for &motion in Motion::ALL {
            let mut b = buf("");
            assert_eq!(moved(&mut b, motion), (0, 0), "{motion:?}");
        }
    }

    #[test]
    fn display_col_counts_tabs_and_wide_chars() {
        let rope = Rope::from_str("a\t日x\u{1}y");
        let line = rope.line(0);
        let cols: Vec<usize> = (0..=6).map(|c| display_col(line, c, TAB)).collect();
        assert_eq!(cols, [0, 1, 4, 6, 7, 8, 9]);
    }
}
