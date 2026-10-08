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
use crate::theme::{Theme, grad, mix};

/// Ranges drawn over a buffer's text, by char index.
#[derive(Debug, Default, Clone)]
pub struct Marks<'a> {
    /// Find matches, sorted by start; drawn on `find_match_bg`, apart from the
    /// selection so a match never reads as selected text.
    pub highlights: &'a [Range<usize>],
    /// The find match Enter acts on, drawn `bg` on `warn` so it stands out from
    /// the other matches.
    pub current: Option<Range<usize>>,
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

/// The fewest cells a line number gets, so the text doesn't shift sideways as a
/// short file grows past 9 or 99 lines (README §2.4's `D`).
const MIN_NUMBER_WIDTH: usize = 3;

/// Blank cells between the line number and the text; there's no divider rule.
const GUTTER_GAP: usize = 2;

/// Cells the line number is right-aligned in: `max(3, digits)`.
fn number_width(buf: &Buffer) -> usize {
    buf.rope.len_lines().to_string().len().max(MIN_NUMBER_WIDTH)
}

/// Cells left of the text: the diagnostic mark cell, the line number, the gap.
fn gutter_width(buf: &Buffer) -> usize {
    1 + number_width(buf) + GUTTER_GAP
}

/// Cells available for text in `area`.
fn text_width(buf: &Buffer, area: Rect) -> usize {
    usize::from(area.width).saturating_sub(gutter_width(buf))
}

/// The bg of the cell `dx` cells into a cursor row `width` cells wide (README
/// §2.4): it lights up out of the gutter and settles into `cur_line` around the
/// middle, so the eye finds the row without a bright band across the screen.
fn glow_at(theme: &Theme, dx: u16, width: u16) -> Color {
    let stops = [
        mix(theme.bg, theme.accent, 0.20),
        mix(theme.bg, theme.accent, 0.07),
        theme.cur_line,
        theme.cur_line,
    ];
    grad(&stops, f64::from(dx) / (f64::from(width) * 0.8))
}

/// Draws `buf` into `area`: a gutter of diagnostic mark and right-aligned line
/// number, then each line cut at the right edge (Glyph never wraps), and puts the
/// terminal cursor on the buffer cursor when it's in view. In the `focused` split
/// the cursor row glows and its number is lit. Diagnostics are underlined in their
/// colour and mark the gutter with the most severe one on the line; the selection
/// and every highlight get the selection colours on top. Pure: reads its inputs
/// only.
#[allow(clippy::too_many_arguments)]
pub fn render_buffer(
    theme: &Theme,
    buf: &Buffer,
    view: &View,
    tab_width: usize,
    marks: Marks,
    focused: bool,
    area: Rect,
    frame: &mut Frame,
) {
    let Marks {
        highlights,
        current,
        diagnostics,
    } = marks;
    let digits = number_width(buf);
    let gutter_width = gutter_width(buf);
    let text_width = text_width(buf, area);
    let number_style = Style::new().fg(theme.gutter);
    // Only the split being typed in lights its cursor row, so two splits never
    // look equally active; the other names the row in plain `text`.
    let cursor_number_style = if focused {
        Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme.text)
    };
    // `mono`'s terminal colours can't be blended, so it marks the row by the
    // number's weight alone.
    let glow = focused && !theme.flat();
    let (cursor_line, _) = buf.cursor_line_col();
    let selection = buf.selection();
    let syntax = visible_spans(buf, view, area);

    let out = frame.buffer_mut();
    for (screen_row, line_idx) in (view.scroll_row..buf.rope.len_lines())
        .take(usize::from(area.height))
        .enumerate()
    {
        // `take(area.height)` keeps the row inside a u16.
        let y = area.y + screen_row as u16;
        let is_cursor_line = line_idx == cursor_line;
        if is_cursor_line && glow {
            for dx in 0..area.width {
                if let Some(cell) = out.cell_mut((area.x + dx, y)) {
                    cell.set_bg(glow_at(theme, dx, area.width));
                }
            }
        }
        let gutter = format!(" {:>digits$}{}", line_idx + 1, " ".repeat(GUTTER_GAP));
        let style = if is_cursor_line {
            cursor_number_style
        } else {
            number_style
        };
        out.set_stringn(area.x, y, &gutter, usize::from(area.width), style);
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
                    cell.set_style(role_style(theme, role));
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
        let paint = |out: &mut ratatui::buffer::Buffer,
                     range: &Range<usize>,
                     draw: &dyn Fn(&mut ratatui::buffer::Cell)| {
            let cells = selected_cells(buf, line_idx, range, tab_width);
            let from = cells.start.max(view.scroll_col);
            let to = cells.end.min(view.scroll_col + text_width);
            for col in from..to {
                // Below `text_width`, which fits in the area's u16 width.
                let cell_x = x + (col - view.scroll_col) as u16;
                if let Some(cell) = out.cell_mut((cell_x, y)) {
                    draw(cell);
                }
            }
        };
        // Only the highlights that touch this line, found by bisecting the sorted list.
        let first = highlights.partition_point(|h| h.end <= line_start);
        let on_line = highlights[first..]
            .iter()
            .take_while(|h| h.start < line_end)
            .filter(|h| Some(*h) != current.as_ref());
        for range in on_line {
            paint(out, range, &|cell| match_style(theme, cell));
        }
        if let Some(range) = &current {
            paint(out, range, &|cell| current_match_style(theme, cell));
        }
        if let Some(range) = &selection {
            paint(out, range, &|cell| selection_style(theme, cell));
        }
    }

    if let Some((x, y)) = cursor_cell(buf, view, tab_width, area) {
        frame.set_cursor_position((x, y));
    }
}

/// A find match: `find_match_bg` (README §5.5) behind text that keeps its colour.
/// `mono` can't blend that colour, so it underlines the match instead.
fn match_style(theme: &Theme, cell: &mut ratatui::buffer::Cell) {
    if theme.ramps() {
        cell.set_bg(mix(theme.bg, theme.warn, FIND_MATCH_MIX));
    } else {
        cell.modifier.insert(Modifier::UNDERLINED);
    }
}

/// How far `find_match_bg` sits from `bg` towards `warn` (README §5.5).
const FIND_MATCH_MIX: f64 = 0.3;

/// The current find match: `bg` on `warn`. In `mono`, whose `bg` is the
/// terminal's own, the same pair as reverse video.
fn current_match_style(theme: &Theme, cell: &mut ratatui::buffer::Cell) {
    if theme.ramps() {
        cell.set_style(Style::new().fg(theme.bg).bg(theme.warn));
    } else {
        cell.set_style(Theme::highlight(theme.warn, theme.bg));
    }
}

/// Selected text: `sel` behind, the text's own (syntax) colour kept
/// (SPEC_V1_LAYOUT §4). It goes through `Theme::highlight` so `mono` still
/// shows it and the PTY tests still find it as reverse video.
fn selection_style(theme: &Theme, cell: &mut ratatui::buffer::Cell) {
    // Plain text and the blank past a line's end have no colour of their own;
    // reversed, a reset colour would draw the terminal's ground, not text.
    let fg = match cell.fg {
        Color::Reset => theme.fg,
        fg => fg,
    };
    cell.set_style(Theme::highlight(theme.sel, fg));
}

/// How text in `role` is drawn: the theme's syntax style, with comments in
/// italic (README §2.4) so they read as asides in every theme, `mono` included.
fn role_style(theme: &Theme, role: Role) -> Style {
    let style = theme.syntax.style(role);
    if role == Role::Comment {
        style.add_modifier(Modifier::ITALIC)
    } else {
        style
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
        // Gutter "   1  " is 6 cells, leaving 10 for text.
        let mut buf = buffer_at("\tabcdefghijklmnop", 0);
        let area = Rect::new(0, 0, 16, 5);
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
                true,
                frame.area(),
                frame,
            )
        })?;
        // Gutter is 6 cells; the tab and `日` fill 6 more.
        terminal.backend_mut().assert_cursor_position((12, 1));
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
        let mut terminal = Terminal::new(TestBackend::new(21, 3))?;
        terminal.draw(|frame| {
            render_buffer(
                &Theme::default(),
                &buf,
                &View::default(),
                4,
                Marks::default(),
                true,
                frame.area(),
                frame,
            )
        })?;
        let screen = terminal.backend().buffer();
        let reversed = |y: u16| -> String {
            (6..21)
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
        // Gutter is 6 cells.
        assert_eq!(reversed(0), ".#####.........");
        assert_eq!(reversed(1), "#..............");
        assert_eq!(reversed(2), "...............");
        Ok(())
    }

    #[test]
    fn screen_to_char_inverts_the_render_columns() {
        // Gutter "   1  " is 6 cells.
        let buf = buffer_at("ab\n\t日x\nlast", 0);
        let view = View::default();
        let area = Rect::new(0, 0, 20, 10);
        let at = |col, row| view.screen_to_char(&buf, area, col, row, 4);
        assert_eq!(at(6, 0), 0);
        assert_eq!(at(7, 0), 1);
        // Past the end of `ab`.
        assert_eq!(at(16, 0), 2);
        // In the gutter: the line's start.
        assert_eq!(at(1, 1), 3);
        // Anywhere on the tab is the tab; both cells of `日` are `日`.
        assert_eq!(at(9, 1), 3);
        assert_eq!(at(10, 1), 4);
        assert_eq!(at(11, 1), 4);
        assert_eq!(at(12, 1), 5);
        // Below the last line: the last line, at that column.
        assert_eq!(at(7, 9), 8);
    }

    #[test]
    fn screen_to_char_accounts_for_scrolling() {
        let text: String = (0..50).map(|n| format!("{n:02}abcdef\n")).collect();
        let buf = buffer_at(&text, 0);
        let view = View {
            scroll_row: 10,
            scroll_col: 2,
        };
        // An area 3 rows down; gutter "  51  " is 6 cells.
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

    /// Draws `buf` 30 cells wide with `marks`, unfocused so no glow sits behind.
    fn draw_marked(theme: &Theme, buf: &Buffer, marks: Marks) -> Result<ratatui::buffer::Buffer> {
        let mut terminal = Terminal::new(TestBackend::new(30, 2))?;
        terminal.draw(|frame| {
            render_buffer(
                theme,
                buf,
                &View::default(),
                4,
                marks,
                false,
                frame.area(),
                frame,
            )
        })?;
        Ok(terminal.backend().buffer().clone())
    }

    /// The symbols on row `y` of `screen` whose cell passes `keep`.
    fn cells_where(
        screen: &ratatui::buffer::Buffer,
        y: u16,
        keep: impl Fn(&ratatui::buffer::Cell) -> bool,
    ) -> String {
        (0..30)
            .filter(|&x| keep(&screen[(x, y)]))
            .map(|x| screen[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn find_matches_have_their_own_colours_apart_from_the_selection() -> Result<()> {
        let theme = Theme::named("aurora").expect("aurora exists");
        let buf = buffer_at("foo x\nab foo foo", 0);
        let screen = draw_marked(
            &theme,
            &buf,
            Marks {
                highlights: &[0..3, 9..12, 13..16],
                current: Some(9..12),
                ..Marks::default()
            },
        )?;
        let match_bg = mix(theme.bg, theme.warn, 0.3);
        assert_eq!(cells_where(&screen, 0, |c| c.bg == match_bg), "foo");
        assert_eq!(cells_where(&screen, 1, |c| c.bg == match_bg), "foo");
        // The current one is `bg` on `warn`. Gutter "   1  " is 6 cells.
        assert_eq!(cells_where(&screen, 1, |c| c.bg == theme.warn), "foo");
        assert_eq!(screen[(9, 1)].fg, theme.bg);
        // The other matches keep the text's colour, and none is reverse video,
        // so a match never reads as selected text.
        assert_eq!(screen[(6, 0)].fg, Color::Reset);
        for y in 0..2 {
            assert_eq!(
                cells_where(&screen, y, |c| c.modifier.contains(Modifier::REVERSED)),
                ""
            );
        }
        Ok(())
    }

    #[test]
    fn selection_keeps_the_syntax_colour_on_sel() -> Result<()> {
        let theme = Theme::named("aurora").expect("aurora exists");
        let mut buf = Buffer {
            rope: Rope::from_str("fn f\nx"),
            path: Some("a.rs".into()),
            ..Buffer::empty()
        };
        buf.sync_highlight();
        // `fn f` and the line break after it.
        buf.anchor = Some(0);
        buf.cursor = 5;
        let screen = draw_marked(&theme, &buf, Marks::default())?;
        // Reverse video over swapped colours: it shows the cell's bg-slot colour
        // as text on its fg-slot colour, so the keyword's own colour on `sel`.
        let keyword = theme.syntax.keyword.fg.unwrap_or_default();
        assert!(screen[(6, 0)].modifier.contains(Modifier::REVERSED));
        assert_eq!(screen[(6, 0)].fg, theme.sel);
        assert_eq!(screen[(6, 0)].bg, keyword);
        // The selected line break is one `sel` cell past the line's end.
        assert_eq!(
            cells_where(&screen, 0, |c| c.modifier.contains(Modifier::REVERSED)),
            "fn f "
        );
        assert_eq!(screen[(10, 0)].fg, theme.sel);
        assert_eq!(screen[(10, 0)].bg, theme.fg);
        Ok(())
    }

    #[test]
    fn mono_underlines_matches_and_reverses_the_current_one() -> Result<()> {
        let theme = Theme::named("mono").expect("mono exists");
        let mut buf = buffer_at("foo foo x", 9);
        buf.anchor = Some(8);
        let screen = draw_marked(
            &theme,
            &buf,
            Marks {
                highlights: &[0..3, 4..7],
                current: Some(4..7),
                ..Marks::default()
            },
        )?;
        let with = |m: Modifier| cells_where(&screen, 0, |c| c.modifier.contains(m));
        assert_eq!(with(Modifier::UNDERLINED), "foo");
        // The current match and the selected `x` are reverse video.
        assert_eq!(with(Modifier::REVERSED), "foox");
        assert_eq!(screen[(10, 0)].fg, theme.warn);
        assert_eq!(screen[(14, 0)].fg, theme.sel);
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
                true,
                frame.area(),
                frame,
            )
        })?;
        let screen = terminal.backend().buffer();
        let fg = |x: u16, y: u16| screen[(x, y)].fg;
        // Gutter "   1  " is 6 cells.
        assert_eq!(fg(6, 0), theme.syntax.keyword.fg.unwrap_or_default());
        assert_eq!(fg(7, 0), theme.syntax.keyword.fg.unwrap_or_default());
        assert_eq!(fg(9, 0), theme.syntax.function.fg.unwrap_or_default());
        // The comment starts after a tab, four cells in, and is italic.
        assert_eq!(fg(10, 1), theme.syntax.comment.fg.unwrap_or_default());
        assert_eq!(fg(13, 1), theme.syntax.comment.fg.unwrap_or_default());
        assert!(screen[(10, 1)].modifier.contains(Modifier::ITALIC));
        assert!(!screen[(6, 0)].modifier.contains(Modifier::ITALIC));
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
                true,
                frame.area(),
                frame,
            )
        })?;
        let screen = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..40).map(|x| screen[(x, y)].symbol()).collect() };
        // At least three cells for the number, then two blanks and no rule.
        assert!(row(0).starts_with("   1  line 1"), "{}", row(0));
        assert!(row(9).starts_with("  10  line 10"), "{}", row(9));
        assert!(row(11).starts_with("  12  line 12"), "{}", row(11));
        Ok(())
    }

    #[test]
    fn gutter_grows_past_three_digits() {
        let text = "x\n".repeat(1200);
        // 1201 lines: four digits, so 1 + 4 + 2.
        assert_eq!(gutter_width(&buffer_at(&text, 0)), 7);
        assert_eq!(gutter_width(&buffer_at("x", 0)), 6);
    }

    /// Draws a three-line buffer 40 cells wide with the cursor on line 2.
    fn draw_cursor_row(theme: &Theme, focused: bool) -> Result<ratatui::buffer::Buffer> {
        let buf = buffer_at("one\ntwo\nthree", 5);
        let mut terminal = Terminal::new(TestBackend::new(40, 3))?;
        terminal.draw(|frame| {
            render_buffer(
                theme,
                &buf,
                &View::default(),
                4,
                Marks::default(),
                focused,
                frame.area(),
                frame,
            )
        })?;
        Ok(terminal.backend().buffer().clone())
    }

    #[test]
    fn focused_cursor_row_glows_out_of_the_gutter() -> Result<()> {
        let theme = Theme::named("aurora").expect("aurora exists");
        let screen = draw_cursor_row(&theme, true)?;
        // The ramp starts a fifth of the way to `accent` in the mark cell...
        assert_eq!(screen[(0, 1)].bg, mix(theme.bg, theme.accent, 0.20));
        // ...and has settled into `cur_line` two thirds of the way along the
        // ramp, which spans 80 % of the width.
        assert_eq!(screen[(22, 1)].bg, theme.cur_line);
        assert_eq!(screen[(39, 1)].bg, theme.cur_line);
        // In between it is still brighter than `cur_line`.
        assert_eq!(screen[(8, 1)].bg, glow_at(&theme, 8, 40));
        assert_ne!(screen[(8, 1)].bg, theme.cur_line);
        // Other rows keep the ground.
        assert_eq!(screen[(0, 0)].bg, Color::Reset);
        // The cursor line's number is `accent` bold; the others `gutter`.
        assert_eq!(screen[(3, 1)].fg, theme.accent);
        assert!(screen[(3, 1)].modifier.contains(Modifier::BOLD));
        assert_eq!(screen[(3, 0)].fg, theme.gutter);
        assert!(!screen[(3, 0)].modifier.contains(Modifier::BOLD));
        Ok(())
    }

    #[test]
    fn unfocused_split_has_no_glow() -> Result<()> {
        let theme = Theme::named("aurora").expect("aurora exists");
        let screen = draw_cursor_row(&theme, false)?;
        assert_eq!(screen[(0, 1)].bg, Color::Reset);
        assert_eq!(screen[(20, 1)].bg, Color::Reset);
        assert_eq!(screen[(3, 1)].fg, theme.text);
        assert!(!screen[(3, 1)].modifier.contains(Modifier::BOLD));
        Ok(())
    }

    #[test]
    fn mono_marks_the_cursor_row_by_weight_only() -> Result<()> {
        let theme = Theme::named("mono").expect("mono exists");
        let screen = draw_cursor_row(&theme, true)?;
        assert_eq!(screen[(0, 1)].bg, Color::Reset);
        assert_eq!(screen[(20, 1)].bg, Color::Reset);
        assert!(screen[(3, 1)].modifier.contains(Modifier::BOLD));
        Ok(())
    }
}
