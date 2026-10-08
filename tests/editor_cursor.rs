mod harness;

use std::time::Duration;

use harness::{COLS, Glyph, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const RENDER: &str = "tests/fixtures/render.txt";
const LONG: &str = "tests/fixtures/long.txt";
/// The first row of the editor pane, just below the tab header.
const TOP: u16 = 3;
/// The last row of the editor pane, just above the status line.
const LAST_TEXT_ROW: u16 = ROWS - 2;

fn screen_row(glyph: &Glyph, row: u16) -> String {
    glyph.screen()[usize::from(row)].clone()
}

/// Waits for the status line to end with `Ln <line>, Col <col>` and the
/// fixtures' language, `Plain text`.
fn wait_for_position(glyph: &Glyph, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}    Plain text");
    glyph.wait_for_text(&position, WAIT);
    let status = screen_row(glyph, ROWS - 1);
    assert!(
        status.trim_end().ends_with(&position),
        "status line {status:?} should end with {position:?}"
    );
}

/// Presses `key`, then waits for the status line and the terminal cursor to agree
/// that the cursor is at `line`/`col` and drawn on screen cell (`x`, `y`).
fn press(glyph: &mut Glyph, key: &str, (line, col): (usize, usize), (x, y): (u16, u16)) {
    glyph.send_keys(key);
    wait_for_position(glyph, line, col);
    glyph.wait_for_cursor(x, y, WAIT);
    assert_eq!(glyph.cursor(), (x, y), "after {key}");
}

fn col_of(glyph: &Glyph, row: u16, text: &str) -> u16 {
    glyph
        .text_col(row, text)
        .unwrap_or_else(|| panic!("{text:?} not on row {row}: {:#?}", glyph.screen()))
}

fn quit(glyph: &mut Glyph) {
    glyph.send_keys("ctrl+q");
    let status = glyph.wait_exit(WAIT);
    assert!(status.success(), "glyph exited with {status:?}");
}

#[test]
fn status_shows_position() {
    let mut glyph = Glyph::spawn(&[RENDER]);
    glyph.wait_for_text("fn main() {", START);
    let x = col_of(&glyph, TOP, "fn main() {");

    wait_for_position(&glyph, 1, 1);
    glyph.wait_for_cursor(x, TOP, WAIT);
    assert_eq!(glyph.cursor(), (x, TOP));

    let g = &mut glyph;
    press(g, "right", (1, 2), (x + 1, 3));
    press(g, "right", (1, 3), (x + 2, 3));
    press(g, "right", (1, 4), (x + 3, 3));
    // Line 2 starts with a tab filling cells 0-3; column 3 is inside it.
    press(g, "down", (2, 1), (x, 4));
    // `\tlet x = 1;`: 11 chars, 14 cells.
    press(g, "end", (2, 12), (x + 14, 4));
    // `日本語 ok` is 9 cells wide, shorter than the goal column 14.
    press(g, "down", (3, 7), (x + 9, 5));
    // The goal column survives the shorter line.
    press(g, "up", (2, 12), (x + 14, 4));
    press(g, "down", (3, 7), (x + 9, 5));
    // `😀 ok`: a 2-cell emoji, then ` ok`.
    press(g, "down", (4, 5), (x + 5, 6));
    press(g, "ctrl+left", (4, 3), (x + 3, 6));
    press(g, "ctrl+left", (4, 1), (x, 6));
    press(g, "ctrl+right", (4, 2), (x + 2, 6));
    press(g, "left", (4, 1), (x, 6));
    press(g, "up", (3, 1), (x, 5));
    press(g, "right", (3, 2), (x + 2, 5));
    press(g, "home", (3, 1), (x, 5));
    press(g, "ctrl+end", (6, 1), (x, 8));
    press(g, "ctrl+home", (1, 1), (x, 3));

    quit(&mut glyph);
}

#[test]
fn scrolls_to_cursor() {
    let mut glyph = Glyph::spawn(&[LONG]);
    // The first frame can arrive in pieces; wait for its last text row, not its first.
    glyph.wait_for_text("line 26", START);
    // 200 lines: a 3-digit gutter, so text starts at column 7.
    let x = col_of(&glyph, TOP, "line 1");
    assert!(screen_row(&glyph, LAST_TEXT_ROW).ends_with(" line 26"));

    // Down to the last visible row: no scrolling yet.
    for line in 2..=26 {
        glyph.send_keys("down");
        wait_for_position(&glyph, line, 1);
    }
    glyph.wait_for_cursor(x, LAST_TEXT_ROW, WAIT);
    assert!(screen_row(&glyph, TOP).ends_with(" line 1"));

    // One more scrolls by one line.
    press(&mut glyph, "down", (27, 1), (x, LAST_TEXT_ROW));
    assert!(screen_row(&glyph, TOP).ends_with(" line 2"));
    assert!(screen_row(&glyph, LAST_TEXT_ROW).ends_with(" line 27"));

    // Back above the first visible row scrolls up.
    press(&mut glyph, "ctrl+home", (1, 1), (x, TOP));
    assert!(screen_row(&glyph, TOP).ends_with(" line 1"));

    press(&mut glyph, "pagedown", (27, 1), (x, LAST_TEXT_ROW));
    assert!(screen_row(&glyph, TOP).ends_with(" line 2"));

    // The visual check: line 200 on the last text row.
    press(&mut glyph, "ctrl+end", (200, 9), (x + 8, LAST_TEXT_ROW));
    assert!(
        screen_row(&glyph, LAST_TEXT_ROW).ends_with(" line 200"),
        "{:#?}",
        glyph.screen()
    );
    assert!(screen_row(&glyph, TOP).ends_with(" line 175"));

    press(&mut glyph, "pageup", (174, 9), (x + 8, TOP));
    assert!(screen_row(&glyph, TOP).ends_with(" line 174"));

    quit(&mut glyph);
}

#[test]
fn scrolls_sideways_on_long_line() {
    let mut glyph = Glyph::spawn(&[RENDER]);
    glyph.wait_for_text("fn main() {", START);
    let x = col_of(&glyph, TOP, "fn main() {");
    let long_row = TOP + 4;
    assert!(!glyph.screen().iter().any(|row| row.contains("END")));

    for line in 2..=5 {
        glyph.send_keys("down");
        wait_for_position(&glyph, line, 1);
    }
    // `long ` + 200 letters + ` END`: 209 chars, past the right edge.
    press(&mut glyph, "end", (5, 210), (COLS - 1, long_row));
    let row = screen_row(&glyph, long_row);
    assert!(row.ends_with(" END"), "{:#?}", glyph.screen());
    // The whole pane scrolled sideways, so the short lines are out of view.
    assert!(
        !glyph.screen()[usize::from(TOP)].contains("fn main"),
        "{:#?}",
        glyph.screen()
    );

    press(&mut glyph, "home", (5, 1), (x, long_row));
    assert!(screen_row(&glyph, TOP).contains("fn main() {"));
    assert!(!glyph.screen().iter().any(|row| row.contains("END")));

    quit(&mut glyph);
}
