mod harness;

use std::time::Duration;

use harness::{Griffin, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// Three `foo`s: `one foo`, `two foo three`, `foo`.
const FIND: &str = "tests/fixtures/find.txt";
/// `foo Foo FOO` then `foooo`: four `foo`s ignoring case, two matching it.
const FIND_CASE: &str = "tests/fixtures/find_case.txt";
/// The default theme's accent, hydra's lime `#c3f53c`.
const ACCENT: vt100::Color = vt100::Color::Rgb(0xc3, 0xf5, 0x3c);
/// The find bar sits on the row above the status line.
const BAR_ROW: u16 = ROWS - 2;
/// Text starts after the gutter ` 1 │ `.
const TEXT_X: u16 = 5;

fn open(path: &str) -> Griffin {
    let griffin = Griffin::spawn(&[path]);
    griffin.wait_for_text("Ln 1, Col 1", START);
    griffin
}

fn row(griffin: &Griffin, row: u16) -> String {
    griffin.screen()[usize::from(row)].clone()
}

/// Waits for the find bar to end with `count`, as `n/m` or an error.
fn wait_for_count(griffin: &Griffin, count: &str) {
    griffin.wait_for_screen(&format!("the find bar to show {count:?}"), WAIT, |lines| {
        let bar = &lines[usize::from(BAR_ROW)];
        bar.starts_with("Find:") && bar.ends_with(&format!("  {count}"))
    });
}

/// Waits for the status line to end with `Ln <line>, Col <col>`.
fn wait_for_position(griffin: &Griffin, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    griffin.wait_for_screen(&position, WAIT, |lines| {
        lines[usize::from(ROWS - 1)].ends_with(&position)
    });
}

fn find(griffin: &mut Griffin, text: &str) {
    griffin.send_keys("ctrl+f");
    griffin.wait_for_text("Find:", WAIT);
    griffin.type_text(text);
}

#[test]
fn ctrl_f_highlights_every_match_as_you_type_and_jumps_to_the_first() {
    let mut griffin = open(FIND);
    griffin.wait_for_text("3 │ foo", WAIT);
    find(&mut griffin, "foo");
    wait_for_count(&griffin, "1/3");
    assert!(row(&griffin, BAR_ROW).starts_with("Find: foo"));
    // The bar sits above the status line, which stays on the last row.
    assert!(row(&griffin, ROWS - 1).starts_with("griffin"));
    // All three matches have the selection colours, and nothing else does.
    griffin.wait_for_reversed(1, "foo", WAIT);
    griffin.wait_for_reversed(2, "foo", WAIT);
    griffin.wait_for_reversed(3, "foo", WAIT);
    assert_eq!(griffin.text_col(1, "foo"), Some(TEXT_X + 4));
    wait_for_position(&griffin, 1, 5);
    // The terminal cursor is in the bar, after the text.
    griffin.wait_for_cursor(9, BAR_ROW, WAIT);

    // Matches update while typing: only line 2 has `foo` before a space.
    griffin.type_text(" ");
    wait_for_count(&griffin, "1/1");
    griffin.wait_for_reversed(1, "", WAIT);
    griffin.wait_for_reversed(3, "", WAIT);
    griffin.wait_for_reversed(2, "foo ", WAIT);
    wait_for_position(&griffin, 2, 5);
    griffin.send_keys("backspace");
    wait_for_count(&griffin, "1/3");
}

#[test]
fn ctrl_f_starts_from_the_selection() {
    let mut griffin = open(FIND);
    griffin.wait_for_text("3 │ foo", WAIT);
    griffin.send_keys("down");
    for _ in 0..3 {
        griffin.send_keys("shift+right");
    }
    griffin.wait_for_reversed(2, "two", WAIT);
    griffin.send_keys("ctrl+f");
    wait_for_count(&griffin, "1/1");
    assert!(row(&griffin, BAR_ROW).starts_with("Find: two"));
    griffin.wait_for_reversed(2, "two", WAIT);
    wait_for_position(&griffin, 2, 1);
}

#[test]
fn enter_and_shift_enter_step_through_matches_wrapping() {
    let mut griffin = open(FIND);
    find(&mut griffin, "foo");
    wait_for_count(&griffin, "1/3");
    wait_for_position(&griffin, 1, 5);

    griffin.send_keys("enter");
    wait_for_count(&griffin, "2/3");
    wait_for_position(&griffin, 2, 5);
    griffin.send_keys("enter");
    wait_for_count(&griffin, "3/3");
    wait_for_position(&griffin, 3, 1);
    griffin.send_keys("enter");
    wait_for_count(&griffin, "1/3");
    wait_for_position(&griffin, 1, 5);

    griffin.send_keys("shift+enter");
    wait_for_count(&griffin, "3/3");
    wait_for_position(&griffin, 3, 1);
    griffin.send_keys("shift+enter");
    wait_for_count(&griffin, "2/3");
    wait_for_position(&griffin, 2, 5);
    // Enter in the bar never edits the buffer: still three lines plus the empty
    // one after the final line break.
    griffin.wait_for_text("3 │ foo", WAIT);
    assert!(!griffin.screen().join("\n").contains("5 │"));
}

#[test]
fn alt_c_toggles_case_and_alt_r_toggles_regex_and_a_bad_regex_says_so() {
    let mut griffin = open(FIND_CASE);
    griffin.wait_for_text("2 │ foooo", WAIT);
    find(&mut griffin, "foo");
    wait_for_count(&griffin, "1/4");
    griffin.wait_for_reversed(1, "fooFooFOO", WAIT);
    griffin.wait_for_fg(BAR_ROW, ACCENT, "", WAIT);

    griffin.send_keys("alt+c");
    wait_for_count(&griffin, "1/2");
    griffin.wait_for_fg(BAR_ROW, ACCENT, "Aa", WAIT);
    griffin.wait_for_reversed(1, "foo", WAIT);
    griffin.wait_for_reversed(2, "foo", WAIT);
    griffin.send_keys("alt+c");
    wait_for_count(&griffin, "1/4");
    griffin.wait_for_fg(BAR_ROW, ACCENT, "", WAIT);

    // `fo+` is literal text until regex is on.
    for _ in 0..3 {
        griffin.send_keys("backspace");
    }
    griffin.type_text("fo+");
    wait_for_count(&griffin, "0/0");
    griffin.wait_for_reversed(1, "", WAIT);
    griffin.send_keys("alt+r");
    wait_for_count(&griffin, "1/4");
    griffin.wait_for_fg(BAR_ROW, ACCENT, ".*", WAIT);
    griffin.wait_for_reversed(2, "foooo", WAIT);

    // An unclosed group is an error, with nothing highlighted.
    griffin.type_text("(");
    wait_for_count(&griffin, "invalid regex");
    griffin.wait_for_reversed(1, "", WAIT);
    griffin.wait_for_reversed(2, "", WAIT);
    griffin.send_keys("backspace");
    wait_for_count(&griffin, "1/4");
}

#[test]
fn esc_closes_the_bar_and_keeps_the_cursor_on_the_current_match() {
    let mut griffin = open(FIND);
    find(&mut griffin, "foo");
    wait_for_count(&griffin, "1/3");
    griffin.send_keys("enter");
    wait_for_count(&griffin, "2/3");

    griffin.send_keys("esc");
    griffin.wait_for_text_gone("Find:", WAIT);
    griffin.wait_for_reversed(1, "", WAIT);
    griffin.wait_for_reversed(2, "", WAIT);
    wait_for_position(&griffin, 2, 5);
    griffin.wait_for_cursor(TEXT_X + 4, 2, WAIT);
    // Typing goes into the buffer again, at the match.
    griffin.type_text("x");
    griffin.wait_for_text("2 │ two xfoo three", WAIT);
}
