mod harness;

use std::time::Duration;

use harness::{COLS, ROWS, Tome};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// Three `foo`s: `one foo`, `two foo three`, `foo`.
const FIND: &str = "tests/fixtures/find.txt";
/// The find bar sits on the row above the status line.
const BAR_ROW: u16 = ROWS - 2;
/// Find covers `[0, floor(W/2))` and the `│` rule sits at `floor(W/2)`.
const RULE_X: u16 = COLS / 2;
/// The Replace field runs from after the rule: ` Replace  ` and then its text.
const REPLACE_TEXT_X: u16 = RULE_X + 11;
/// hydra's `guide`, the rule between the fields.
const GUIDE: vt100::Color = vt100::Color::Rgb(0x17, 0x22, 0x2e);
/// hydra's `text`, the label of the field with focus.
const TEXT: vt100::Color = vt100::Color::Rgb(0xa7, 0xb4, 0xc2);
/// hydra's `muted`, the other label.
const MUTED: vt100::Color = vt100::Color::Rgb(0x71, 0x80, 0x8f);

fn open(path: &str) -> Tome {
    let tome = Tome::spawn(&[path]);
    tome.wait_for_text("Ln 1, Col 1", START);
    tome.wait_for_text("3  foo", WAIT);
    tome
}

fn bar(tome: &Tome) -> String {
    tome.screen()[usize::from(BAR_ROW)].clone()
}

/// Waits for the find bar to end with `count`, as `n/m` or an error.
fn wait_for_count(tome: &Tome, count: &str) {
    tome.wait_for_screen(&format!("the find bar to show {count:?}"), WAIT, |lines| {
        let bar = &lines[usize::from(BAR_ROW)];
        bar.starts_with(" Find  ") && bar.ends_with(&format!("  {count}"))
    });
}

/// Waits for the status line to show `Ln <line>, Col <col>` (see `shows_position`).
fn wait_for_position(tome: &Tome, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    tome.wait_for_screen(&position, WAIT, |lines| {
        harness::shows_position(&lines[usize::from(ROWS - 1)], &position)
    });
}

/// Ctrl+R, `find` in the Find field, Tab, `with` in the Replace field.
fn replace(tome: &mut Tome, find: &str, with: &str) {
    tome.send_keys("ctrl+r");
    tome.wait_for_text(" Replace  ", WAIT);
    tome.type_text(find);
    tome.wait_for_text(&format!(" Find  {find}"), WAIT);
    // A key and quick typing after it can arrive as one paste on Windows, so wait
    // for each key to land before the next.
    tome.send_keys("tab");
    tome.wait_for_cursor(REPLACE_TEXT_X, BAR_ROW, WAIT);
    tome.type_text(with);
    tome.wait_for_text(&format!(" Replace  {with}"), WAIT);
}

#[test]
fn ctrl_r_shows_find_and_replace_fields_and_tab_moves_between_them() {
    let mut tome = open(FIND);
    tome.send_keys("ctrl+r");
    tome.wait_for_text(" Replace  ", WAIT);
    let row = bar(&tome);
    assert!(row.starts_with(" Find  "), "{row:?}");
    assert_eq!(tome.text_col(BAR_ROW, "│ Replace  "), Some(RULE_X));
    assert_eq!(tome.fg_at(RULE_X, BAR_ROW), GUIDE);
    // Typing starts in Find, whose label is lit while the other is quiet.
    tome.wait_for_cursor(7, BAR_ROW, WAIT);
    assert_eq!(tome.fg_at(1, BAR_ROW), TEXT);
    assert_eq!(tome.fg_at(RULE_X + 2, BAR_ROW), MUTED);
    tome.type_text("foo");
    wait_for_count(&tome, "1/3");
    tome.wait_for_cursor(10, BAR_ROW, WAIT);
    // The field stays put while the count changes width.
    assert_eq!(tome.text_col(BAR_ROW, "│ Replace  "), Some(RULE_X));

    tome.send_keys("tab");
    tome.wait_for_cursor(REPLACE_TEXT_X, BAR_ROW, WAIT);
    // Focus moved, and the labels' colours with it.
    assert_eq!(tome.fg_at(1, BAR_ROW), MUTED);
    assert_eq!(tome.fg_at(RULE_X + 2, BAR_ROW), TEXT);
    tome.type_text("xy");
    tome.wait_for_text(" Replace  xy", WAIT);
    tome.wait_for_cursor(REPLACE_TEXT_X + 2, BAR_ROW, WAIT);
    // Typing in Replace leaves the pattern and its matches alone.
    assert!(bar(&tome).starts_with(" Find  foo "));
    wait_for_count(&tome, "1/3");

    tome.send_keys("tab");
    tome.wait_for_cursor(10, BAR_ROW, WAIT);
    tome.type_text("x");
    wait_for_count(&tome, "0/0");
}

#[test]
fn enter_in_replace_replaces_the_current_match_and_moves_to_the_next() {
    let mut tome = open(FIND);
    replace(&mut tome, "foo", "bar");
    wait_for_count(&tome, "1/3");

    tome.send_keys("enter");
    tome.wait_for_text("1  one bar", WAIT);
    wait_for_count(&tome, "1/2");
    wait_for_position(&tome, 2, 5);
    tome.wait_for_reversed(3, "", WAIT);
    tome.wait_for_text("2  two foo three", WAIT);

    tome.send_keys("enter");
    tome.wait_for_text("2  two bar three", WAIT);
    wait_for_count(&tome, "1/1");
    wait_for_position(&tome, 3, 1);
    tome.wait_for_text("3  foo", WAIT);
    // The buffer has unsaved changes now.
    tome.wait_for_text("find.txt •", WAIT);
}

#[test]
fn alt_a_replaces_all_reports_the_count_and_undoes_in_one_step() {
    let mut tome = open(FIND);
    replace(&mut tome, "foo", "quux");
    wait_for_count(&tome, "1/3");

    tome.send_keys("alt+a");
    tome.wait_for_text("Replaced 3", WAIT);
    tome.wait_for_text("1  one quux", WAIT);
    tome.wait_for_text("2  two quux three", WAIT);
    tome.wait_for_text("3  quux", WAIT);
    wait_for_count(&tome, "0/0");

    tome.send_keys("esc");
    tome.wait_for_text_gone(" Find  ", WAIT);
    tome.send_keys("ctrl+z");
    tome.wait_for_text("1  one foo", WAIT);
    tome.wait_for_text("2  two foo three", WAIT);
    tome.wait_for_text("3  foo", WAIT);
}

#[test]
fn regex_replace_expands_groups() {
    let mut tome = open(FIND);
    replace(&mut tome, r"(\w+) foo", "$1-baz");
    // Literal text until regex is on.
    wait_for_count(&tome, "0/0");
    tome.send_keys("alt+r");
    wait_for_count(&tome, "1/2");

    tome.send_keys("alt+a");
    tome.wait_for_text("Replaced 2", WAIT);
    tome.wait_for_text("1  one-baz", WAIT);
    tome.wait_for_text("2  two-baz three", WAIT);
    tome.wait_for_text("3  foo", WAIT);
}
