use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ropey::RopeSlice;
use unicode_width::UnicodeWidthChar;

use crate::buffer::Buffer;

/// Which part of a buffer the editor pane shows. Cursor-following scrolling
/// arrives in #5; until then both stay at 0.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct View {
    /// First buffer line shown.
    pub scroll_row: usize,
    /// First display column shown.
    pub scroll_col: usize,
}

/// Between the line number and the text.
const GUTTER_SEPARATOR: &str = " │ ";

/// Draws `buf` into `area`: a right-aligned line-number gutter, then each line cut
/// at the right edge (Griffin never wraps). Pure: reads its inputs only.
pub fn render_buffer(buf: &Buffer, view: &View, tab_width: usize, area: Rect, frame: &mut Frame) {
    let digits = buf.rope.len_lines().to_string().len();
    // One cell of padding on the left, then the number, then the separator.
    let gutter_width = 1 + digits + GUTTER_SEPARATOR.chars().count();
    let text_width = usize::from(area.width).saturating_sub(gutter_width);
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
