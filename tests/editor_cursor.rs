mod harness;

use std::time::Duration;

use harness::{COLS, Griffin, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const RENDER: &str = "tests/fixtures/render.txt";
const LONG: &str = "tests/fixtures/long.txt";
/// The last row of the editor pane, just above the status line.
const LAST_TEXT_ROW: u16 = ROWS - 2;

fn screen_row(griffin: &Griffin, row: u16) -> String {
    griffin.screen()[usize::from(row)].clone()
}

/// Waits for the status line to end with `Ln <line>, Col <col>`.
fn wait_for_position(griffin: &Griffin, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    griffin.wait_for_text(&position, WAIT);
    let status = screen_row(griffin, ROWS - 1);
    assert!(
        status.trim_end().ends_with(&position),
        "status line {status:?} should end with {position:?}"
    );
}

/// Presses `key`, then waits for the status line and the terminal cursor to agree
/// that the cursor is at `line`/`col` and drawn on screen cell (`x`, `y`).
fn press(griffin: &mut Griffin, key: &str, (line, col): (usize, usize), (x, y): (u16, u16)) {
    griffin.send_keys(key);
    wait_for_position(griffin, line, col);
    griffin.wait_for_cursor(x, y, WAIT);
    assert_eq!(griffin.cursor(), (x, y), "after {key}");
}

fn col_of(griffin: &Griffin, row: u16, text: &str) -> u16 {
    griffin
        .text_col(row, text)
        .unwrap_or_else(|| panic!("{text:?} not on row {row}: {:#?}", griffin.screen()))
}

fn quit(griffin: &mut Griffin) {
    griffin.send_keys("ctrl+q");
    let status = griffin.wait_exit(WAIT);
    assert!(status.success(), "griffin exited with {status:?}");
}

#[test]
fn status_shows_position() {
    let mut griffin = Griffin::spawn(&[RENDER]);
    griffin.wait_for_text("fn main() {", START);
    let x = col_of(&griffin, 0, "fn main() {");

    wait_for_position(&griffin, 1, 1);
    griffin.wait_for_cursor(x, 0, WAIT);
    assert_eq!(griffin.cursor(), (x, 0));

    let g = &mut griffin;
    press(g, "right", (1, 2), (x + 1, 0));
    press(g, "right", (1, 3), (x + 2, 0));
    press(g, "right", (1, 4), (x + 3, 0));
    // Line 2 starts with a tab filling cells 0-3; column 3 is inside it.
    press(g, "down", (2, 1), (x, 1));
    // `\tlet x = 1;`: 11 chars, 14 cells.
    press(g, "end", (2, 12), (x + 14, 1));
    // `日本語 ok` is 9 cells wide, shorter than the goal column 14.
    press(g, "down", (3, 7), (x + 9, 2));
    // The goal column survives the shorter line.
    press(g, "up", (2, 12), (x + 14, 1));
    press(g, "down", (3, 7), (x + 9, 2));
    // `😀 ok`: a 2-cell emoji, then ` ok`.
    press(g, "down", (4, 5), (x + 5, 3));
    press(g, "ctrl+left", (4, 3), (x + 3, 3));
    press(g, "ctrl+left", (4, 1), (x, 3));
    press(g, "ctrl+right", (4, 2), (x + 2, 3));
    press(g, "left", (4, 1), (x, 3));
    press(g, "up", (3, 1), (x, 2));
    press(g, "right", (3, 2), (x + 2, 2));
    press(g, "home", (3, 1), (x, 2));
    press(g, "ctrl+end", (6, 1), (x, 5));
    press(g, "ctrl+home", (1, 1), (x, 0));

    quit(&mut griffin);
}

#[test]
fn scrolls_to_cursor() {
    let mut griffin = Griffin::spawn(&[LONG]);
    // The first frame can arrive in pieces; wait for its last text row, not its first.
    griffin.wait_for_text("line 29", START);
    // 200 lines: a 3-digit gutter, so text starts at column 7.
    let x = col_of(&griffin, 0, "line 1");
    assert!(screen_row(&griffin, LAST_TEXT_ROW).ends_with(" line 29"));

    // Down to the last visible row: no scrolling yet.
    for line in 2..=29 {
        griffin.send_keys("down");
        wait_for_position(&griffin, line, 1);
    }
    griffin.wait_for_cursor(x, LAST_TEXT_ROW, WAIT);
    assert!(screen_row(&griffin, 0).ends_with(" line 1"));

    // One more scrolls by one line.
    press(&mut griffin, "down", (30, 1), (x, LAST_TEXT_ROW));
    assert!(screen_row(&griffin, 0).ends_with(" line 2"));
    assert!(screen_row(&griffin, LAST_TEXT_ROW).ends_with(" line 30"));

    // Back above the first visible row scrolls up.
    press(&mut griffin, "ctrl+home", (1, 1), (x, 0));
    assert!(screen_row(&griffin, 0).ends_with(" line 1"));

    press(&mut griffin, "pagedown", (30, 1), (x, LAST_TEXT_ROW));
    assert!(screen_row(&griffin, 0).ends_with(" line 2"));

    // The visual check: line 200 on the last text row.
    press(&mut griffin, "ctrl+end", (200, 9), (x + 8, LAST_TEXT_ROW));
    assert!(
        screen_row(&griffin, LAST_TEXT_ROW).ends_with(" line 200"),
        "{:#?}",
        griffin.screen()
    );
    assert!(screen_row(&griffin, 0).ends_with(" line 172"));

    press(&mut griffin, "pageup", (171, 9), (x + 8, 0));
    assert!(screen_row(&griffin, 0).ends_with(" line 171"));

    quit(&mut griffin);
}

#[test]
fn scrolls_sideways_on_long_line() {
    let mut griffin = Griffin::spawn(&[RENDER]);
    griffin.wait_for_text("fn main() {", START);
    let x = col_of(&griffin, 0, "fn main() {");
    let long_row = 4;
    assert!(!griffin.screen().iter().any(|row| row.contains("END")));

    for line in 2..=5 {
        griffin.send_keys("down");
        wait_for_position(&griffin, line, 1);
    }
    // `long ` + 200 letters + ` END`: 209 chars, past the right edge.
    press(&mut griffin, "end", (5, 210), (COLS - 1, long_row));
    let row = screen_row(&griffin, long_row);
    assert!(row.ends_with(" END"), "{:#?}", griffin.screen());
    // The whole pane scrolled sideways, so the short lines are out of view.
    assert!(
        !griffin.screen()[0].contains("fn main"),
        "{:#?}",
        griffin.screen()
    );

    press(&mut griffin, "home", (5, 1), (x, long_row));
    assert!(screen_row(&griffin, 0).contains("fn main() {"));
    assert!(!griffin.screen().iter().any(|row| row.contains("END")));

    quit(&mut griffin);
}
