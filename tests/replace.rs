mod harness;

use std::time::Duration;

use harness::{COLS, Griffin, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// Three `foo`s: `one foo`, `two foo three`, `foo`.
const FIND: &str = "tests/fixtures/find.txt";
/// The find bar sits on the row above the status line.
const BAR_ROW: u16 = ROWS - 2;
/// The Replace field takes the right half of the bar.
const REPLACE_X: u16 = COLS / 2;

fn open(path: &str) -> Griffin {
    let griffin = Griffin::spawn(&[path]);
    griffin.wait_for_text("Ln 1, Col 1", START);
    griffin.wait_for_text("3 │ foo", WAIT);
    griffin
}

fn bar(griffin: &Griffin) -> String {
    griffin.screen()[usize::from(BAR_ROW)].clone()
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

/// Ctrl+R, `find` in the Find field, Tab, `with` in the Replace field.
fn replace(griffin: &mut Griffin, find: &str, with: &str) {
    griffin.send_keys("ctrl+r");
    griffin.wait_for_text("Replace:", WAIT);
    griffin.type_text(find);
    griffin.wait_for_text(&format!("Find: {find}"), WAIT);
    // A key and quick typing after it can arrive as one paste on Windows, so wait
    // for each key to land before the next.
    griffin.send_keys("tab");
    griffin.wait_for_cursor(REPLACE_X + 9, BAR_ROW, WAIT);
    griffin.type_text(with);
    griffin.wait_for_text(&format!("Replace: {with}"), WAIT);
}

#[test]
fn ctrl_r_shows_find_and_replace_fields_and_tab_moves_between_them() {
    let mut griffin = open(FIND);
    griffin.send_keys("ctrl+r");
    griffin.wait_for_text("Replace:", WAIT);
    let row = bar(&griffin);
    assert!(row.starts_with("Find:"), "{row:?}");
    assert_eq!(griffin.text_col(BAR_ROW, "Replace: "), Some(REPLACE_X));
    // Typing starts in Find.
    griffin.wait_for_cursor(6, BAR_ROW, WAIT);
    griffin.type_text("foo");
    wait_for_count(&griffin, "1/3");
    griffin.wait_for_cursor(9, BAR_ROW, WAIT);
    // The field stays put while the count changes width.
    assert_eq!(griffin.text_col(BAR_ROW, "Replace: "), Some(REPLACE_X));

    griffin.send_keys("tab");
    griffin.wait_for_cursor(REPLACE_X + 9, BAR_ROW, WAIT);
    griffin.type_text("xy");
    griffin.wait_for_text("Replace: xy", WAIT);
    griffin.wait_for_cursor(REPLACE_X + 11, BAR_ROW, WAIT);
    // Typing in Replace leaves the pattern and its matches alone.
    assert!(bar(&griffin).starts_with("Find: foo "));
    wait_for_count(&griffin, "1/3");

    griffin.send_keys("tab");
    griffin.wait_for_cursor(9, BAR_ROW, WAIT);
    griffin.type_text("x");
    wait_for_count(&griffin, "0/0");
}

#[test]
fn enter_in_replace_replaces_the_current_match_and_moves_to_the_next() {
    let mut griffin = open(FIND);
    replace(&mut griffin, "foo", "bar");
    wait_for_count(&griffin, "1/3");

    griffin.send_keys("enter");
    griffin.wait_for_text("1 │ one bar", WAIT);
    wait_for_count(&griffin, "1/2");
    wait_for_position(&griffin, 2, 5);
    griffin.wait_for_reversed(1, "", WAIT);
    griffin.wait_for_text("2 │ two foo three", WAIT);

    griffin.send_keys("enter");
    griffin.wait_for_text("2 │ two bar three", WAIT);
    wait_for_count(&griffin, "1/1");
    wait_for_position(&griffin, 3, 1);
    griffin.wait_for_text("3 │ foo", WAIT);
    // The buffer has unsaved changes now.
    griffin.wait_for_text("find.txt ●", WAIT);
}

#[test]
fn alt_a_replaces_all_reports_the_count_and_undoes_in_one_step() {
    let mut griffin = open(FIND);
    replace(&mut griffin, "foo", "quux");
    wait_for_count(&griffin, "1/3");

    griffin.send_keys("alt+a");
    griffin.wait_for_text("Replaced 3", WAIT);
    griffin.wait_for_text("1 │ one quux", WAIT);
    griffin.wait_for_text("2 │ two quux three", WAIT);
    griffin.wait_for_text("3 │ quux", WAIT);
    wait_for_count(&griffin, "0/0");

    griffin.send_keys("esc");
    griffin.wait_for_text_gone("Find:", WAIT);
    griffin.send_keys("ctrl+z");
    griffin.wait_for_text("1 │ one foo", WAIT);
    griffin.wait_for_text("2 │ two foo three", WAIT);
    griffin.wait_for_text("3 │ foo", WAIT);
}

#[test]
fn regex_replace_expands_groups() {
    let mut griffin = open(FIND);
    replace(&mut griffin, r"(\w+) foo", "$1-baz");
    // Literal text until regex is on.
    wait_for_count(&griffin, "0/0");
    griffin.send_keys("alt+r");
    wait_for_count(&griffin, "1/2");

    griffin.send_keys("alt+a");
    griffin.wait_for_text("Replaced 2", WAIT);
    griffin.wait_for_text("1 │ one-baz", WAIT);
    griffin.wait_for_text("2 │ two-baz three", WAIT);
    griffin.wait_for_text("3 │ foo", WAIT);
}
