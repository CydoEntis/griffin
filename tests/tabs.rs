mod harness;

use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{Glyph, ROWS};
use tempfile::TempDir;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const PROMPT: &str = "has unsaved changes";
/// The tree's width plus its divider: the tab bar starts in this column.
const BAR_X: u16 = 31;

/// A temp folder holding `a.txt` ("alpha") and `b.txt` ("bravo"), opened as the
/// project with the tree showing, cwd inside it.
fn open_project() -> (TempDir, Glyph) {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join("a.txt"), "alpha\n").expect("write a.txt");
    fs::write(dir.path().join("b.txt"), "bravo\n").expect("write b.txt");
    let glyph = Glyph::spawn_in(dir.path(), &["."]);
    glyph.wait_for_text("b.txt", START);
    (dir, glyph)
}

/// Opens both files from the tree (rows 1 and 2), leaving b.txt active.
fn open_both(glyph: &mut Glyph) {
    glyph.click(3, 1);
    glyph.wait_for_text("1 │ alpha", WAIT);
    glyph.click(3, 2);
    glyph.wait_for_text("1 │ bravo", WAIT);
    wait_for_tabs(glyph, " a.txt  b.txt");
}

/// Waits for the tab bar to read `text`, trailing spaces aside.
fn wait_for_tabs(glyph: &Glyph, text: &str) {
    glyph.wait_for_screen(&format!("tab bar {text:?}"), WAIT, |screen| {
        let bar: String = screen[0].chars().skip(usize::from(BAR_X)).collect();
        bar.trim_end() == text
    });
}

/// Waits for the active tab, the only reversed text on row 0, to be `text`.
fn wait_for_active(glyph: &Glyph, text: &str) {
    glyph.wait_for_reversed(0, text, WAIT);
}

fn status_line(glyph: &Glyph) -> String {
    glyph.screen()[usize::from(ROWS - 1)].clone()
}

fn read(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).expect("read file")
}

#[test]
fn row_0_shows_each_buffer_with_the_active_one_and_dirty_marks() {
    let (_dir, mut glyph) = open_project();
    // An untitled tab before anything is opened.
    wait_for_tabs(&glyph, " untitled");
    // Opening a file replaces that untouched untitled tab; a second gets its own.
    open_both(&mut glyph);
    wait_for_active(&glyph, " b.txt ");

    // The visual check: both names, the active one highlighted, the dirty one marked.
    glyph.type_text("x");
    wait_for_tabs(&glyph, " a.txt  b.txt ●");
    wait_for_active(&glyph, " b.txt ● ");
    let a = glyph.text_col(0, "a.txt").expect("a.txt tab");
    let b = glyph.text_col(0, "b.txt").expect("b.txt tab");
    assert!(BAR_X <= a && a < b, "{:#?}", glyph.screen());

    // Opening an already-open file switches to its tab instead of opening another.
    glyph.click(3, 1);
    wait_for_active(&glyph, " a.txt ");
    glyph.wait_for_text("1 │ alpha", WAIT);
    wait_for_tabs(&glyph, " a.txt  b.txt ●");
    glyph.click(3, 2);
    glyph.wait_for_text("1 │ xbravo", WAIT);
    wait_for_tabs(&glyph, " a.txt  b.txt ●");
}

#[test]
fn alt_keys_move_between_tabs_and_ctrl_w_closes_with_the_guard() {
    let (dir, mut glyph) = open_project();
    open_both(&mut glyph);
    glyph.send_keys("ctrl+n");
    wait_for_tabs(&glyph, " a.txt  b.txt  untitled");
    wait_for_active(&glyph, " untitled ");

    glyph.send_keys("alt+,");
    wait_for_active(&glyph, " b.txt ");
    glyph.send_keys("alt+.");
    wait_for_active(&glyph, " untitled ");
    // Past the end wraps round to the first.
    glyph.send_keys("alt+.");
    wait_for_active(&glyph, " a.txt ");
    glyph.wait_for_text("1 │ alpha", WAIT);
    glyph.send_keys("alt+3");
    wait_for_active(&glyph, " untitled ");
    glyph.send_keys("alt+2");
    wait_for_active(&glyph, " b.txt ");

    // A dirty tab asks first; cancel keeps it.
    glyph.type_text("y");
    wait_for_active(&glyph, " b.txt ● ");
    glyph.send_keys("ctrl+w");
    glyph.wait_for_text(PROMPT, WAIT);
    glyph.type_text("c");
    glyph.wait_for_text_gone(PROMPT, WAIT);
    wait_for_tabs(&glyph, " a.txt  b.txt ●  untitled");
    // Discard closes it without touching the file; the next tab takes over.
    glyph.send_keys("ctrl+w");
    glyph.wait_for_text(PROMPT, WAIT);
    glyph.type_text("d");
    wait_for_tabs(&glyph, " a.txt  untitled");
    wait_for_active(&glyph, " untitled ");
    assert_eq!(read(dir.path(), "b.txt"), "bravo\n");

    // Clean tabs close straight away.
    glyph.send_keys("ctrl+w");
    wait_for_tabs(&glyph, " a.txt");
    glyph.send_keys("ctrl+w");
    wait_for_tabs(&glyph, " untitled");
    glyph.send_keys("ctrl+q");
    assert!(glyph.wait_exit(WAIT).success());
}

#[test]
fn ctrl_n_then_alt_s_saves_relative_to_the_project_root() {
    let parent = tempfile::tempdir().expect("create temp dir");
    let root = parent.path().join("proj");
    fs::create_dir(&root).expect("create proj");
    fs::write(root.join("a.txt"), "alpha\n").expect("write a.txt");
    // cwd is the parent, so a path relative to the root is not one relative to cwd.
    let mut glyph = Glyph::spawn_in(parent.path(), &["proj"]);
    glyph.wait_for_text("a.txt", START);

    glyph.send_keys("ctrl+n");
    wait_for_active(&glyph, " untitled ");
    glyph.type_text("hello");
    glyph.wait_for_text("1 │ hello", WAIT);
    wait_for_active(&glyph, " untitled ● ");

    // A name another file already has changes nothing.
    glyph.send_keys("alt+s");
    glyph.wait_for_text("Save as:", WAIT);
    glyph.type_text("a.txt");
    glyph.send_keys("enter");
    glyph.wait_for_text("a.txt already exists", WAIT);
    assert_eq!(read(&root, "a.txt"), "alpha\n");

    glyph.send_keys("alt+s");
    glyph.wait_for_text("Save as:", WAIT);
    glyph.type_text("new.txt");
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("Save as:", WAIT);
    wait_for_active(&glyph, " new.txt ");
    assert_eq!(read(&root, "new.txt"), "hello");
    assert!(!parent.path().join("new.txt").exists());
    assert!(!status_line(&glyph).contains('●'));
    // The tree shows the new file.
    glyph.wait_for_text("  new.txt", WAIT);

    // Save as again offers the current path, relative to the root, and moves the
    // tab to the new name; the old file stays as it was.
    glyph.type_text("!");
    glyph.send_keys("alt+s");
    glyph.wait_for_text("Save as: new.txt", WAIT);
    for _ in 0.."new.txt".len() {
        glyph.send_keys("backspace");
    }
    glyph.type_text("renamed.txt");
    glyph.send_keys("enter");
    wait_for_active(&glyph, " renamed.txt ");
    assert_eq!(read(&root, "renamed.txt"), "hello!");
    assert_eq!(read(&root, "new.txt"), "hello");
    glyph.send_keys("ctrl+q");
    assert!(glyph.wait_exit(WAIT).success());
}

#[test]
fn click_selects_a_tab_and_middle_click_closes_it() {
    let (_dir, mut glyph) = open_project();
    open_both(&mut glyph);
    let a = glyph.text_col(0, "a.txt").expect("a.txt tab");
    glyph.click(a, 0);
    wait_for_active(&glyph, " a.txt ");
    glyph.wait_for_text("1 │ alpha", WAIT);
    // Clicking a tab gives the editor focus: typing edits a.txt.
    glyph.type_text("z");
    glyph.wait_for_text("1 │ zalpha", WAIT);
    wait_for_active(&glyph, " a.txt ● ");

    // Middle click on a clean tab closes it.
    let b = glyph.text_col(0, "b.txt").expect("b.txt tab");
    glyph.middle_click(b, 0);
    wait_for_tabs(&glyph, " a.txt ●");

    // On a dirty one it asks first.
    let a = glyph.text_col(0, "a.txt").expect("a.txt tab");
    glyph.middle_click(a, 0);
    glyph.wait_for_text(PROMPT, WAIT);
    glyph.type_text("c");
    glyph.wait_for_text_gone(PROMPT, WAIT);
    wait_for_tabs(&glyph, " a.txt ●");
    glyph.middle_click(a, 0);
    glyph.wait_for_text(PROMPT, WAIT);
    glyph.type_text("d");
    wait_for_tabs(&glyph, " untitled");
}

#[test]
fn ctrl_q_asks_about_each_dirty_tab_in_turn() {
    let (dir, mut glyph) = open_project();
    open_both(&mut glyph);
    glyph.type_text("2");
    glyph.click(3, 1);
    glyph.wait_for_text("1 │ alpha", WAIT);
    glyph.type_text("1");
    // A clean untitled tab in between is never asked about.
    glyph.send_keys("ctrl+n");
    wait_for_tabs(&glyph, " a.txt ●  b.txt ●  untitled");

    glyph.send_keys("ctrl+q");
    glyph.wait_for_text("a.txt has unsaved changes", WAIT);
    // The question is about a.txt, shown as the active tab.
    wait_for_active(&glyph, " a.txt ● ");
    glyph.wait_for_text("1 │ 1alpha", WAIT);
    glyph.type_text("s");
    // Then b.txt.
    wait_for_active(&glyph, " b.txt ● ");
    glyph.wait_for_text("b.txt has unsaved changes", WAIT);
    glyph.wait_for_text("1 │ 2bravo", WAIT);
    assert_eq!(read(dir.path(), "a.txt"), "1alpha\n");

    // Cancel stops the quit with b.txt still unsaved.
    glyph.type_text("c");
    glyph.wait_for_text_gone(PROMPT, WAIT);
    glyph.assert_running_for(Duration::from_millis(300));
    wait_for_tabs(&glyph, " a.txt  b.txt ●  untitled");

    // Asked again, discard: nothing is left to ask about, so glyph quits.
    glyph.send_keys("ctrl+q");
    glyph.wait_for_text("b.txt has unsaved changes", WAIT);
    wait_for_active(&glyph, " b.txt ● ");
    glyph.type_text("d");
    assert!(glyph.wait_exit(WAIT).success());
    assert_eq!(read(dir.path(), "b.txt"), "bravo\n");
}
