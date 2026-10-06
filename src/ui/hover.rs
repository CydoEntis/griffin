//! The hover popup: a small card anchored at the cursor showing what the language
//! server said about the symbol under it, as plain text wrapped to the card.
//! `anchor` places it and is kept apart so the completion popup can share it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::Clear;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::Theme;

/// Widest and tallest the text gets inside the card.
const MAX_TEXT_WIDTH: u16 = 60;
const MAX_TEXT_HEIGHT: u16 = 12;
/// Border plus one cell of padding on each side.
const SIDE: u16 = 2;

/// Where a card of `width` by `height` goes for the cursor at screen cell
/// `cursor`, inside `bounds`: on the row below the cursor, starting at its column.
/// When it doesn't fit below it flips above, and when neither side has room it
/// takes the roomier one, cut to fit. Near the right edge it shifts left. The
/// cursor's own cell is never covered.
pub fn anchor(cursor: (u16, u16), width: u16, height: u16, bounds: Rect) -> Rect {
    let (col, row) = cursor;
    let width = width.min(bounds.width);
    let x = col.min(bounds.right().saturating_sub(width)).max(bounds.x);
    let below = bounds.bottom().saturating_sub(row.saturating_add(1));
    let above = row.saturating_sub(bounds.y);
    let (y, height) = if height <= below {
        (row + 1, height)
    } else if height <= above {
        (row - height, height)
    } else if below >= above {
        (row + 1, below)
    } else {
        (bounds.y, above)
    };
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// `text` broken into lines no wider than `width` cells: at the last space that
/// fits, or mid-word when a word is wider than the line. Line breaks in `text`
/// are kept and tabs become four spaces.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.replace('\t', "    ");
        let mut current = String::new();
        let mut current_width = 0;
        // Byte index just past the last space in `current`.
        let mut last_space = None;
        for ch in line.chars() {
            let w = ch.width().unwrap_or(0);
            if current_width + w > width && !current.is_empty() {
                if ch == ' ' {
                    out.push(current.trim_end().to_string());
                    current.clear();
                    current_width = 0;
                    last_space = None;
                    continue;
                }
                match last_space {
                    Some(at) if !current[..at].trim().is_empty() => {
                        let rest = current.split_off(at);
                        out.push(current.trim_end().to_string());
                        current_width = rest.width();
                        current = rest;
                    }
                    _ => {
                        out.push(std::mem::take(&mut current));
                        current_width = 0;
                    }
                }
                last_space = None;
            }
            current.push(ch);
            current_width += w;
            if ch == ' ' {
                last_space = Some(current.len());
            }
        }
        out.push(current.trim_end().to_string());
    }
    out
}

/// Draws `text` in a card anchored at the cursor cell `cursor`, kept inside
/// `bounds`. Leaves the terminal cursor where it was.
pub fn render_hover(
    theme: &Theme,
    text: &str,
    cursor: (u16, u16),
    bounds: Rect,
    frame: &mut Frame,
) {
    let room = bounds.width.saturating_sub(SIDE * 2).min(MAX_TEXT_WIDTH);
    let longest = text.lines().map(UnicodeWidthStr::width).max().unwrap_or(0);
    let text_width = u16::try_from(longest)
        .unwrap_or(u16::MAX)
        .clamp(1, room.max(1));
    let lines = wrap(text, usize::from(text_width));
    let rows = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .min(MAX_TEXT_HEIGHT);
    let card = anchor(cursor, text_width + SIDE * 2, rows + 2, bounds);
    if card.width <= SIDE * 2 || card.height <= 2 {
        return;
    }
    frame.render_widget(Clear, card);
    let block = super::card_block(theme);
    let inner = block.inner(card);
    frame.render_widget(block, card);
    let width = usize::from(inner.width.saturating_sub(2));
    let out = frame.buffer_mut();
    for (y, line) in (inner.y..inner.bottom()).zip(&lines) {
        out.set_stringn(inner.x + 1, y, line, width, ratatui::style::Style::new());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect {
        x: 0,
        y: 0,
        width: 100,
        height: 29,
    };

    #[test]
    fn the_card_sits_below_the_cursor_at_its_column() {
        assert_eq!(anchor((10, 5), 20, 4, SCREEN), Rect::new(10, 6, 20, 4));
    }

    #[test]
    fn near_the_bottom_it_flips_above() {
        assert_eq!(anchor((10, 28), 20, 4, SCREEN), Rect::new(10, 24, 20, 4));
        // Exactly enough room below still goes below.
        assert_eq!(anchor((10, 24), 20, 4, SCREEN), Rect::new(10, 25, 20, 4));
    }

    #[test]
    fn near_the_right_edge_it_shifts_left() {
        assert_eq!(anchor((95, 5), 20, 4, SCREEN), Rect::new(80, 6, 20, 4));
        assert_eq!(anchor((95, 5), 200, 4, SCREEN), Rect::new(0, 6, 100, 4));
    }

    #[test]
    fn without_room_either_side_it_takes_the_roomier_cut_to_fit() {
        let short = Rect::new(0, 0, 100, 10);
        assert_eq!(anchor((0, 2), 20, 12, short), Rect::new(0, 3, 20, 7));
        assert_eq!(anchor((0, 7), 20, 12, short), Rect::new(0, 0, 20, 7));
    }

    #[test]
    fn wrapping_breaks_at_spaces_and_keeps_line_breaks() {
        assert_eq!(
            wrap("the quick brown fox\n\njumps", 10),
            ["the quick", "brown fox", "", "jumps"]
        );
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("ab  cd", 3), ["ab", "cd"]);
        assert_eq!(wrap("\tx", 10), ["    x"]);
        // Wide characters count two cells.
        assert_eq!(wrap("日本語", 4), ["日本", "語"]);
    }
}
