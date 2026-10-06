mod harness;

use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{Griffin, ROWS};
use tempfile::TempDir;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const PROMPT: &str = "Unsaved changes: [S]ave [D]iscard [C]ancel";
/// The tree's width plus its divider: the tab bar starts in this column.
const BAR_X: u16 = 31;

/// A temp folder holding `a.txt` ("alpha") and `b.txt` ("bravo"), opened as the
/// project with the tree showing, cwd inside it.
fn open_project() -> (TempDir, Griffin) {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join("a.txt"), "alpha\n").expect("write a.txt");
    fs::write(dir.path().join("b.txt"), "bravo\n").expect("write b.txt");
    let griffin = Griffin::spawn_in(dir.path(), &["."]);
    griffin.wait_for_text("b.txt", START);
    (dir, griffin)
}

/// Opens both files from the tree (rows 1 and 2), leaving b.txt active.
fn open_both(griffin: &mut Griffin) {
    griffin.click(3, 1);
    griffin.wait_for_text("1 │ alpha", WAIT);
    griffin.click(3, 2);
    griffin.wait_for_text("1 │ bravo", WAIT);
    wait_for_tabs(griffin, " a.txt  b.txt");
}

/// Waits for the tab bar to read `text`, trailing spaces aside.
fn wait_for_tabs(griffin: &Griffin, text: &str) {
    griffin.wait_for_screen(&format!("tab bar {text:?}"), WAIT, |screen| {
        let bar: String = screen[0].chars().skip(usize::from(BAR_X)).collect();
        bar.trim_end() == text
    });
}

/// Waits for the active tab, the only reversed text on row 0, to be `text`.
fn wait_for_active(griffin: &Griffin, text: &str) {
    griffin.wait_for_reversed(0, text, WAIT);
}

fn status_line(griffin: &Griffin) -> String {
    griffin.screen()[usize::from(ROWS - 1)].clone()
}

fn read(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).expect("read file")
}

#[test]
fn row_0_shows_each_buffer_with_the_active_one_and_dirty_marks() {
    let (_dir, mut griffin) = open_project();
    // An untitled tab before anything is opened.
    wait_for_tabs(&griffin, " untitled");
    // Opening a file replaces that untouched untitled tab; a second gets its own.
    open_both(&mut griffin);
    wait_for_active(&griffin, " b.txt ");

    // The visual check: both names, the active one highlighted, the dirty one marked.
    griffin.type_text("x");
    wait_for_tabs(&griffin, " a.txt  b.txt ●");
    wait_for_active(&griffin, " b.txt ● ");
    let a = griffin.text_col(0, "a.txt").expect("a.txt tab");
    let b = griffin.text_col(0, "b.txt").expect("b.txt tab");
    assert!(BAR_X <= a && a < b, "{:#?}", griffin.screen());

    // Opening an already-open file switches to its tab instead of opening another.
    griffin.click(3, 1);
    wait_for_active(&griffin, " a.txt ");
    griffin.wait_for_text("1 │ alpha", WAIT);
    wait_for_tabs(&griffin, " a.txt  b.txt ●");
    griffin.click(3, 2);
    griffin.wait_for_text("1 │ xbravo", WAIT);
    wait_for_tabs(&griffin, " a.txt  b.txt ●");
}

#[test]
fn alt_keys_move_between_tabs_and_ctrl_w_closes_with_the_guard() {
    let (dir, mut griffin) = open_project();
    open_both(&mut griffin);
    griffin.send_keys("ctrl+n");
    wait_for_tabs(&griffin, " a.txt  b.txt  untitled");
    wait_for_active(&griffin, " untitled ");

    griffin.send_keys("alt+,");
    wait_for_active(&griffin, " b.txt ");
    griffin.send_keys("alt+.");
    wait_for_active(&griffin, " untitled ");
    // Past the end wraps round to the first.
    griffin.send_keys("alt+.");
    wait_for_active(&griffin, " a.txt ");
    griffin.wait_for_text("1 │ alpha", WAIT);
    griffin.send_keys("alt+3");
    wait_for_active(&griffin, " untitled ");
    griffin.send_keys("alt+2");
    wait_for_active(&griffin, " b.txt ");

    // A dirty tab asks first; cancel keeps it.
    griffin.type_text("y");
    wait_for_active(&griffin, " b.txt ● ");
    griffin.send_keys("ctrl+w");
    griffin.wait_for_text(PROMPT, WAIT);
    griffin.type_text("c");
    griffin.wait_for_text_gone(PROMPT, WAIT);
    wait_for_tabs(&griffin, " a.txt  b.txt ●  untitled");
    // Discard closes it without touching the file; the next tab takes over.
    griffin.send_keys("ctrl+w");
    griffin.wait_for_text(PROMPT, WAIT);
    griffin.type_text("d");
    wait_for_tabs(&griffin, " a.txt  untitled");
    wait_for_active(&griffin, " untitled ");
    assert_eq!(read(dir.path(), "b.txt"), "bravo\n");

    // Clean tabs close straight away.
    griffin.send_keys("ctrl+w");
    wait_for_tabs(&griffin, " a.txt");
    griffin.send_keys("ctrl+w");
    wait_for_tabs(&griffin, " untitled");
    griffin.send_keys("ctrl+q");
    assert!(griffin.wait_exit(WAIT).success());
}

#[test]
fn ctrl_n_then_alt_s_saves_relative_to_the_project_root() {
    let parent = tempfile::tempdir().expect("create temp dir");
    let root = parent.path().join("proj");
    fs::create_dir(&root).expect("create proj");
    fs::write(root.join("a.txt"), "alpha\n").expect("write a.txt");
    // cwd is the parent, so a path relative to the root is not one relative to cwd.
    let mut griffin = Griffin::spawn_in(parent.path(), &["proj"]);
    griffin.wait_for_text("a.txt", START);

    griffin.send_keys("ctrl+n");
    wait_for_active(&griffin, " untitled ");
    griffin.type_text("hello");
    griffin.wait_for_text("1 │ hello", WAIT);
    wait_for_active(&griffin, " untitled ● ");

    // A name another file already has changes nothing.
    griffin.send_keys("alt+s");
    griffin.wait_for_text("Save as:", WAIT);
    griffin.type_text("a.txt");
    griffin.send_keys("enter");
    griffin.wait_for_text("a.txt already exists", WAIT);
    assert_eq!(read(&root, "a.txt"), "alpha\n");

    griffin.send_keys("alt+s");
    griffin.wait_for_text("Save as:", WAIT);
    griffin.type_text("new.txt");
    griffin.send_keys("enter");
    griffin.wait_for_text_gone("Save as:", WAIT);
    wait_for_active(&griffin, " new.txt ");
    assert_eq!(read(&root, "new.txt"), "hello");
    assert!(!parent.path().join("new.txt").exists());
    assert!(!status_line(&griffin).contains('●'));
    // The tree shows the new file.
    griffin.wait_for_text("  new.txt", WAIT);

    // Save as again offers the current path, relative to the root, and moves the
    // tab to the new name; the old file stays as it was.
    griffin.type_text("!");
    griffin.send_keys("alt+s");
    griffin.wait_for_text("Save as: new.txt", WAIT);
    for _ in 0.."new.txt".len() {
        griffin.send_keys("backspace");
    }
    griffin.type_text("renamed.txt");
    griffin.send_keys("enter");
    wait_for_active(&griffin, " renamed.txt ");
    assert_eq!(read(&root, "renamed.txt"), "hello!");
    assert_eq!(read(&root, "new.txt"), "hello");
    griffin.send_keys("ctrl+q");
    assert!(griffin.wait_exit(WAIT).success());
}

#[test]
fn click_selects_a_tab_and_middle_click_closes_it() {
    let (_dir, mut griffin) = open_project();
    open_both(&mut griffin);
    let a = griffin.text_col(0, "a.txt").expect("a.txt tab");
    griffin.click(a, 0);
    wait_for_active(&griffin, " a.txt ");
    griffin.wait_for_text("1 │ alpha", WAIT);
    // Clicking a tab gives the editor focus: typing edits a.txt.
    griffin.type_text("z");
    griffin.wait_for_text("1 │ zalpha", WAIT);
    wait_for_active(&griffin, " a.txt ● ");

    // Middle click on a clean tab closes it.
    let b = griffin.text_col(0, "b.txt").expect("b.txt tab");
    griffin.middle_click(b, 0);
    wait_for_tabs(&griffin, " a.txt ●");

    // On a dirty one it asks first.
    let a = griffin.text_col(0, "a.txt").expect("a.txt tab");
    griffin.middle_click(a, 0);
    griffin.wait_for_text(PROMPT, WAIT);
    griffin.type_text("c");
    griffin.wait_for_text_gone(PROMPT, WAIT);
    wait_for_tabs(&griffin, " a.txt ●");
    griffin.middle_click(a, 0);
    griffin.wait_for_text(PROMPT, WAIT);
    griffin.type_text("d");
    wait_for_tabs(&griffin, " untitled");
}

#[test]
fn ctrl_q_asks_about_each_dirty_tab_in_turn() {
    let (dir, mut griffin) = open_project();
    open_both(&mut griffin);
    griffin.type_text("2");
    griffin.click(3, 1);
    griffin.wait_for_text("1 │ alpha", WAIT);
    griffin.type_text("1");
    // A clean untitled tab in between is never asked about.
    griffin.send_keys("ctrl+n");
    wait_for_tabs(&griffin, " a.txt ●  b.txt ●  untitled");

    griffin.send_keys("ctrl+q");
    griffin.wait_for_text(PROMPT, WAIT);
    // The question is about a.txt, shown as the active tab.
    wait_for_active(&griffin, " a.txt ● ");
    griffin.wait_for_text("1 │ 1alpha", WAIT);
    griffin.type_text("s");
    // Then b.txt.
    wait_for_active(&griffin, " b.txt ● ");
    griffin.wait_for_text(PROMPT, WAIT);
    griffin.wait_for_text("1 │ 2bravo", WAIT);
    assert_eq!(read(dir.path(), "a.txt"), "1alpha\n");

    // Cancel stops the quit with b.txt still unsaved.
    griffin.type_text("c");
    griffin.wait_for_text_gone(PROMPT, WAIT);
    griffin.assert_running_for(Duration::from_millis(300));
    wait_for_tabs(&griffin, " a.txt  b.txt ●  untitled");

    // Asked again, discard: nothing is left to ask about, so griffin quits.
    griffin.send_keys("ctrl+q");
    griffin.wait_for_text(PROMPT, WAIT);
    wait_for_active(&griffin, " b.txt ● ");
    griffin.type_text("d");
    assert!(griffin.wait_exit(WAIT).success());
    assert_eq!(read(dir.path(), "b.txt"), "bravo\n");
}
