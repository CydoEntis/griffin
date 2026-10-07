mod harness;

use std::fs;
use std::time::Duration;

use harness::{Glyph, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const RENDER: &str = "tests/fixtures/render.txt";
const LONG: &str = "tests/fixtures/long.txt";

/// Waits for the status line to end with `Ln <line>, Col <col>`.
fn wait_for_position(glyph: &Glyph, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    glyph.wait_for_text(&position, WAIT);
    let status = glyph.screen()[usize::from(ROWS - 1)].clone();
    assert!(
        status.trim_end().ends_with(&position),
        "status line {status:?} should end with {position:?}"
    );
}

/// Column of `text` on `row`, panicking with the screen when it's missing.
fn col_of(glyph: &Glyph, row: u16, text: &str) -> u16 {
    glyph
        .text_col(row, text)
        .unwrap_or_else(|| panic!("{text:?} not on row {row}: {:#?}", glyph.screen()))
}

/// The screen row whose trimmed text is exactly `text`.
fn row_of(glyph: &Glyph, text: &str) -> u16 {
    let screen = glyph.screen();
    let row = screen
        .iter()
        .position(|line| line.trim() == text)
        .unwrap_or_else(|| panic!("no row reads {text:?}: {screen:#?}"));
    u16::try_from(row).expect("screen rows fit in u16")
}

/// Opens a scratch file holding `text`.
fn open(text: &str) -> (tempfile::TempDir, Glyph) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("mouse.txt");
    fs::write(&path, text).expect("write fixture");
    let path = path.to_str().expect("temp path is UTF-8").to_string();
    let glyph = Glyph::spawn_with_config("", &[&path]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    (dir, glyph)
}

#[test]
fn click_places_cursor() {
    let mut glyph = Glyph::spawn(&[RENDER]);
    glyph.wait_for_text("😀 ok", START);

    // Past the gutter and a tab: `\tlet x = 1;`, on the `x`.
    let x = col_of(&glyph, 2, "x = 1;");
    glyph.click(x, 2);
    wait_for_position(&glyph, 2, 6);
    glyph.wait_for_cursor(x, 2, WAIT);

    // Scrolled sideways: End on the long line 5 scrolls it, then click on `END`.
    glyph.click(col_of(&glyph, 5, "long"), 5);
    wait_for_position(&glyph, 5, 1);
    glyph.send_keys("end");
    let text = fs::read_to_string(RENDER).expect("read fixture");
    let line = text.lines().nth(4).expect("fixture has a line 5");
    let end_col = line.find("END").expect("line 5 ends in END") + 1;
    wait_for_position(&glyph, 5, line.trim_end().len() + 1);
    let x = col_of(&glyph, 5, "END");
    glyph.click(x, 5);
    wait_for_position(&glyph, 5, end_col);
    glyph.wait_for_cursor(x, 5, WAIT);
}

#[test]
fn click_accounts_for_vertical_scroll() {
    let mut glyph = Glyph::spawn(&[LONG]);
    glyph.wait_for_text("line 1", START);
    glyph.send_keys("ctrl+end");
    wait_for_position(&glyph, 200, 9);
    glyph.wait_for_text("200 │ line 200", WAIT);

    let row = row_of(&glyph, "180 │ line 180");
    // On the `1` of `180`.
    let x = col_of(&glyph, row, "line 180") + 5;
    glyph.click(x, row);
    wait_for_position(&glyph, 180, 6);
    glyph.wait_for_cursor(x, row, WAIT);
}

#[test]
fn click_past_end() {
    let (_dir, mut glyph) = open("one\ntwo");
    glyph.wait_for_text("2 │ two", WAIT);
    let base = col_of(&glyph, 1, "one");

    glyph.click(90, 1);
    wait_for_position(&glyph, 1, 4);
    glyph.wait_for_cursor(base + 3, 1, WAIT);

    // Below the last line: the last line, at the clicked column.
    glyph.click(base + 1, 15);
    wait_for_position(&glyph, 2, 2);
    glyph.wait_for_cursor(base + 1, 2, WAIT);

    // Below and past the end: the end of the last line.
    glyph.click(80, 20);
    wait_for_position(&glyph, 2, 4);
    glyph.wait_for_cursor(base + 3, 2, WAIT);
}

#[test]
fn click_on_wide_char() {
    let mut glyph = Glyph::spawn(&[RENDER]);
    glyph.wait_for_text("😀 ok", START);

    // `日本語 ok`: the right half of `本` is still `本`.
    let base = col_of(&glyph, 3, "日本語");
    glyph.click(base + 3, 3);
    wait_for_position(&glyph, 3, 2);
    glyph.wait_for_cursor(base + 2, 3, WAIT);

    // The spec's visual check: `ok` after `日本語 ` is column 5.
    glyph.click(col_of(&glyph, 3, "ok"), 3);
    wait_for_position(&glyph, 3, 5);

    // Either half of the emoji lands on it.
    let base = col_of(&glyph, 4, "😀");
    glyph.click(base + 1, 4);
    wait_for_position(&glyph, 4, 1);
    glyph.wait_for_cursor(base, 4, WAIT);
}

#[test]
fn drag_selects() {
    let (_dir, mut glyph) = open("hello world");
    glyph.wait_for_text("1 │ hello world", WAIT);
    let base = col_of(&glyph, 1, "hello");

    // From `w` back to `e`: the selection is the same either way round.
    glyph.drag((base + 6, 1), (base + 1, 1));
    glyph.wait_for_reversed(1, "ello ", WAIT);
    wait_for_position(&glyph, 1, 2);

    glyph.type_text("x");
    glyph.wait_for_text("1 │ hxworld", WAIT);
    glyph.wait_for_reversed(1, "", WAIT);
    wait_for_position(&glyph, 1, 3);

    // Undo puts the dragged-over text back in one step.
    glyph.send_keys("ctrl+z");
    glyph.wait_for_text("1 │ hello world", WAIT);
}

#[test]
fn double_click_selects_word() {
    let (_dir, mut glyph) = open("foo bar_baz qux");
    glyph.wait_for_text("1 │ foo bar_baz qux", WAIT);
    let x = col_of(&glyph, 1, "baz");

    glyph.double_click(x, 1);
    glyph.wait_for_reversed(1, "bar_baz", WAIT);
    wait_for_position(&glyph, 1, 12);

    glyph.type_text("x");
    glyph.wait_for_text("1 │ foo x qux", WAIT);
    glyph.wait_for_reversed(1, "", WAIT);
}

#[test]
fn wheel_scrolls() {
    let mut glyph = Glyph::spawn(&[LONG]);
    glyph.wait_for_text("28 │ line 28", START);
    assert_eq!(glyph.screen()[1].trim(), "1 │ line 1");

    glyph.scroll_down(20, 10);
    glyph.wait_for_text("31 │ line 31", WAIT);
    assert_eq!(glyph.screen()[1].trim(), "4 │ line 4");
    glyph.scroll_down(20, 10);
    glyph.wait_for_text("34 │ line 34", WAIT);
    assert_eq!(glyph.screen()[1].trim(), "7 │ line 7");
    wait_for_position(&glyph, 1, 1);

    glyph.scroll_up(20, 10);
    glyph.wait_for_text_gone("34 │ line 34", WAIT);
    assert_eq!(glyph.screen()[1].trim(), "4 │ line 4");
    wait_for_position(&glyph, 1, 1);

    // The cursor never moved: typing lands on line 1 and brings it back into view.
    glyph.type_text("x");
    glyph.wait_for_text("1 │ xline 1", WAIT);
    wait_for_position(&glyph, 1, 2);
}
