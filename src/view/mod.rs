use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ropey::RopeSlice;
use unicode_width::UnicodeWidthChar;

use crate::buffer::Buffer;
use crate::buffer::movement::display_col;

/// Which part of a buffer the editor pane shows.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct View {
    /// First buffer line shown.
    pub scroll_row: usize,
    /// First display column shown.
    pub scroll_col: usize,
}

impl View {
    /// Scrolls just enough that `buf`'s cursor is inside `area`. Called from the
    /// event handler after the cursor moves, so rendering never has to.
    pub fn follow(&mut self, buf: &Buffer, area: Rect, tab_width: usize) {
        let (line, col) = buf.cursor_line_col();
        let height = usize::from(area.height).max(1);
        if line < self.scroll_row {
            self.scroll_row = line;
        } else if line >= self.scroll_row + height {
            self.scroll_row = line + 1 - height;
        }

        let width = text_width(buf, area).max(1);
        let x = display_col(buf.rope.line(line), col, tab_width);
        if x < self.scroll_col {
            self.scroll_col = x;
        } else if x >= self.scroll_col + width {
            self.scroll_col = x + 1 - width;
        }
    }
}

/// Between the line number and the text.
const GUTTER_SEPARATOR: &str = " │ ";

/// Cells left of the text: one of padding, the widest line number, the separator.
fn gutter_width(buf: &Buffer) -> usize {
    let digits = buf.rope.len_lines().to_string().len();
    1 + digits + GUTTER_SEPARATOR.chars().count()
}

/// Cells available for text in `area`.
fn text_width(buf: &Buffer, area: Rect) -> usize {
    usize::from(area.width).saturating_sub(gutter_width(buf))
}

/// Draws `buf` into `area`: a right-aligned line-number gutter, then each line cut
/// at the right edge (Griffin never wraps), and puts the terminal cursor on the
/// buffer cursor when it's in view. Pure: reads its inputs only.
pub fn render_buffer(buf: &Buffer, view: &View, tab_width: usize, area: Rect, frame: &mut Frame) {
    let digits = buf.rope.len_lines().to_string().len();
    let gutter_width = gutter_width(buf);
    let text_width = text_width(buf, area);
    let gutter_style = Style::new().dim();

    let out = frame.buffer_mut();
    for (screen_row, line_idx) in (view.scroll_row..buf.rope.len_lines())
        .take(usize::from(area.height))
        .enumerate()
    {
        // `take(area.height)` keeps the row inside a u16.
        let y = area.y + screen_row as u16;
        let gutter = format!(" {:>digits$}{GUTTER_SEPARATOR}", line_idx + 1);
        out.set_stringn(area.x, y, &gutter, usize::from(area.width), gutter_style);
        if text_width == 0 {
            continue;
        }
        let text = visible_text(
            buf.rope.line(line_idx),
            tab_width,
            view.scroll_col,
            text_width,
        );
        // `gutter_width` is at most `area.width` here, so it fits in a u16.
        let x = area.x + gutter_width as u16;
        out.set_stringn(x, y, &text, text_width, Style::new());
    }

    if let Some((x, y)) = cursor_cell(buf, view, tab_width, area) {
        frame.set_cursor_position((x, y));
    }
}

/// The screen cell of `buf`'s cursor, or `None` when it's scrolled out of `area`.
fn cursor_cell(buf: &Buffer, view: &View, tab_width: usize, area: Rect) -> Option<(u16, u16)> {
    let (line, col) = buf.cursor_line_col();
    let row = line.checked_sub(view.scroll_row)?;
    let x = display_col(buf.rope.line(line), col, tab_width).checked_sub(view.scroll_col)?;
    if row >= usize::from(area.height) || x >= text_width(buf, area) {
        return None;
    }
    // Both are below the area's u16 width and height, checked just above.
    let x = area.x + (gutter_width(buf) + x) as u16;
    Some((x, area.y + row as u16))
}

/// The part of `line` that falls in display columns `scroll_col..scroll_col+width`,
/// as a string whose display width is at most `width`. Tabs become spaces up to the
/// next multiple of `tab_width`; a wide character cut by either edge becomes spaces
/// so later text keeps its column; control characters show as `?`.
pub fn visible_text(line: RopeSlice, tab_width: usize, scroll_col: usize, width: usize) -> String {
    let tab_width = tab_width.max(1);
    let end = scroll_col + width;
    let mut out = String::new();
    let mut col = 0;
    let pad = |out: &mut String, from: usize, to: usize| {
        let cells = to.min(end).saturating_sub(from.max(scroll_col));
        out.extend(std::iter::repeat_n(' ', cells));
    };
    for ch in line.chars() {
        if ch == '\n' || col >= end {
            break;
        }
        if ch == '\t' {
            let next = (col / tab_width + 1) * tab_width;
            pad(&mut out, col, next);
            col = next;
            continue;
        }
        let ch = if ch.is_control() { '?' } else { ch };
        let w = ch.width().unwrap_or(0);
        if w == 0 {
            // Combining marks ride on the character before them.
            if col > scroll_col {
                out.push(ch);
            }
            continue;
        }
        if col >= scroll_col && col + w <= end {
            out.push(ch);
        } else {
            pad(&mut out, col, col + w);
        }
        col += w;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ropey::Rope;

    fn visible(text: &str, scroll_col: usize, width: usize) -> String {
        visible_text(Rope::from_str(text).line(0), 4, scroll_col, width)
    }

    #[test]
    fn tabs_expand_to_the_next_stop() {
        assert_eq!(visible("\tx", 0, 20), "    x");
        assert_eq!(visible("ab\tx", 0, 20), "ab  x");
        assert_eq!(visible("abcd\tx", 0, 20), "abcd    x");
        assert_eq!(
            visible_text(Rope::from_str("a\tx").line(0), 8, 0, 20),
            "a       x"
        );
    }

    #[test]
    fn line_break_is_not_drawn() {
        assert_eq!(visible("abc\n", 0, 20), "abc");
    }

    #[test]
    fn long_lines_are_cut() {
        assert_eq!(visible("abcdefghij", 0, 4), "abcd");
    }

    #[test]
    fn wide_char_cut_by_the_edge_becomes_a_space() {
        assert_eq!(visible("a日本", 0, 4), "a日 ");
        assert_eq!(visible("日本", 1, 10), " 本");
    }

    #[test]
    fn control_chars_are_visible() {
        assert_eq!(visible("a\u{1}b", 0, 10), "a?b");
    }

    fn buffer_at(text: &str, cursor: usize) -> Buffer {
        Buffer {
            rope: Rope::from_str(text),
            cursor,
            ..Buffer::empty()
        }
    }

    #[test]
    fn follow_scrolls_down_and_back_up_to_the_cursor() {
        let text: String = (1..=50).map(|n| format!("{n}\n")).collect();
        let mut buf = buffer_at(&text, 0);
        let area = Rect::new(0, 0, 40, 10);
        let mut view = View::default();
        buf.cursor = buf.rope.line_to_char(9);
        view.follow(&buf, area, 4);
        assert_eq!(view.scroll_row, 0);
        buf.cursor = buf.rope.line_to_char(10);
        view.follow(&buf, area, 4);
        assert_eq!(view.scroll_row, 1);
        buf.cursor = buf.rope.line_to_char(40);
        view.follow(&buf, area, 4);
        assert_eq!(view.scroll_row, 31);
        buf.cursor = buf.rope.line_to_char(35);
        view.follow(&buf, area, 4);
        assert_eq!(view.scroll_row, 31);
        buf.cursor = buf.rope.line_to_char(5);
        view.follow(&buf, area, 4);
        assert_eq!(view.scroll_row, 5);
    }

    #[test]
    fn follow_scrolls_sideways_in_display_columns() {
        // Gutter " 1 │ " is 5 cells, leaving 10 for text.
        let mut buf = buffer_at("\tabcdefghijklmnop", 0);
        let area = Rect::new(0, 0, 15, 5);
        let mut view = View::default();
        buf.cursor = 6; // display column 9: the last visible cell
        view.follow(&buf, area, 4);
        assert_eq!(view.scroll_col, 0);
        buf.cursor = 7; // display column 10
        view.follow(&buf, area, 4);
        assert_eq!(view.scroll_col, 1);
        buf.cursor = 0;
        view.follow(&buf, area, 4);
        assert_eq!(view.scroll_col, 0);
    }

    #[test]
    fn follow_with_an_empty_area_does_not_panic() {
        let buf = buffer_at("abc\ndef", 5);
        let mut view = View::default();
        view.follow(&buf, Rect::default(), 4);
        assert_eq!(view.scroll_row, 1);
    }

    #[test]
    fn cursor_is_drawn_on_its_cell() -> Result<()> {
        // Cursor on `x`.
        let buf = buffer_at("ab\n\t日x", 5);
        let mut terminal = Terminal::new(TestBackend::new(20, 5))?;
        terminal.draw(|frame| render_buffer(&buf, &View::default(), 4, frame.area(), frame))?;
        // Gutter is 5 cells; the tab and `日` fill 6 more.
        terminal.backend_mut().assert_cursor_position((11, 1));
        Ok(())
    }

    #[test]
    fn cursor_scrolled_out_of_view_is_not_placed() {
        let buf = buffer_at("abc\ndef", 5);
        let view = View {
            scroll_row: 0,
            scroll_col: 3,
        };
        assert_eq!(cursor_cell(&buf, &view, 4, Rect::new(0, 0, 20, 5)), None);
        let view = View {
            scroll_row: 0,
            scroll_col: 0,
        };
        assert_eq!(
            cursor_cell(&buf, &view, 4, Rect::new(0, 0, 20, 1)),
            None,
            "line 2 is below a one-row area"
        );
    }

    #[test]
    fn gutter_is_right_aligned_to_the_widest_number() -> Result<()> {
        let text: String = (1..=12).map(|n| format!("line {n}\n")).collect();
        let buf = Buffer {
            rope: Rope::from_str(text.trim_end()),
            ..Buffer::empty()
        };
        let mut terminal = Terminal::new(TestBackend::new(40, 12))?;
        terminal.draw(|frame| render_buffer(&buf, &View::default(), 4, frame.area(), frame))?;
        let screen = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..40).map(|x| screen[(x, y)].symbol()).collect() };
        assert!(row(0).starts_with("  1 │ line 1"), "{}", row(0));
        assert!(row(9).starts_with(" 10 │ line 10"), "{}", row(9));
        assert!(row(11).starts_with(" 12 │ line 12"), "{}", row(11));
        Ok(())
    }
}
