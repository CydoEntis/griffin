use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ropey::RopeSlice;
use unicode_width::UnicodeWidthChar;

use crate::buffer::Buffer;
use crate::buffer::movement::{char_col_at, char_width, display_col};
use crate::highlight::Role;
use crate::lsp::{Diagnostic, Severity};
use crate::theme::Theme;

/// Ranges drawn over a buffer's text, by char index.
#[derive(Debug, Default, Clone, Copy)]
pub struct Marks<'a> {
    /// Find matches, sorted by start; drawn in the selection colours.
    pub highlights: &'a [Range<usize>],
    /// The language server's diagnostics, sorted by start; underlined in their
    /// severity's colour, with a mark in the gutter.
    pub diagnostics: &'a [Diagnostic],
}

/// The colour a diagnostic of `severity` is drawn in: errors in `err`, warnings
/// in Hydra's `working`, the milder kinds muted.
fn severity_color(theme: &Theme, severity: Severity) -> Color {
    match severity {
        Severity::Error => theme.err,
        Severity::Warning => theme.warning,
        Severity::Information | Severity::Hint => theme.muted,
    }
}

/// In the gutter's first cell, on a line with a diagnostic.
const GUTTER_MARK: &str = "●";

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

    /// The char index under screen cell (`col`, `row`) of `area`: the inverse of the
    /// column maths `render_buffer` draws with. A cell in the gutter maps to the
    /// line's start, one past a line's end to that end, one below the last line to
    /// the last line, and one inside a tab or wide character to that character.
    pub fn screen_to_char(
        &self,
        buf: &Buffer,
        area: Rect,
        col: u16,
        row: u16,
        tab_width: usize,
    ) -> usize {
        let last = buf.rope.len_lines() - 1;
        let line = (self.scroll_row + usize::from(row.saturating_sub(area.y))).min(last);
        let x = usize::from(col.saturating_sub(area.x)).saturating_sub(gutter_width(buf));
        let slice = buf.rope.line(line);
        buf.rope.line_to_char(line) + char_col_at(slice, self.scroll_col + x, tab_width)
    }

    /// Scrolls by `lines` (negative is up) without touching the cursor, stopping
    /// when the first or last line reaches the edge of `area`.
    pub fn scroll_by(&mut self, buf: &Buffer, area: Rect, lines: isize) {
        let height = usize::from(area.height).max(1);
        let max = buf.rope.len_lines().saturating_sub(height);
        let target = self.scroll_row.saturating_add_signed(lines);
        // A view already past `max` (a pane that just grew) may still scroll up.
        self.scroll_row = if lines >= 0 {
            target.min(max.max(self.scroll_row))
        } else {
            target
        };
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
/// buffer cursor when it's in view. Diagnostics are underlined in their colour and
/// mark the gutter with the most severe one on the line; the selection and every
/// highlight get the selection colours on top. Pure: reads its inputs only.
pub fn render_buffer(
    theme: &Theme,
    buf: &Buffer,
    view: &View,
    tab_width: usize,
    marks: Marks,
    area: Rect,
    frame: &mut Frame,
) {
    let Marks {
        highlights,
        diagnostics,
    } = marks;
    let digits = buf.rope.len_lines().to_string().len();
    let gutter_width = gutter_width(buf);
    let text_width = text_width(buf, area);
    let gutter_style = Style::new().fg(theme.muted);
    let selection = buf.selection();
    let syntax = visible_spans(buf, view, area);

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
        for (cells, role) in role_cells(buf, line_idx, &syntax, tab_width) {
            let from = cells.start.max(view.scroll_col);
            let to = cells.end.min(view.scroll_col + text_width);
            for col in from..to {
                // Below `text_width`, which fits in the area's u16 width.
                let cell_x = x + (col - view.scroll_col) as u16;
                if let Some(cell) = out.cell_mut((cell_x, y)) {
                    cell.set_style(theme.syntax.style(role));
                }
            }
        }
        let line_start = buf.rope.line_to_char(line_idx);
        let line_end = line_start + buf.rope.line(line_idx).len_chars();
        let mut worst: Option<Severity> = None;
        for diagnostic in diagnostics.iter().take_while(|d| d.range.start < line_end) {
            let range = &diagnostic.range;
            // An empty range (a missing `;`, say) still gets one cell to show it.
            let range = if range.is_empty() {
                range.start..range.start + 1
            } else {
                range.clone()
            };
            if range.end <= line_start {
                continue;
            }
            let severity = diagnostic.severity;
            worst = Some(worst.map_or(severity, |w| w.min(severity)));
            let style = Style::new()
                .fg(severity_color(theme, diagnostic.severity))
                .add_modifier(Modifier::UNDERLINED);
            let cells = selected_cells(buf, line_idx, &range, tab_width);
            let from = cells.start.max(view.scroll_col);
            let to = cells.end.min(view.scroll_col + text_width);
            for col in from..to {
                // Below `text_width`, which fits in the area's u16 width.
                let cell_x = x + (col - view.scroll_col) as u16;
                if let Some(cell) = out.cell_mut((cell_x, y)) {
                    cell.set_style(style);
                }
            }
        }
        if let Some(severity) = worst {
            out.set_stringn(
                area.x,
                y,
                GUTTER_MARK,
                usize::from(area.width),
                Style::new().fg(severity_color(theme, severity)),
            );
        }
        // Only the highlights that touch this line, found by bisecting the sorted list.
        let first = highlights.partition_point(|h| h.end <= line_start);
        let on_line = highlights[first..]
            .iter()
            .take_while(|h| h.start < line_end);
        for range in selection.iter().chain(on_line) {
            let cells = selected_cells(buf, line_idx, range, tab_width);
            let from = cells.start.max(view.scroll_col);
            let to = cells.end.min(view.scroll_col + text_width);
            for col in from..to {
                // Below `text_width`, which fits in the area's u16 width.
                let cell_x = x + (col - view.scroll_col) as u16;
                if let Some(cell) = out.cell_mut((cell_x, y)) {
                    cell.set_style(Theme::highlight(theme.selection_bg, theme.fg));
                }
            }
        }
    }

    if let Some((x, y)) = cursor_cell(buf, view, tab_width, area) {
        frame.set_cursor_position((x, y));
    }
}

/// The syntax roles in the lines `view` shows of `buf` in `area`, as sorted byte
/// ranges. Empty when the buffer isn't highlighted.
fn visible_spans(buf: &Buffer, view: &View, area: Rect) -> Vec<(Range<usize>, Role)> {
    let Some(highlighter) = &buf.highlighter else {
        return Vec::new();
    };
    let lines = buf.rope.len_lines();
    let first = view.scroll_row.min(lines);
    let last = (view.scroll_row + usize::from(area.height)).min(lines);
    let bytes = buf.rope.line_to_byte(first)..buf.rope.line_to_byte(last);
    highlighter.spans(&buf.rope, bytes)
}

/// Display columns of line `line_idx` drawn in each role of `spans` (sorted byte
/// ranges), merged where neighbouring characters share a role.
fn role_cells(
    buf: &Buffer,
    line_idx: usize,
    spans: &[(Range<usize>, Role)],
    tab_width: usize,
) -> Vec<(Range<usize>, Role)> {
    let mut byte = buf.rope.line_to_byte(line_idx);
    let mut next = spans.partition_point(|(range, _)| range.end <= byte);
    let mut out: Vec<(Range<usize>, Role)> = Vec::new();
    let mut col = 0;
    for ch in buf.rope.line(line_idx).chars() {
        if ch == '\n' || next >= spans.len() {
            break;
        }
        let w = char_width(ch, col, tab_width);
        while next < spans.len() && spans[next].0.end <= byte {
            next += 1;
        }
        if let Some((range, role)) = spans.get(next)
            && range.contains(&byte)
        {
            match out.last_mut() {
                Some((cells, prev)) if cells.end == col && prev == role => cells.end = col + w,
                _ => out.push((col..col + w, *role)),
            }
        }
        byte += ch.len_utf8();
        col += w;
    }
    out
}

/// Display columns of line `line_idx` covered by `selection`. A selected line break
/// counts as one cell past the line's end, so a selection spanning lines shows
/// where each line is included.
fn selected_cells(
    buf: &Buffer,
    line_idx: usize,
    selection: &Range<usize>,
    tab_width: usize,
) -> Range<usize> {
    let start = buf.rope.line_to_char(line_idx);
    let (mut first, mut last) = (None, 0);
    let mut col = 0;
    for (i, ch) in buf.rope.line(line_idx).chars().enumerate() {
        let w = if ch == '\n' {
            1
        } else {
            char_width(ch, col, tab_width)
        };
        if selection.contains(&(start + i)) {
            first.get_or_insert(col);
            last = col + w;
        }
        col += w;
    }
    first.map_or(0..0, |first| first..last)
}

/// The screen cell of `buf`'s cursor, or `None` when it's scrolled out of `area`.
pub fn cursor_cell(buf: &Buffer, view: &View, tab_width: usize, area: Rect) -> Option<(u16, u16)> {
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
        terminal.draw(|frame| {
            render_buffer(
                &Theme::default(),
                &buf,
                &View::default(),
                4,
                Marks::default(),
                frame.area(),
                frame,
            )
        })?;
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
    fn selection_is_drawn_in_reverse_video() -> Result<()> {
        // `ello` on line 1 plus its line break, then `w` on line 2.
        let buf = Buffer {
            rope: Rope::from_str("hello\nworld"),
            anchor: Some(1),
            cursor: 7,
            ..Buffer::empty()
        };
        let mut terminal = Terminal::new(TestBackend::new(20, 3))?;
        terminal.draw(|frame| {
            render_buffer(
                &Theme::default(),
                &buf,
                &View::default(),
                4,
                Marks::default(),
                frame.area(),
                frame,
            )
        })?;
        let screen = terminal.backend().buffer();
        let reversed = |y: u16| -> String {
            (5..20)
                .map(|x| {
                    let cell = &screen[(x, y)];
                    if cell.modifier.contains(ratatui::style::Modifier::REVERSED) {
                        '#'
                    } else {
                        '.'
                    }
                })
                .collect()
        };
        // Gutter is 5 cells.
        assert_eq!(reversed(0), ".#####.........");
        assert_eq!(reversed(1), "#..............");
        assert_eq!(reversed(2), "...............");
        Ok(())
    }

    #[test]
    fn screen_to_char_inverts_the_render_columns() {
        // Gutter " 1 │ " is 5 cells.
        let buf = buffer_at("ab\n\t日x\nlast", 0);
        let view = View::default();
        let area = Rect::new(0, 0, 20, 10);
        let at = |col, row| view.screen_to_char(&buf, area, col, row, 4);
        assert_eq!(at(5, 0), 0);
        assert_eq!(at(6, 0), 1);
        // Past the end of `ab`.
        assert_eq!(at(15, 0), 2);
        // In the gutter: the line's start.
        assert_eq!(at(1, 1), 3);
        // Anywhere on the tab is the tab; both cells of `日` are `日`.
        assert_eq!(at(8, 1), 3);
        assert_eq!(at(9, 1), 4);
        assert_eq!(at(10, 1), 4);
        assert_eq!(at(11, 1), 5);
        // Below the last line: the last line, at that column.
        assert_eq!(at(6, 9), 8);
    }

    #[test]
    fn screen_to_char_accounts_for_scrolling() {
        let text: String = (0..50).map(|n| format!("{n:02}abcdef\n")).collect();
        let buf = buffer_at(&text, 0);
        let view = View {
            scroll_row: 10,
            scroll_col: 2,
        };
        // An area 3 rows down; gutter " 51 │ " is 6 cells.
        let area = Rect::new(0, 3, 30, 5);
        let pos = view.screen_to_char(&buf, area, 6, 4, 4);
        assert_eq!(pos, buf.rope.line_to_char(11) + 2);
    }

    #[test]
    fn scroll_by_stops_at_both_ends() {
        let text: String = (0..20).map(|n| format!("{n}\n")).collect();
        let buf = buffer_at(&text, 0);
        let area = Rect::new(0, 0, 20, 10);
        let mut view = View::default();
        view.scroll_by(&buf, area, -3);
        assert_eq!(view.scroll_row, 0);
        view.scroll_by(&buf, area, 3);
        assert_eq!(view.scroll_row, 3);
        for _ in 0..5 {
            view.scroll_by(&buf, area, 3);
        }
        // 21 lines (the last one empty) in a 10-row area.
        assert_eq!(view.scroll_row, 11);
        view.scroll_by(&buf, area, -3);
        assert_eq!(view.scroll_row, 8);
    }

    #[test]
    fn highlights_get_the_selection_colours_on_their_own_cells() -> Result<()> {
        let buf = Buffer {
            rope: Rope::from_str(
                "foo x
ab foo foo",
            ),
            ..Buffer::empty()
        };
        let mut terminal = Terminal::new(TestBackend::new(30, 2))?;
        terminal.draw(|frame| {
            render_buffer(
                &Theme::default(),
                &buf,
                &View::default(),
                4,
                Marks {
                    highlights: &[0..3, 9..12, 13..16],
                    ..Marks::default()
                },
                frame.area(),
                frame,
            )
        })?;
        let screen = terminal.backend().buffer();
        let marked = |y: u16| -> String {
            (0..30)
                .filter(|&x| {
                    screen[(x, y)]
                        .modifier
                        .contains(ratatui::style::Modifier::REVERSED)
                })
                .map(|x| screen[(x, y)].symbol())
                .collect()
        };
        assert_eq!(marked(0), "foo");
        assert_eq!(marked(1), "foofoo");
        Ok(())
    }

    #[test]
    fn syntax_roles_colour_their_cells() -> Result<()> {
        let mut buf = Buffer {
            rope: Rope::from_str(
                "fn f() {}
	// x",
            ),
            path: Some("a.rs".into()),
            ..Buffer::empty()
        };
        buf.sync_highlight();
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(20, 2))?;
        terminal.draw(|frame| {
            render_buffer(
                &theme,
                &buf,
                &View::default(),
                4,
                Marks::default(),
                frame.area(),
                frame,
            )
        })?;
        let screen = terminal.backend().buffer();
        let fg = |x: u16, y: u16| screen[(x, y)].fg;
        // Gutter " 1 │ " is 5 cells.
        assert_eq!(fg(5, 0), theme.syntax.keyword.fg.unwrap_or_default());
        assert_eq!(fg(6, 0), theme.syntax.keyword.fg.unwrap_or_default());
        assert_eq!(fg(8, 0), theme.syntax.function.fg.unwrap_or_default());
        // The comment starts after a tab, four cells in.
        assert_eq!(fg(9, 1), theme.syntax.comment.fg.unwrap_or_default());
        assert_eq!(fg(12, 1), theme.syntax.comment.fg.unwrap_or_default());
        Ok(())
    }

    #[test]
    fn gutter_is_right_aligned_to_the_widest_number() -> Result<()> {
        let text: String = (1..=12).map(|n| format!("line {n}\n")).collect();
        let buf = Buffer {
            rope: Rope::from_str(text.trim_end()),
            ..Buffer::empty()
        };
        let mut terminal = Terminal::new(TestBackend::new(40, 12))?;
        terminal.draw(|frame| {
            render_buffer(
                &Theme::default(),
                &buf,
                &View::default(),
                4,
                Marks::default(),
                frame.area(),
                frame,
            )
        })?;
        let screen = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..40).map(|x| screen[(x, y)].symbol()).collect() };
        assert!(row(0).starts_with("  1 │ line 1"), "{}", row(0));
        assert!(row(9).starts_with(" 10 │ line 10"), "{}", row(9));
        assert!(row(11).starts_with(" 12 │ line 12"), "{}", row(11));
        Ok(())
    }
}
