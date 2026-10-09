mod harness;

use std::time::Duration;

use harness::{COLS, ROWS, Tome};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const RENDER: &str = "tests/fixtures/render.txt";
const LONG: &str = "tests/fixtures/long.txt";
/// The first row of the editor pane, just below the tab header.
const TOP: u16 = 3;
/// The last row of the editor pane, just above the status line.
const LAST_TEXT_ROW: u16 = ROWS - 2;

fn screen_row(tome: &Tome, row: u16) -> String {
    tome.screen()[usize::from(row)].clone()
}

/// Waits for the status line to end with `Ln <line>, Col <col>` and the
/// fixtures' language, `Plain text`.
fn wait_for_position(tome: &Tome, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}    Plain text");
    tome.wait_for_text(&position, WAIT);
    let status = screen_row(tome, ROWS - 1);
    assert!(
        status.trim_end().ends_with(&position),
        "status line {status:?} should end with {position:?}"
    );
}

/// Presses `key`, then waits for the status line and the terminal cursor to agree
/// that the cursor is at `line`/`col` and drawn on screen cell (`x`, `y`).
fn press(tome: &mut Tome, key: &str, (line, col): (usize, usize), (x, y): (u16, u16)) {
    tome.send_keys(key);
    wait_for_position(tome, line, col);
    tome.wait_for_cursor(x, y, WAIT);
    assert_eq!(tome.cursor(), (x, y), "after {key}");
}

fn col_of(tome: &Tome, row: u16, text: &str) -> u16 {
    tome.text_col(row, text)
        .unwrap_or_else(|| panic!("{text:?} not on row {row}: {:#?}", tome.screen()))
}

fn quit(tome: &mut Tome) {
    tome.send_keys("ctrl+q");
    let status = tome.wait_exit(WAIT);
    assert!(status.success(), "tome exited with {status:?}");
}

#[test]
fn status_shows_position() {
    let mut tome = Tome::spawn(&[RENDER]);
    tome.wait_for_text("fn main() {", START);
    let x = col_of(&tome, TOP, "fn main() {");

    wait_for_position(&tome, 1, 1);
    tome.wait_for_cursor(x, TOP, WAIT);
    assert_eq!(tome.cursor(), (x, TOP));

    let g = &mut tome;
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

    quit(&mut tome);
}

#[test]
fn scrolls_to_cursor() {
    let mut tome = Tome::spawn(&[LONG]);
    // The first frame can arrive in pieces; wait for its last text row, not its first.
    tome.wait_for_text("line 26", START);
    // 200 lines: a 3-digit gutter, so text starts at column 7.
    let x = col_of(&tome, TOP, "line 1");
    assert!(screen_row(&tome, LAST_TEXT_ROW).ends_with(" line 26"));

    // Down to the last visible row: no scrolling yet.
    for line in 2..=26 {
        tome.send_keys("down");
        wait_for_position(&tome, line, 1);
    }
    tome.wait_for_cursor(x, LAST_TEXT_ROW, WAIT);
    assert!(screen_row(&tome, TOP).ends_with(" line 1"));

    // One more scrolls by one line.
    press(&mut tome, "down", (27, 1), (x, LAST_TEXT_ROW));
    assert!(screen_row(&tome, TOP).ends_with(" line 2"));
    assert!(screen_row(&tome, LAST_TEXT_ROW).ends_with(" line 27"));

    // Back above the first visible row scrolls up.
    press(&mut tome, "ctrl+home", (1, 1), (x, TOP));
    assert!(screen_row(&tome, TOP).ends_with(" line 1"));

    press(&mut tome, "pagedown", (27, 1), (x, LAST_TEXT_ROW));
    assert!(screen_row(&tome, TOP).ends_with(" line 2"));

    // The visual check: line 200 on the last text row.
    press(&mut tome, "ctrl+end", (200, 9), (x + 8, LAST_TEXT_ROW));
    assert!(
        screen_row(&tome, LAST_TEXT_ROW).ends_with(" line 200"),
        "{:#?}",
        tome.screen()
    );
    assert!(screen_row(&tome, TOP).ends_with(" line 175"));

    press(&mut tome, "pageup", (174, 9), (x + 8, TOP));
    assert!(screen_row(&tome, TOP).ends_with(" line 174"));

    quit(&mut tome);
}

#[test]
fn scrolls_sideways_on_long_line() {
    let mut tome = Tome::spawn(&[RENDER]);
    tome.wait_for_text("fn main() {", START);
    let x = col_of(&tome, TOP, "fn main() {");
    let long_row = TOP + 4;
    assert!(!tome.screen().iter().any(|row| row.contains("END")));

    for line in 2..=5 {
        tome.send_keys("down");
        wait_for_position(&tome, line, 1);
    }
    // `long ` + 200 letters + ` END`: 209 chars, past the right edge.
    press(&mut tome, "end", (5, 210), (COLS - 1, long_row));
    let row = screen_row(&tome, long_row);
    assert!(row.ends_with(" END"), "{:#?}", tome.screen());
    // The whole pane scrolled sideways, so the short lines are out of view.
    assert!(
        !tome.screen()[usize::from(TOP)].contains("fn main"),
        "{:#?}",
        tome.screen()
    );

    press(&mut tome, "home", (5, 1), (x, long_row));
    assert!(screen_row(&tome, TOP).contains("fn main() {"));
    assert!(!tome.screen().iter().any(|row| row.contains("END")));

    quit(&mut tome);
}
