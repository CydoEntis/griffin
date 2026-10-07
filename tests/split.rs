mod harness;

use std::fs;
use std::time::Duration;

use harness::{Glyph, ROWS};
use tempfile::TempDir;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// With the tree hidden the two splits share 100 columns: 0..50, the `│` in
/// column 50, then 51..100.
const DIVIDER: usize = 50;
/// With the tree showing they share the 69 columns right of it: 31..65, the `│` in
/// column 65, then 66..100.
const TREE_DIVIDER: usize = 65;
const LEFT_X: u16 = 31;

/// A temp folder holding `a.txt` ("alpha") and `b.txt` ("bravo").
fn project() -> TempDir {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join("a.txt"), "alpha\n").expect("write a.txt");
    fs::write(dir.path().join("b.txt"), "bravo\n").expect("write b.txt");
    dir
}

/// `a.txt` opened on its own, so the tree is hidden.
fn open_file(dir: &TempDir) -> Glyph {
    let glyph = Glyph::spawn_in(dir.path(), &["a.txt"]);
    glyph.wait_for_text("1 │ alpha", START);
    glyph
}

/// The folder opened with the tree showing and a.txt open in the left split.
fn open_folder(dir: &TempDir) -> Glyph {
    let mut glyph = Glyph::spawn_in(dir.path(), &["."]);
    glyph.wait_for_text("b.txt", START);
    glyph.click(3, 1);
    glyph.wait_for_text("1 │ alpha", WAIT);
    glyph
}

/// The columns `from..to` of `line`, trailing spaces cut.
fn cols(line: &str, from: usize, to: usize) -> String {
    line.chars()
        .skip(from)
        .take(to.saturating_sub(from))
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// What the left and right splits show on `row`, either side of `divider`; the
/// left split starts at `left`.
fn halves(screen: &[String], row: usize, left: usize, divider: usize) -> (String, String) {
    (
        cols(&screen[row], left, divider),
        cols(&screen[row], divider + 1, usize::MAX),
    )
}

/// Whether `divider` holds a `│` on every row above the status line.
fn divided(screen: &[String], divider: usize) -> bool {
    screen[..usize::from(ROWS - 1)]
        .iter()
        .all(|line| line.chars().nth(divider) == Some('│'))
}

/// Waits for the two tab bars to read `left` and `right`.
fn wait_for_bars(glyph: &Glyph, left_x: usize, divider: usize, left: &str, right: &str) {
    glyph.wait_for_screen(&format!("tab bars {left:?} | {right:?}"), WAIT, |screen| {
        halves(screen, 0, left_x, divider) == (left.to_string(), right.to_string())
    });
}

#[test]
fn alt_v_shows_two_editors_side_by_side_and_alt_v_again_closes_the_right_one() {
    let dir = project();
    let mut glyph = open_file(&dir);
    glyph.send_keys("alt+v");

    // The visual check: two editors of about equal width split by a `│`, each with
    // its own tab bar, and only the focused (new, right) split's tab highlighted.
    glyph.wait_for_screen("two splits on a.txt", WAIT, |screen| {
        divided(screen, DIVIDER)
            && halves(screen, 0, 0, DIVIDER) == (" a.txt".into(), " a.txt".into())
            && halves(screen, 1, 0, DIVIDER) == (" 1 │ alpha".into(), " 1 │ alpha".into())
    });
    glyph.wait_for_reversed(0, " a.txt ", WAIT);
    let cursor = glyph.cursor();
    assert!(
        usize::from(cursor.0) > DIVIDER,
        "cursor {cursor:?} should be in the right split\n{:#?}",
        glyph.screen()
    );

    // Each split has its own tabs and active tab: a new file opens on the right.
    glyph.send_keys("ctrl+n");
    wait_for_bars(&glyph, 0, DIVIDER, " a.txt", " a.txt  untitled");
    glyph.wait_for_reversed(0, " untitled ", WAIT);
    glyph.wait_for_screen("right split empty", WAIT, |screen| {
        halves(screen, 1, 0, DIVIDER) == (" 1 │ alpha".into(), " 1 │".into())
    });

    // Alt+V again: one editor, keeping the tab only the right split had.
    glyph.send_keys("alt+v");
    glyph.wait_for_screen("one editor", WAIT, |screen| {
        cols(&screen[0], 0, 100) == " a.txt  untitled"
            && screen[1].matches("alpha").count() == 1
            && !divided(screen, DIVIDER)
    });
    glyph.wait_for_reversed(0, " a.txt ", WAIT);
    glyph.send_keys("ctrl+q");
    assert!(glyph.wait_exit(WAIT).success());
}

#[test]
fn f6_cycles_tree_left_right_and_skips_the_hidden_tree() {
    let dir = project();
    let mut glyph = open_folder(&dir);
    glyph.send_keys("alt+v");
    // b.txt opens in the focused right split, which then shows it.
    glyph.click(3, 2);
    wait_for_bars(&glyph, 31, TREE_DIVIDER, " a.txt", " a.txt  b.txt");
    glyph.wait_for_text("1 │ bravo", WAIT);

    // From the right split F6 wraps to the tree, where typing edits nothing...
    glyph.send_keys("f6");
    glyph.wait_for_cursor(0, 2, WAIT);
    glyph.type_text("0");
    // ...then the left split, then the right one.
    glyph.send_keys("f6");
    glyph.type_text("1");
    glyph.wait_for_text("1 │ 1alpha", WAIT);
    glyph.send_keys("f6");
    glyph.type_text("2");
    glyph.wait_for_text("1 │ 2bravo", WAIT);
    glyph.send_keys("f6");
    glyph.wait_for_cursor(0, 2, WAIT);

    // With the tree hidden F6 goes left, right, left.
    glyph.send_keys("ctrl+b");
    glyph.send_keys("f6");
    glyph.type_text("3");
    glyph.wait_for_screen("typed on the left", WAIT, |screen| {
        halves(screen, 1, 0, DIVIDER) == (" 1 │ 13alpha".into(), " 1 │ 2bravo".into())
    });
    glyph.send_keys("f6");
    glyph.type_text("4");
    glyph.wait_for_text("1 │ 24bravo", WAIT);
    glyph.send_keys("f6");
    glyph.type_text("5");
    glyph.wait_for_text("1 │ 135alpha", WAIT);
}

#[test]
fn a_click_focuses_its_split_and_tab_and_open_actions_go_there() {
    let dir = project();
    let mut glyph = open_folder(&dir);
    glyph.send_keys("alt+v");
    wait_for_bars(&glyph, 31, TREE_DIVIDER, " a.txt", " a.txt");

    // A click in the left editor focuses it: a new tab opens there.
    glyph.click(LEFT_X + 10, 5);
    glyph.send_keys("ctrl+n");
    wait_for_bars(&glyph, 31, TREE_DIVIDER, " a.txt  untitled", " a.txt");
    glyph.wait_for_reversed(0, " untitled ", WAIT);

    // A click in the right editor focuses that one: the tree opens b.txt there.
    glyph.click(80, 5);
    glyph.wait_for_reversed(0, " a.txt ", WAIT);
    glyph.click(3, 2);
    wait_for_bars(
        &glyph,
        31,
        TREE_DIVIDER,
        " a.txt  untitled",
        " a.txt  b.txt",
    );
    glyph.wait_for_reversed(0, " b.txt ", WAIT);

    // Tab keys move along the focused split's tabs only.
    glyph.send_keys("alt+,");
    glyph.wait_for_reversed(0, " a.txt ", WAIT);
    glyph.wait_for_screen("right split back on a.txt", WAIT, |screen| {
        halves(screen, 1, 31, TREE_DIVIDER) == (" 1 │".into(), " 1 │ alpha".into())
    });
    // Ctrl+W closes the right split's tab; the left split keeps its own a.txt tab.
    glyph.send_keys("ctrl+w");
    wait_for_bars(&glyph, 31, TREE_DIVIDER, " a.txt  untitled", " b.txt");

    // A click on the left tab bar focuses the left split too.
    let a = glyph.text_col(0, "a.txt").expect("left a.txt tab");
    glyph.click(a, 0);
    glyph.wait_for_reversed(0, " a.txt ", WAIT);
    // Typing lands at the left split's own cursor, where the click below the text
    // left it on the last line.
    glyph.type_text("z");
    glyph.wait_for_text("2 │ z", WAIT);
    wait_for_bars(&glyph, 31, TREE_DIVIDER, " a.txt ●  untitled", " b.txt");
}

#[test]
fn edits_to_a_buffer_open_in_both_splits_show_in_both() {
    let dir = project();
    let mut glyph = open_file(&dir);
    glyph.send_keys("alt+v");
    glyph.wait_for_reversed(0, " a.txt ", WAIT);

    // Typed on the right, shown on both sides, both tabs marked dirty.
    glyph.send_keys("end");
    glyph.type_text("!");
    glyph.wait_for_screen("edit in both", WAIT, |screen| {
        halves(screen, 1, 0, DIVIDER) == (" 1 │ alpha!".into(), " 1 │ alpha!".into())
            && halves(screen, 0, 0, DIVIDER) == (" a.txt ●".into(), " a.txt ●".into())
    });

    // Typed on the left, at the left split's own cursor (still at the start).
    glyph.click(10, 1);
    glyph.send_keys("home");
    glyph.type_text("<");
    glyph.wait_for_screen("second edit in both", WAIT, |screen| {
        halves(screen, 1, 0, DIVIDER) == (" 1 │ <alpha!".into(), " 1 │ <alpha!".into())
    });

    // Saving from either split saves the one buffer.
    glyph.send_keys("ctrl+s");
    wait_for_bars(&glyph, 0, DIVIDER, " a.txt", " a.txt");
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).expect("read a.txt"),
        "<alpha!\n"
    );
    glyph.send_keys("ctrl+q");
    assert!(glyph.wait_exit(WAIT).success());
}
