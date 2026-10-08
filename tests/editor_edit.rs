mod harness;

use std::time::Duration;

use harness::{Glyph, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// The editor's first row, below the tab bar.
const TOP: u16 = 1;

/// Waits for the status line to show `Ln <line>, Col <col>` (see `shows_position`).
fn wait_for_position(glyph: &Glyph, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    glyph.wait_for_text(&position, WAIT);
    let status = glyph.screen()[usize::from(ROWS - 1)].clone();
    assert!(
        harness::shows_position(&status, &position),
        "status line {status:?} should show {position:?}"
    );
}

/// Waits until editor row `row` (0 is the first, on screen row `TOP`) reads
/// `text` after the gutter.
fn wait_for_row(glyph: &Glyph, row: u16, number: usize, text: &str) {
    let expected = format!("{number} │ {text}");
    glyph.wait_for_text(&expected, WAIT);
    let line = glyph.screen()[usize::from(TOP + row)].clone();
    assert_eq!(line.trim(), expected.trim_end(), "{:#?}", glyph.screen());
}

/// An untitled buffer, with the starting cell of its text.
fn start_empty(config: &str) -> (Glyph, u16) {
    let glyph = Glyph::spawn_with_config(config, &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph.wait_for_text("1 │", WAIT);
    // Text starts one cell after the gutter's divider. The terminal cursor moves
    // there only after the frame is drawn, so wait for it rather than read it.
    let divider = glyph
        .text_col(TOP, "│")
        .unwrap_or_else(|| panic!("no gutter: {:#?}", glyph.screen()));
    let x = divider + 2;
    glyph.wait_for_cursor(x, TOP, WAIT);
    (glyph, x)
}

fn assert_one_line(glyph: &Glyph) {
    let screen = glyph.screen();
    let second = screen.iter().any(|row| row.trim_start().starts_with("2 "));
    assert!(!second, "lines should have joined: {screen:#?}");
}

/// Every test here edits an untitled buffer, so Ctrl+Q asks first; discard.
fn quit(glyph: &mut Glyph) {
    glyph.send_keys("ctrl+q");
    glyph.wait_for_text("has unsaved changes", WAIT);
    glyph.type_text("d");
    let status = glyph.wait_exit(WAIT);
    assert!(status.success(), "glyph exited with {status:?}");
}

#[test]
fn typing_inserts() {
    let (mut glyph, x) = start_empty("");

    glyph.type_text("hello");
    wait_for_row(&glyph, 0, 1, "hello");
    wait_for_position(&glyph, 1, 6);
    glyph.wait_for_cursor(x + 5, TOP, WAIT);

    // Typing in the middle pushes the rest along.
    glyph.send_keys("left");
    glyph.send_keys("left");
    wait_for_position(&glyph, 1, 4);
    glyph.type_text("p 日");
    wait_for_row(&glyph, 0, 1, "help 日lo");
    wait_for_position(&glyph, 1, 7);
    // The wide character fills two cells.
    glyph.wait_for_cursor(x + 7, TOP, WAIT);

    quit(&mut glyph);
}

#[test]
fn enter_tab_and_typing_match_the_visual_check() {
    let (mut glyph, x) = start_empty("");

    glyph.type_text("fn x() {");
    wait_for_row(&glyph, 0, 1, "fn x() {");
    glyph.send_keys("enter");
    wait_for_position(&glyph, 2, 1);
    glyph.send_keys("tab");
    wait_for_position(&glyph, 2, 5);
    glyph.type_text("y");
    wait_for_position(&glyph, 2, 6);
    wait_for_row(&glyph, 1, 2, "    y");
    wait_for_row(&glyph, 0, 1, "fn x() {");
    glyph.wait_for_cursor(x + 5, TOP + 1, WAIT);

    // Enter on an indented line keeps the indent.
    glyph.send_keys("enter");
    wait_for_position(&glyph, 3, 5);
    glyph.type_text("z");
    wait_for_row(&glyph, 2, 3, "    z");

    quit(&mut glyph);
}

#[test]
fn backspace_and_delete_join_lines() {
    let (mut glyph, _) = start_empty("");

    glyph.type_text("ab");
    glyph.send_keys("enter");
    glyph.type_text("cd");
    wait_for_row(&glyph, 1, 2, "cd");

    // Backspace at column 0 joins with the line above.
    glyph.send_keys("home");
    wait_for_position(&glyph, 2, 1);
    glyph.send_keys("backspace");
    wait_for_row(&glyph, 0, 1, "abcd");
    wait_for_position(&glyph, 1, 3);
    assert_one_line(&glyph);

    glyph.send_keys("backspace");
    wait_for_row(&glyph, 0, 1, "acd");
    glyph.send_keys("delete");
    wait_for_row(&glyph, 0, 1, "ad");
    wait_for_position(&glyph, 1, 2);

    // Delete at line end joins with the (empty) line below.
    glyph.send_keys("end");
    glyph.send_keys("enter");
    wait_for_position(&glyph, 2, 1);
    glyph.send_keys("left");
    wait_for_position(&glyph, 1, 3);
    glyph.send_keys("delete");
    // Row 0 already read `ad`; typing proves the delete was handled first.
    glyph.type_text("x");
    wait_for_row(&glyph, 0, 1, "adx");
    assert_one_line(&glyph);

    // Nothing to delete at either edge of the document.
    glyph.send_keys("ctrl+end");
    glyph.send_keys("delete");
    glyph.send_keys("ctrl+home");
    glyph.send_keys("backspace");
    glyph.type_text("!");
    wait_for_row(&glyph, 0, 1, "!adx");

    quit(&mut glyph);
}

#[test]
fn tab_inserts_a_tab_character_when_spaces_are_off() {
    let (mut glyph, x) = start_empty("[editor]\ninsert_spaces = false\ntab_width = 8\n");

    glyph.type_text("ab");
    glyph.send_keys("tab");
    // One char, drawn as far as the next 8-column stop.
    wait_for_position(&glyph, 1, 4);
    glyph.wait_for_cursor(x + 8, TOP, WAIT);
    glyph.type_text("c");
    wait_for_row(&glyph, 0, 1, "ab      c");

    quit(&mut glyph);
}
