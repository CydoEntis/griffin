mod harness;

use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{COLS, Griffin, ROWS};
use tempfile::TempDir;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const PROMPT: &str = "Unsaved changes: [S]ave [D]iscard [C]ancel";

fn status_line(griffin: &Griffin) -> String {
    griffin.screen()[usize::from(ROWS - 1)].clone()
}

/// Opens `a.txt` holding `contents` in a fresh temp dir, with griffin running there.
fn open_a(contents: &[u8]) -> (TempDir, Griffin) {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join("a.txt"), contents).expect("write a.txt");
    let griffin = Griffin::spawn_in(dir.path(), &["a.txt"]);
    griffin.wait_for_text("a.txt", START);
    griffin.wait_for_text("Ln 1, Col 1", START);
    (dir, griffin)
}

fn read_a(dir: &Path) -> Vec<u8> {
    fs::read(dir.join("a.txt")).expect("read a.txt")
}

/// Types `x` and waits for the dirty marker.
fn make_dirty(griffin: &mut Griffin) {
    griffin.type_text("x");
    griffin.wait_for_text("a.txt ●", WAIT);
}

/// Ctrl+Q, then checks the prompt sits in the middle of the screen.
fn open_quit_prompt(griffin: &mut Griffin) {
    griffin.send_keys("ctrl+q");
    griffin.wait_for_text(PROMPT, WAIT);
    let screen = griffin.screen();
    let row = screen
        .iter()
        .position(|line| line.contains(PROMPT))
        .unwrap_or_else(|| panic!("{screen:#?}"));
    assert!((13..=16).contains(&row), "prompt on row {row}: {screen:#?}");
    let col = griffin
        .text_col(row as u16, PROMPT)
        .unwrap_or_else(|| panic!("{screen:#?}"));
    let right = COLS - (col + PROMPT.len() as u16);
    assert!(col.abs_diff(right) <= 1, "prompt not centred: {screen:#?}");
}

fn assert_exits(griffin: &mut Griffin) {
    let status = griffin.wait_exit(WAIT);
    assert!(status.success(), "griffin exited with {status:?}");
}

#[test]
fn ctrl_s_saves() {
    let (dir, mut griffin) = open_a(b"hello\r\nworld");
    assert!(!status_line(&griffin).contains('●'));

    make_dirty(&mut griffin);
    griffin.send_keys("ctrl+s");
    griffin.wait_for_text("saved a.txt", WAIT);
    let status = status_line(&griffin);
    assert!(!status.contains('●'), "still dirty: {status:?}");
    // CRLF and the missing final newline survive the save.
    assert_eq!(read_a(dir.path()), b"xhello\r\nworld");

    // A clean buffer quits without asking.
    griffin.send_keys("ctrl+q");
    assert_exits(&mut griffin);
}

#[test]
fn ctrl_s_on_an_untitled_buffer_asks_for_a_path() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let mut griffin = Griffin::spawn_in(dir.path(), &[]);
    griffin.wait_for_text("untitled", START);
    griffin.type_text("x");
    griffin.wait_for_text("untitled ●", WAIT);
    griffin.send_keys("ctrl+s");
    griffin.wait_for_text("Save as:", WAIT);
    // Esc leaves it unsaved.
    griffin.send_keys("esc");
    griffin.wait_for_text_gone("Save as:", WAIT);
    assert!(status_line(&griffin).contains("untitled ●"));
    assert_eq!(fs::read_dir(dir.path()).expect("list dir").count(), 0);

    griffin.send_keys("ctrl+q");
    griffin.wait_for_text(PROMPT, WAIT);
    griffin.type_text("d");
    assert_exits(&mut griffin);
}

#[test]
fn quit_guard_save() {
    let (dir, mut griffin) = open_a(b"hello\n");
    make_dirty(&mut griffin);
    open_quit_prompt(&mut griffin);
    griffin.type_text("s");
    assert_exits(&mut griffin);
    assert_eq!(read_a(dir.path()), b"xhello\n");
}

#[test]
fn quit_guard_discard() {
    let (dir, mut griffin) = open_a(b"hello\n");
    make_dirty(&mut griffin);
    open_quit_prompt(&mut griffin);
    griffin.type_text("d");
    assert_exits(&mut griffin);
    assert_eq!(read_a(dir.path()), b"hello\n");
}

#[test]
fn quit_guard_cancel() {
    let (dir, mut griffin) = open_a(b"hello\n");
    make_dirty(&mut griffin);

    // C closes the prompt and editing carries on.
    open_quit_prompt(&mut griffin);
    griffin.type_text("c");
    griffin.wait_for_text_gone(PROMPT, WAIT);
    griffin.type_text("y");
    griffin.wait_for_text("xyhello", WAIT);

    // So does Esc.
    open_quit_prompt(&mut griffin);
    griffin.send_keys("esc");
    griffin.wait_for_text_gone(PROMPT, WAIT);
    griffin.type_text("z");
    griffin.wait_for_text("xyzhello", WAIT);
    assert_eq!(read_a(dir.path()), b"hello\n");

    open_quit_prompt(&mut griffin);
    griffin.type_text("d");
    assert_exits(&mut griffin);
}
