mod harness;

use std::fs;
use std::time::Duration;

use harness::{ROWS, Tome};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const RENDER: &str = "tests/fixtures/render.txt";
const LONG: &str = "tests/fixtures/long.txt";

/// Waits for the status line to show `Ln <line>, Col <col>` (see `shows_position`).
fn wait_for_position(tome: &Tome, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    tome.wait_for_text(&position, WAIT);
    let status = tome.screen()[usize::from(ROWS - 1)].clone();
    assert!(
        harness::shows_position(&status, &position),
        "status line {status:?} should show {position:?}"
    );
}

/// Column of `text` on `row`, panicking with the screen when it's missing.
fn col_of(tome: &Tome, row: u16, text: &str) -> u16 {
    tome.text_col(row, text)
        .unwrap_or_else(|| panic!("{text:?} not on row {row}: {:#?}", tome.screen()))
}

/// The screen row whose trimmed text is exactly `text`.
fn row_of(tome: &Tome, text: &str) -> u16 {
    let screen = tome.screen();
    let row = screen
        .iter()
        .position(|line| line.trim() == text)
        .unwrap_or_else(|| panic!("no row reads {text:?}: {screen:#?}"));
    u16::try_from(row).expect("screen rows fit in u16")
}

/// The screen row of buffer line `line` (1-based) while the view is at the top:
/// the editor starts below the three rows of the tab header.
fn row(line: u16) -> u16 {
    line + 2
}

/// Opens a scratch file holding `text`.
fn open(text: &str) -> (tempfile::TempDir, Tome) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("mouse.txt");
    fs::write(&path, text).expect("write fixture");
    let path = path.to_str().expect("temp path is UTF-8").to_string();
    let tome = Tome::spawn_with_config("", &[&path]);
    tome.wait_for_text("Ln 1, Col 1", START);
    (dir, tome)
}

#[test]
fn click_places_cursor() {
    let mut tome = Tome::spawn(&[RENDER]);
    tome.wait_for_text("😀 ok", START);

    // Past the gutter and a tab: `\tlet x = 1;`, on the `x`.
    let x = col_of(&tome, row(2), "x = 1;");
    tome.click(x, row(2));
    wait_for_position(&tome, 2, 6);
    tome.wait_for_cursor(x, row(2), WAIT);

    // Scrolled sideways: End on the long line 5 scrolls it, then click on `END`.
    tome.click(col_of(&tome, row(5), "long"), row(5));
    wait_for_position(&tome, 5, 1);
    tome.send_keys("end");
    let text = fs::read_to_string(RENDER).expect("read fixture");
    let line = text.lines().nth(4).expect("fixture has a line 5");
    let end_col = line.find("END").expect("line 5 ends in END") + 1;
    wait_for_position(&tome, 5, line.trim_end().len() + 1);
    let x = col_of(&tome, row(5), "END");
    tome.click(x, row(5));
    wait_for_position(&tome, 5, end_col);
    tome.wait_for_cursor(x, row(5), WAIT);
}

#[test]
fn click_accounts_for_vertical_scroll() {
    let mut tome = Tome::spawn(&[LONG]);
    tome.wait_for_text("line 1", START);
    tome.send_keys("ctrl+end");
    wait_for_position(&tome, 200, 9);
    tome.wait_for_text("200  line 200", WAIT);

    let row = row_of(&tome, "180  line 180");
    // On the `1` of `180`.
    let x = col_of(&tome, row, "line 180") + 5;
    tome.click(x, row);
    wait_for_position(&tome, 180, 6);
    tome.wait_for_cursor(x, row, WAIT);
}

#[test]
fn click_past_end() {
    let (_dir, mut tome) = open("one\ntwo");
    tome.wait_for_text("2  two", WAIT);
    let base = col_of(&tome, row(1), "one");

    tome.click(90, row(1));
    wait_for_position(&tome, 1, 4);
    tome.wait_for_cursor(base + 3, row(1), WAIT);

    // Below the last line: the last line, at the clicked column.
    tome.click(base + 1, 15);
    wait_for_position(&tome, 2, 2);
    tome.wait_for_cursor(base + 1, row(2), WAIT);

    // Below and past the end: the end of the last line.
    tome.click(80, 20);
    wait_for_position(&tome, 2, 4);
    tome.wait_for_cursor(base + 3, row(2), WAIT);
}

#[test]
fn click_on_wide_char() {
    let mut tome = Tome::spawn(&[RENDER]);
    tome.wait_for_text("😀 ok", START);

    // `日本語 ok`: the right half of `本` is still `本`.
    let base = col_of(&tome, row(3), "日本語");
    tome.click(base + 3, row(3));
    wait_for_position(&tome, 3, 2);
    tome.wait_for_cursor(base + 2, row(3), WAIT);

    // The spec's visual check: `ok` after `日本語 ` is column 5.
    tome.click(col_of(&tome, row(3), "ok"), row(3));
    wait_for_position(&tome, 3, 5);

    // Either half of the emoji lands on it.
    let base = col_of(&tome, row(4), "😀");
    tome.click(base + 1, row(4));
    wait_for_position(&tome, 4, 1);
    tome.wait_for_cursor(base, row(4), WAIT);
}

#[test]
fn clicks_land_past_the_six_cell_gutter() {
    // The gutter is a mark cell, three for the number and two blanks, so the
    // text starts in column 6.
    let (_dir, mut tome) = open("abcdef\nghij");
    tome.wait_for_text("   2  ghij", WAIT);
    assert_eq!(col_of(&tome, row(1), "abcdef"), 6);

    tome.click(8, row(1));
    wait_for_position(&tome, 1, 3);
    tome.wait_for_cursor(8, row(1), WAIT);
    tome.click(6, row(1));
    wait_for_position(&tome, 1, 1);
    tome.wait_for_cursor(6, row(1), WAIT);
    // Anywhere in the gutter is the line's start.
    tome.click(10, row(2));
    wait_for_position(&tome, 2, 5);
    tome.click(3, row(2));
    wait_for_position(&tome, 2, 1);
    tome.wait_for_cursor(6, row(2), WAIT);

    // A drag from column 7 to 10 covers `bcd`.
    tome.drag((7, row(1)), (10, row(1)));
    tome.wait_for_reversed(row(1), "bcd", WAIT);
    wait_for_position(&tome, 1, 5);
}

#[test]
fn drag_selects() {
    let (_dir, mut tome) = open("hello world");
    tome.wait_for_text("1  hello world", WAIT);
    let base = col_of(&tome, row(1), "hello");

    // From `w` back to `e`: the selection is the same either way round.
    tome.drag((base + 6, row(1)), (base + 1, row(1)));
    tome.wait_for_reversed(row(1), "ello ", WAIT);
    wait_for_position(&tome, 1, 2);

    tome.type_text("x");
    tome.wait_for_text("1  hxworld", WAIT);
    tome.wait_for_reversed(row(1), "", WAIT);
    wait_for_position(&tome, 1, 3);

    // Undo puts the dragged-over text back in one step.
    tome.send_keys("ctrl+z");
    tome.wait_for_text("1  hello world", WAIT);
}

#[test]
fn double_click_selects_word() {
    let (_dir, mut tome) = open("foo bar_baz qux");
    tome.wait_for_text("1  foo bar_baz qux", WAIT);
    let x = col_of(&tome, row(1), "baz");

    tome.double_click(x, row(1));
    tome.wait_for_reversed(row(1), "bar_baz", WAIT);
    wait_for_position(&tome, 1, 12);

    tome.type_text("x");
    tome.wait_for_text("1  foo x qux", WAIT);
    tome.wait_for_reversed(row(1), "", WAIT);
}

#[test]
fn wheel_scrolls() {
    let mut tome = Tome::spawn(&[LONG]);
    tome.wait_for_text("26  line 26", START);
    assert_eq!(tome.screen()[3].trim(), "1  line 1");

    tome.scroll_down(20, 10);
    tome.wait_for_text("29  line 29", WAIT);
    assert_eq!(tome.screen()[3].trim(), "4  line 4");
    tome.scroll_down(20, 10);
    tome.wait_for_text("32  line 32", WAIT);
    assert_eq!(tome.screen()[3].trim(), "7  line 7");
    wait_for_position(&tome, 1, 1);

    tome.scroll_up(20, 10);
    tome.wait_for_text_gone("32  line 32", WAIT);
    assert_eq!(tome.screen()[3].trim(), "4  line 4");
    wait_for_position(&tome, 1, 1);

    // The cursor never moved: typing lands on line 1 and brings it back into view.
    tome.type_text("x");
    tome.wait_for_text("1  xline 1", WAIT);
    wait_for_position(&tome, 1, 2);
}
