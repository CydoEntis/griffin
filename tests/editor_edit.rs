mod harness;

use std::time::Duration;

use harness::{ROWS, Tome};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// The editor's first row, below the tab header.
const TOP: u16 = 3;

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

/// Waits until editor row `row` (0 is the first, on screen row `TOP`) reads
/// `text` after the gutter.
fn wait_for_row(tome: &Tome, row: u16, number: usize, text: &str) {
    let expected = format!("{number}  {text}");
    tome.wait_for_text(&expected, WAIT);
    let line = tome.screen()[usize::from(TOP + row)].clone();
    assert_eq!(line.trim(), expected.trim_end(), "{:#?}", tome.screen());
}

/// An untitled buffer, with the starting cell of its text.
fn start_empty(config: &str) -> (Tome, u16) {
    let mut tome = Tome::spawn_with_config(config, &[]);
    tome.wait_for_text("Open directory", START);
    // Leave the splash for the untitled buffer under it.
    tome.send_keys("ctrl+n");
    tome.wait_for_text("   1", WAIT);
    // Text starts two blank cells after the line number. The terminal cursor
    // moves there only after the frame is drawn, so wait for it rather than read
    // it.
    let number = tome
        .text_col(TOP, "1")
        .unwrap_or_else(|| panic!("no gutter: {:#?}", tome.screen()));
    let x = number + 3;
    tome.wait_for_cursor(x, TOP, WAIT);
    (tome, x)
}

fn assert_one_line(tome: &Tome) {
    let screen = tome.screen();
    let second = screen.iter().any(|row| row.trim_start().starts_with("2 "));
    assert!(!second, "lines should have joined: {screen:#?}");
}

/// Every test here edits an untitled buffer, so Ctrl+Q asks first; discard.
fn quit(tome: &mut Tome) {
    tome.send_keys("ctrl+q");
    tome.wait_for_text("has unsaved changes", WAIT);
    tome.type_text("d");
    let status = tome.wait_exit(WAIT);
    assert!(status.success(), "tome exited with {status:?}");
}

#[test]
fn typing_inserts() {
    let (mut tome, x) = start_empty("");

    tome.type_text("hello");
    wait_for_row(&tome, 0, 1, "hello");
    wait_for_position(&tome, 1, 6);
    tome.wait_for_cursor(x + 5, TOP, WAIT);

    // Typing in the middle pushes the rest along.
    tome.send_keys("left");
    tome.send_keys("left");
    wait_for_position(&tome, 1, 4);
    tome.type_text("p 日");
    wait_for_row(&tome, 0, 1, "help 日lo");
    wait_for_position(&tome, 1, 7);
    // The wide character fills two cells.
    tome.wait_for_cursor(x + 7, TOP, WAIT);

    quit(&mut tome);
}

#[test]
fn enter_tab_and_typing_match_the_visual_check() {
    // R6's plain Enter, with no closer for the `{` to push down.
    let (mut tome, x) = start_empty("[editor]\nauto_pairs = false\n");

    tome.type_text("fn x() {");
    wait_for_row(&tome, 0, 1, "fn x() {");
    tome.send_keys("enter");
    wait_for_position(&tome, 2, 1);
    tome.send_keys("tab");
    wait_for_position(&tome, 2, 5);
    tome.type_text("y");
    wait_for_position(&tome, 2, 6);
    wait_for_row(&tome, 1, 2, "    y");
    wait_for_row(&tome, 0, 1, "fn x() {");
    tome.wait_for_cursor(x + 5, TOP + 1, WAIT);

    // Enter on an indented line keeps the indent.
    tome.send_keys("enter");
    wait_for_position(&tome, 3, 5);
    tome.type_text("z");
    wait_for_row(&tome, 2, 3, "    z");

    quit(&mut tome);
}

#[test]
fn brackets_close_themselves() {
    let (mut tome, x) = start_empty("");

    tome.type_text("foo(");
    wait_for_row(&tome, 0, 1, "foo()");
    wait_for_position(&tome, 1, 5);
    tome.wait_for_cursor(x + 4, TOP, WAIT);

    // The typed closer steps over the one Tome put in.
    tome.type_text("bar)");
    wait_for_row(&tome, 0, 1, "foo(bar)");
    wait_for_position(&tome, 1, 9);
    tome.wait_for_cursor(x + 8, TOP, WAIT);

    // Before a word an opener comes alone.
    tome.send_keys("home");
    tome.type_text("[");
    wait_for_row(&tome, 0, 1, "[foo(bar)");

    quit(&mut tome);
}

#[test]
fn backspace_in_an_empty_pair_deletes_both() {
    let (mut tome, _) = start_empty("");

    tome.type_text("x[");
    wait_for_row(&tome, 0, 1, "x[]");
    tome.send_keys("backspace");
    // "x" alone is also a prefix of the old row, so wait for the cursor first.
    wait_for_position(&tome, 1, 2);
    wait_for_row(&tome, 0, 1, "x");

    quit(&mut tome);
}

#[test]
fn enter_in_a_pair_opens_an_indented_line() {
    let (mut tome, x) = start_empty("");

    tome.type_text("if x {");
    wait_for_row(&tome, 0, 1, "if x {}");
    tome.send_keys("enter");
    wait_for_position(&tome, 2, 5);
    tome.type_text("y");
    wait_for_row(&tome, 1, 2, "    y");
    wait_for_row(&tome, 0, 1, "if x {");
    wait_for_row(&tome, 2, 3, "}");
    tome.wait_for_cursor(x + 5, TOP + 1, WAIT);

    quit(&mut tome);
}

#[test]
fn auto_pairs_off_types_brackets_alone() {
    let (mut tome, _) = start_empty("[editor]\nauto_pairs = false\n");

    tome.type_text("foo(");
    wait_for_row(&tome, 0, 1, "foo(");
    wait_for_position(&tome, 1, 5);
    tome.type_text(")");
    wait_for_row(&tome, 0, 1, "foo()");
    wait_for_position(&tome, 1, 6);

    quit(&mut tome);
}

#[test]
fn backspace_and_delete_join_lines() {
    let (mut tome, _) = start_empty("");

    tome.type_text("ab");
    tome.send_keys("enter");
    tome.type_text("cd");
    wait_for_row(&tome, 1, 2, "cd");

    // Backspace at column 0 joins with the line above.
    tome.send_keys("home");
    wait_for_position(&tome, 2, 1);
    tome.send_keys("backspace");
    wait_for_row(&tome, 0, 1, "abcd");
    wait_for_position(&tome, 1, 3);
    assert_one_line(&tome);

    tome.send_keys("backspace");
    wait_for_row(&tome, 0, 1, "acd");
    tome.send_keys("delete");
    wait_for_row(&tome, 0, 1, "ad");
    wait_for_position(&tome, 1, 2);

    // Delete at line end joins with the (empty) line below.
    tome.send_keys("end");
    tome.send_keys("enter");
    wait_for_position(&tome, 2, 1);
    tome.send_keys("left");
    wait_for_position(&tome, 1, 3);
    tome.send_keys("delete");
    // Row 0 already read `ad`; typing proves the delete was handled first.
    tome.type_text("x");
    wait_for_row(&tome, 0, 1, "adx");
    assert_one_line(&tome);

    // Nothing to delete at either edge of the document.
    tome.send_keys("ctrl+end");
    tome.send_keys("delete");
    tome.send_keys("ctrl+home");
    tome.send_keys("backspace");
    tome.type_text("!");
    wait_for_row(&tome, 0, 1, "!adx");

    quit(&mut tome);
}

#[test]
fn tab_inserts_a_tab_character_when_spaces_are_off() {
    let (mut tome, x) = start_empty("[editor]\ninsert_spaces = false\ntab_width = 8\n");

    tome.type_text("ab");
    tome.send_keys("tab");
    // One char, drawn as far as the next 8-column stop.
    wait_for_position(&tome, 1, 4);
    tome.wait_for_cursor(x + 8, TOP, WAIT);
    tome.type_text("c");
    wait_for_row(&tome, 0, 1, "ab      c");

    quit(&mut tome);
}
