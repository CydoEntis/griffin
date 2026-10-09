mod harness;

use std::fs;
use std::time::Duration;

use harness::Tome;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);

/// The screen row of buffer line `line` (1-based) while the view is at the top:
/// the editor starts below the three rows of the tab header.
fn row(line: u16) -> u16 {
    line + 2
}

/// Whether line `line`'s gutter starts with a breakpoint's `●`.
fn has_breakpoint(screen: &[String], line: u16) -> bool {
    screen[usize::from(row(line))].starts_with('●')
}

/// Waits until exactly the lines in `lines` (1-based, of the first `of`) show a
/// breakpoint.
fn wait_for_breakpoints(tome: &Tome, lines: &[u16], of: u16) {
    tome.wait_for_screen(&format!("breakpoints on {lines:?}"), WAIT, |screen| {
        (1..=of).all(|line| has_breakpoint(screen, line) == lines.contains(&line))
    });
}

/// Tome in a scratch folder holding `f.txt` with three lines, open in a tab.
fn open() -> (tempfile::TempDir, Tome) {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join("f.txt"), "one\ntwo\nthree\n").expect("write fixture");
    let tome = Tome::spawn_in(dir.path(), &["f.txt"]);
    tome.wait_for_text("Ln 1, Col 1", START);
    tome.wait_for_text("1  one", START);
    (dir, tome)
}

/// Runs the cast command titled `title` from the Ctrl+P palette.
fn cast(tome: &mut Tome, title: &str) {
    tome.send_keys("ctrl+p");
    tome.wait_for_text("cast", WAIT);
    tome.type_text(&format!(">{title}"));
    tome.wait_for_text(title, WAIT);
    tome.send_keys("enter");
    tome.wait_for_text_gone("cast · commands", WAIT);
}

#[test]
fn f9_toggles_the_cursor_lines_breakpoint_and_clear_breakpoints_takes_them_all() {
    let (_dir, mut tome) = open();
    tome.send_keys("f9");
    wait_for_breakpoints(&tome, &[1], 3);
    tome.send_keys("f9");
    wait_for_breakpoints(&tome, &[], 3);

    tome.send_keys("down");
    tome.wait_for_text("Ln 2, Col 1", WAIT);
    tome.send_keys("f9");
    wait_for_breakpoints(&tome, &[2], 3);
    tome.send_keys("down");
    tome.wait_for_text("Ln 3, Col 1", WAIT);
    cast(&mut tome, "Toggle breakpoint");
    wait_for_breakpoints(&tome, &[2, 3], 3);

    cast(&mut tome, "Clear breakpoints");
    wait_for_breakpoints(&tome, &[], 3);
}

#[test]
fn a_click_in_the_mark_cell_toggles_a_breakpoint_and_the_numbers_place_the_cursor() {
    let (_dir, mut tome) = open();
    tome.click(0, row(2));
    wait_for_breakpoints(&tome, &[2], 3);
    assert!(
        tome.screen()
            .iter()
            .any(|line| line.contains("Ln 1, Col 1"))
    );

    // On the line number: the cursor goes to the line's start, no breakpoint.
    tome.click(2, row(3));
    tome.wait_for_text("Ln 3, Col 1", WAIT);
    wait_for_breakpoints(&tome, &[2], 3);

    tome.click(0, row(2));
    wait_for_breakpoints(&tome, &[], 3);
}

#[test]
fn breakpoints_survive_closing_the_tab_and_move_with_lines_above() {
    let (dir, mut tome) = open();
    tome.send_keys("down");
    tome.wait_for_text("Ln 2, Col 1", WAIT);
    tome.send_keys("f9");
    wait_for_breakpoints(&tome, &[2], 3);

    // A new line above pushes the breakpoint down with `two`.
    tome.send_keys("up");
    tome.wait_for_text("Ln 1, Col 1", WAIT);
    tome.send_keys("enter");
    tome.wait_for_text("3  two", WAIT);
    wait_for_breakpoints(&tome, &[3], 4);
    tome.send_keys("ctrl+s");
    let path = dir.path().join("f.txt");
    tome.wait_for_files("the save", WAIT, || {
        fs::read_to_string(&path).is_ok_and(|text| text == "\none\ntwo\nthree\n")
    });

    tome.send_keys("ctrl+w");
    tome.wait_for_text("no file open", WAIT);

    tome.send_keys("ctrl+p");
    tome.wait_for_text("cast", WAIT);
    tome.type_text("f.txt");
    tome.wait_for_text("f.txt  ", WAIT);
    tome.send_keys("enter");
    tome.wait_for_text("3  two", WAIT);
    wait_for_breakpoints(&tome, &[3], 4);
}
