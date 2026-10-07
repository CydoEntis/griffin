mod harness;

use std::time::Duration;

use harness::{Glyph, ROWS};

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

fn open(path: &str) -> Glyph {
    let glyph = Glyph::spawn(&[path]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph
}

fn row(glyph: &Glyph, row: u16) -> String {
    glyph.screen()[usize::from(row)].clone()
}

/// Waits for the find bar to end with `count`, as `n/m` or an error.
fn wait_for_count(glyph: &Glyph, count: &str) {
    glyph.wait_for_screen(&format!("the find bar to show {count:?}"), WAIT, |lines| {
        let bar = &lines[usize::from(BAR_ROW)];
        bar.starts_with("Find:") && bar.ends_with(&format!("  {count}"))
    });
}

/// Waits for the status line to end with `Ln <line>, Col <col>`.
fn wait_for_position(glyph: &Glyph, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    glyph.wait_for_screen(&position, WAIT, |lines| {
        lines[usize::from(ROWS - 1)].ends_with(&position)
    });
}

fn find(glyph: &mut Glyph, text: &str) {
    glyph.send_keys("ctrl+f");
    glyph.wait_for_text("Find:", WAIT);
    glyph.type_text(text);
    // A prefix of the text may already show the same count, so wait for all of it.
    glyph.wait_for_text(&format!("Find: {text}"), WAIT);
}

#[test]
fn ctrl_f_highlights_every_match_as_you_type_and_jumps_to_the_first() {
    let mut glyph = open(FIND);
    glyph.wait_for_text("3 │ foo", WAIT);
    find(&mut glyph, "foo");
    wait_for_count(&glyph, "1/3");
    assert!(row(&glyph, BAR_ROW).starts_with("Find: foo"));
    // The bar sits above the status line, which stays on the last row.
    assert!(row(&glyph, ROWS - 1).starts_with(" ✦ glyph"));
    // All three matches have the selection colours, and nothing else does.
    glyph.wait_for_reversed(1, "foo", WAIT);
    glyph.wait_for_reversed(2, "foo", WAIT);
    glyph.wait_for_reversed(3, "foo", WAIT);
    assert_eq!(glyph.text_col(1, "foo"), Some(TEXT_X + 4));
    wait_for_position(&glyph, 1, 5);
    // The terminal cursor is in the bar, after the text.
    glyph.wait_for_cursor(9, BAR_ROW, WAIT);

    // Matches update while typing: only line 2 has `foo` before a space.
    glyph.type_text(" ");
    wait_for_count(&glyph, "1/1");
    glyph.wait_for_reversed(1, "", WAIT);
    glyph.wait_for_reversed(3, "", WAIT);
    glyph.wait_for_reversed(2, "foo ", WAIT);
    wait_for_position(&glyph, 2, 5);
    glyph.send_keys("backspace");
    wait_for_count(&glyph, "1/3");
}

#[test]
fn ctrl_f_starts_from_the_selection() {
    let mut glyph = open(FIND);
    glyph.wait_for_text("3 │ foo", WAIT);
    glyph.send_keys("down");
    for _ in 0..3 {
        glyph.send_keys("shift+right");
    }
    glyph.wait_for_reversed(2, "two", WAIT);
    glyph.send_keys("ctrl+f");
    wait_for_count(&glyph, "1/1");
    assert!(row(&glyph, BAR_ROW).starts_with("Find: two"));
    glyph.wait_for_reversed(2, "two", WAIT);
    wait_for_position(&glyph, 2, 1);
}

#[test]
fn enter_and_shift_enter_step_through_matches_wrapping() {
    let mut glyph = open(FIND);
    find(&mut glyph, "foo");
    wait_for_count(&glyph, "1/3");
    wait_for_position(&glyph, 1, 5);

    glyph.send_keys("enter");
    wait_for_count(&glyph, "2/3");
    wait_for_position(&glyph, 2, 5);
    glyph.send_keys("enter");
    wait_for_count(&glyph, "3/3");
    wait_for_position(&glyph, 3, 1);
    glyph.send_keys("enter");
    wait_for_count(&glyph, "1/3");
    wait_for_position(&glyph, 1, 5);

    glyph.send_keys("shift+enter");
    wait_for_count(&glyph, "3/3");
    wait_for_position(&glyph, 3, 1);
    glyph.send_keys("shift+enter");
    wait_for_count(&glyph, "2/3");
    wait_for_position(&glyph, 2, 5);
    // Enter in the bar never edits the buffer: still three lines plus the empty
    // one after the final line break.
    glyph.wait_for_text("3 │ foo", WAIT);
    assert!(!glyph.screen().join("\n").contains("5 │"));
}

#[test]
fn alt_c_toggles_case_and_alt_r_toggles_regex_and_a_bad_regex_says_so() {
    let mut glyph = open(FIND_CASE);
    glyph.wait_for_text("2 │ foooo", WAIT);
    find(&mut glyph, "foo");
    wait_for_count(&glyph, "1/4");
    glyph.wait_for_reversed(1, "fooFooFOO", WAIT);
    glyph.wait_for_fg(BAR_ROW, ACCENT, "", WAIT);

    glyph.send_keys("alt+c");
    wait_for_count(&glyph, "1/2");
    glyph.wait_for_fg(BAR_ROW, ACCENT, "Aa", WAIT);
    glyph.wait_for_reversed(1, "foo", WAIT);
    glyph.wait_for_reversed(2, "foo", WAIT);
    glyph.send_keys("alt+c");
    wait_for_count(&glyph, "1/4");
    glyph.wait_for_fg(BAR_ROW, ACCENT, "", WAIT);

    // `fo+` is literal text until regex is on.
    for _ in 0..3 {
        glyph.send_keys("backspace");
    }
    glyph.type_text("fo+");
    wait_for_count(&glyph, "0/0");
    glyph.wait_for_reversed(1, "", WAIT);
    glyph.send_keys("alt+r");
    wait_for_count(&glyph, "1/4");
    glyph.wait_for_fg(BAR_ROW, ACCENT, ".*", WAIT);
    glyph.wait_for_reversed(2, "foooo", WAIT);

    // An unclosed group is an error, with nothing highlighted.
    glyph.type_text("(");
    wait_for_count(&glyph, "invalid regex");
    glyph.wait_for_reversed(1, "", WAIT);
    glyph.wait_for_reversed(2, "", WAIT);
    glyph.send_keys("backspace");
    wait_for_count(&glyph, "1/4");
}

#[test]
fn esc_closes_the_bar_and_keeps_the_cursor_on_the_current_match() {
    let mut glyph = open(FIND);
    find(&mut glyph, "foo");
    wait_for_count(&glyph, "1/3");
    glyph.send_keys("enter");
    wait_for_count(&glyph, "2/3");

    glyph.send_keys("esc");
    glyph.wait_for_text_gone("Find:", WAIT);
    glyph.wait_for_reversed(1, "", WAIT);
    glyph.wait_for_reversed(2, "", WAIT);
    wait_for_position(&glyph, 2, 5);
    glyph.wait_for_cursor(TEXT_X + 4, 2, WAIT);
    // Typing goes into the buffer again, at the match.
    glyph.type_text("x");
    glyph.wait_for_text("2 │ two xfoo three", WAIT);
}
