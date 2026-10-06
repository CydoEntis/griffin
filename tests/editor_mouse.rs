mod harness;

use std::fs;
use std::time::Duration;

use harness::{Griffin, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const RENDER: &str = "tests/fixtures/render.txt";
const LONG: &str = "tests/fixtures/long.txt";

/// Waits for the status line to end with `Ln <line>, Col <col>`.
fn wait_for_position(griffin: &Griffin, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    griffin.wait_for_text(&position, WAIT);
    let status = griffin.screen()[usize::from(ROWS - 1)].clone();
    assert!(
        status.trim_end().ends_with(&position),
        "status line {status:?} should end with {position:?}"
    );
}

/// Column of `text` on `row`, panicking with the screen when it's missing.
fn col_of(griffin: &Griffin, row: u16, text: &str) -> u16 {
    griffin
        .text_col(row, text)
        .unwrap_or_else(|| panic!("{text:?} not on row {row}: {:#?}", griffin.screen()))
}

/// The screen row whose trimmed text is exactly `text`.
fn row_of(griffin: &Griffin, text: &str) -> u16 {
    let screen = griffin.screen();
    let row = screen
        .iter()
        .position(|line| line.trim() == text)
        .unwrap_or_else(|| panic!("no row reads {text:?}: {screen:#?}"));
    u16::try_from(row).expect("screen rows fit in u16")
}

/// Opens a scratch file holding `text`.
fn open(text: &str) -> (tempfile::TempDir, Griffin) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("mouse.txt");
    fs::write(&path, text).expect("write fixture");
    let path = path.to_str().expect("temp path is UTF-8").to_string();
    let griffin = Griffin::spawn_with_config("", &[&path]);
    griffin.wait_for_text("Ln 1, Col 1", START);
    (dir, griffin)
}

#[test]
fn click_places_cursor() {
    let mut griffin = Griffin::spawn(&[RENDER]);
    griffin.wait_for_text("😀 ok", START);

    // Past the gutter and a tab: `\tlet x = 1;`, on the `x`.
    let x = col_of(&griffin, 1, "x = 1;");
    griffin.click(x, 1);
    wait_for_position(&griffin, 2, 6);
    griffin.wait_for_cursor(x, 1, WAIT);

    // Scrolled sideways: End on the long line 5 scrolls it, then click on `END`.
    griffin.click(col_of(&griffin, 4, "long"), 4);
    wait_for_position(&griffin, 5, 1);
    griffin.send_keys("end");
    let text = fs::read_to_string(RENDER).expect("read fixture");
    let line = text.lines().nth(4).expect("fixture has a line 5");
    let end_col = line.find("END").expect("line 5 ends in END") + 1;
    wait_for_position(&griffin, 5, line.trim_end().len() + 1);
    let x = col_of(&griffin, 4, "END");
    griffin.click(x, 4);
    wait_for_position(&griffin, 5, end_col);
    griffin.wait_for_cursor(x, 4, WAIT);
}

#[test]
fn click_accounts_for_vertical_scroll() {
    let mut griffin = Griffin::spawn(&[LONG]);
    griffin.wait_for_text("line 1", START);
    griffin.send_keys("ctrl+end");
    wait_for_position(&griffin, 200, 9);
    griffin.wait_for_text("200 │ line 200", WAIT);

    let row = row_of(&griffin, "180 │ line 180");
    // On the `1` of `180`.
    let x = col_of(&griffin, row, "line 180") + 5;
    griffin.click(x, row);
    wait_for_position(&griffin, 180, 6);
    griffin.wait_for_cursor(x, row, WAIT);
}

#[test]
fn click_past_end() {
    let (_dir, mut griffin) = open("one\ntwo");
    griffin.wait_for_text("2 │ two", WAIT);
    let base = col_of(&griffin, 0, "one");

    griffin.click(90, 0);
    wait_for_position(&griffin, 1, 4);
    griffin.wait_for_cursor(base + 3, 0, WAIT);

    // Below the last line: the last line, at the clicked column.
    griffin.click(base + 1, 15);
    wait_for_position(&griffin, 2, 2);
    griffin.wait_for_cursor(base + 1, 1, WAIT);

    // Below and past the end: the end of the last line.
    griffin.click(80, 20);
    wait_for_position(&griffin, 2, 4);
    griffin.wait_for_cursor(base + 3, 1, WAIT);
}

#[test]
fn click_on_wide_char() {
    let mut griffin = Griffin::spawn(&[RENDER]);
    griffin.wait_for_text("😀 ok", START);

    // `日本語 ok`: the right half of `本` is still `本`.
    let base = col_of(&griffin, 2, "日本語");
    griffin.click(base + 3, 2);
    wait_for_position(&griffin, 3, 2);
    griffin.wait_for_cursor(base + 2, 2, WAIT);

    // The spec's visual check: `ok` after `日本語 ` is column 5.
    griffin.click(col_of(&griffin, 2, "ok"), 2);
    wait_for_position(&griffin, 3, 5);

    // Either half of the emoji lands on it.
    let base = col_of(&griffin, 3, "😀");
    griffin.click(base + 1, 3);
    wait_for_position(&griffin, 4, 1);
    griffin.wait_for_cursor(base, 3, WAIT);
}

#[test]
fn drag_selects() {
    let (_dir, mut griffin) = open("hello world");
    griffin.wait_for_text("1 │ hello world", WAIT);
    let base = col_of(&griffin, 0, "hello");

    // From `w` back to `e`: the selection is the same either way round.
    griffin.drag((base + 6, 0), (base + 1, 0));
    griffin.wait_for_reversed(0, "ello ", WAIT);
    wait_for_position(&griffin, 1, 2);

    griffin.type_text("x");
    griffin.wait_for_text("1 │ hxworld", WAIT);
    griffin.wait_for_reversed(0, "", WAIT);
    wait_for_position(&griffin, 1, 3);

    // Undo puts the dragged-over text back in one step.
    griffin.send_keys("ctrl+z");
    griffin.wait_for_text("1 │ hello world", WAIT);
}

#[test]
fn double_click_selects_word() {
    let (_dir, mut griffin) = open("foo bar_baz qux");
    griffin.wait_for_text("1 │ foo bar_baz qux", WAIT);
    let x = col_of(&griffin, 0, "baz");

    griffin.double_click(x, 0);
    griffin.wait_for_reversed(0, "bar_baz", WAIT);
    wait_for_position(&griffin, 1, 12);

    griffin.type_text("x");
    griffin.wait_for_text("1 │ foo x qux", WAIT);
    griffin.wait_for_reversed(0, "", WAIT);
}

#[test]
fn wheel_scrolls() {
    let mut griffin = Griffin::spawn(&[LONG]);
    griffin.wait_for_text("29 │ line 29", START);
    assert_eq!(griffin.screen()[0].trim(), "1 │ line 1");

    griffin.scroll_down(20, 10);
    griffin.wait_for_text("32 │ line 32", WAIT);
    assert_eq!(griffin.screen()[0].trim(), "4 │ line 4");
    griffin.scroll_down(20, 10);
    griffin.wait_for_text("35 │ line 35", WAIT);
    assert_eq!(griffin.screen()[0].trim(), "7 │ line 7");
    wait_for_position(&griffin, 1, 1);

    griffin.scroll_up(20, 10);
    griffin.wait_for_text_gone("35 │ line 35", WAIT);
    assert_eq!(griffin.screen()[0].trim(), "4 │ line 4");
    wait_for_position(&griffin, 1, 1);

    // The cursor never moved: typing lands on line 1 and brings it back into view.
    griffin.type_text("x");
    griffin.wait_for_text("1 │ xline 1", WAIT);
    wait_for_position(&griffin, 1, 2);
}
