mod harness;

use std::time::Duration;

use harness::{Griffin, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);

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

/// Waits until screen row `row` reads `text` after the gutter.
fn wait_for_row(griffin: &Griffin, row: u16, number: usize, text: &str) {
    let expected = format!("{number} │ {text}");
    griffin.wait_for_text(&expected, WAIT);
    let line = griffin.screen()[usize::from(row)].clone();
    assert_eq!(line.trim(), expected.trim_end(), "{:#?}", griffin.screen());
}

/// An untitled buffer, with the starting cell of its text.
fn start_empty(config: &str) -> (Griffin, u16) {
    let griffin = Griffin::spawn_with_config(config, &[]);
    griffin.wait_for_text("Ln 1, Col 1", START);
    griffin.wait_for_text("1 │", WAIT);
    // Text starts one cell after the gutter's divider. The terminal cursor moves
    // there only after the frame is drawn, so wait for it rather than read it.
    let divider = griffin
        .text_col(0, "│")
        .unwrap_or_else(|| panic!("no gutter: {:#?}", griffin.screen()));
    let x = divider + 2;
    griffin.wait_for_cursor(x, 0, WAIT);
    (griffin, x)
}

fn assert_one_line(griffin: &Griffin) {
    let screen = griffin.screen();
    let second = screen.iter().any(|row| row.trim_start().starts_with("2 "));
    assert!(!second, "lines should have joined: {screen:#?}");
}

fn quit(griffin: &mut Griffin) {
    griffin.send_keys("ctrl+q");
    let status = griffin.wait_exit(WAIT);
    assert!(status.success(), "griffin exited with {status:?}");
}

#[test]
fn typing_inserts() {
    let (mut griffin, x) = start_empty("");

    griffin.type_text("hello");
    wait_for_row(&griffin, 0, 1, "hello");
    wait_for_position(&griffin, 1, 6);
    griffin.wait_for_cursor(x + 5, 0, WAIT);

    // Typing in the middle pushes the rest along.
    griffin.send_keys("left");
    griffin.send_keys("left");
    wait_for_position(&griffin, 1, 4);
    griffin.type_text("p 日");
    wait_for_row(&griffin, 0, 1, "help 日lo");
    wait_for_position(&griffin, 1, 7);
    // The wide character fills two cells.
    griffin.wait_for_cursor(x + 7, 0, WAIT);

    quit(&mut griffin);
}

#[test]
fn enter_tab_and_typing_match_the_visual_check() {
    let (mut griffin, x) = start_empty("");

    griffin.type_text("fn x() {");
    wait_for_row(&griffin, 0, 1, "fn x() {");
    griffin.send_keys("enter");
    wait_for_position(&griffin, 2, 1);
    griffin.send_keys("tab");
    wait_for_position(&griffin, 2, 5);
    griffin.type_text("y");
    wait_for_position(&griffin, 2, 6);
    wait_for_row(&griffin, 1, 2, "    y");
    wait_for_row(&griffin, 0, 1, "fn x() {");
    griffin.wait_for_cursor(x + 5, 1, WAIT);

    // Enter on an indented line keeps the indent.
    griffin.send_keys("enter");
    wait_for_position(&griffin, 3, 5);
    griffin.type_text("z");
    wait_for_row(&griffin, 2, 3, "    z");

    quit(&mut griffin);
}

#[test]
fn backspace_and_delete_join_lines() {
    let (mut griffin, _) = start_empty("");

    griffin.type_text("ab");
    griffin.send_keys("enter");
    griffin.type_text("cd");
    wait_for_row(&griffin, 1, 2, "cd");

    // Backspace at column 0 joins with the line above.
    griffin.send_keys("home");
    wait_for_position(&griffin, 2, 1);
    griffin.send_keys("backspace");
    wait_for_row(&griffin, 0, 1, "abcd");
    wait_for_position(&griffin, 1, 3);
    assert_one_line(&griffin);

    griffin.send_keys("backspace");
    wait_for_row(&griffin, 0, 1, "acd");
    griffin.send_keys("delete");
    wait_for_row(&griffin, 0, 1, "ad");
    wait_for_position(&griffin, 1, 2);

    // Delete at line end joins with the (empty) line below.
    griffin.send_keys("end");
    griffin.send_keys("enter");
    wait_for_position(&griffin, 2, 1);
    griffin.send_keys("left");
    wait_for_position(&griffin, 1, 3);
    griffin.send_keys("delete");
    // Row 0 already read `ad`; typing proves the delete was handled first.
    griffin.type_text("x");
    wait_for_row(&griffin, 0, 1, "adx");
    assert_one_line(&griffin);

    // Nothing to delete at either edge of the document.
    griffin.send_keys("ctrl+end");
    griffin.send_keys("delete");
    griffin.send_keys("ctrl+home");
    griffin.send_keys("backspace");
    griffin.type_text("!");
    wait_for_row(&griffin, 0, 1, "!adx");

    quit(&mut griffin);
}

#[test]
fn tab_inserts_a_tab_character_when_spaces_are_off() {
    let (mut griffin, x) = start_empty("[editor]\ninsert_spaces = false\ntab_width = 8\n");

    griffin.type_text("ab");
    griffin.send_keys("tab");
    // One char, drawn as far as the next 8-column stop.
    wait_for_position(&griffin, 1, 4);
    griffin.wait_for_cursor(x + 8, 0, WAIT);
    griffin.type_text("c");
    wait_for_row(&griffin, 0, 1, "ab      c");

    quit(&mut griffin);
}
