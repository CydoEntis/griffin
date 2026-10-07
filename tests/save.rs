mod harness;

use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{COLS, Glyph, ROWS};
use tempfile::TempDir;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const PROMPT: &str = "Unsaved changes: [S]ave [D]iscard [C]ancel";

fn status_line(glyph: &Glyph) -> String {
    glyph.screen()[usize::from(ROWS - 1)].clone()
}

/// Opens `a.txt` holding `contents` in a fresh temp dir, with glyph running there.
fn open_a(contents: &[u8]) -> (TempDir, Glyph) {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join("a.txt"), contents).expect("write a.txt");
    let glyph = Glyph::spawn_in(dir.path(), &["a.txt"]);
    glyph.wait_for_text("a.txt", START);
    glyph.wait_for_text("Ln 1, Col 1", START);
    (dir, glyph)
}

fn read_a(dir: &Path) -> Vec<u8> {
    fs::read(dir.join("a.txt")).expect("read a.txt")
}

/// Types `x` and waits for the dirty marker.
fn make_dirty(glyph: &mut Glyph) {
    glyph.type_text("x");
    glyph.wait_for_text("a.txt ●", WAIT);
}

/// Ctrl+Q, then checks the prompt sits in the middle of the screen.
fn open_quit_prompt(glyph: &mut Glyph) {
    glyph.send_keys("ctrl+q");
    glyph.wait_for_text(PROMPT, WAIT);
    let screen = glyph.screen();
    let row = screen
        .iter()
        .position(|line| line.contains(PROMPT))
        .unwrap_or_else(|| panic!("{screen:#?}"));
    assert!((13..=16).contains(&row), "prompt on row {row}: {screen:#?}");
    let col = glyph
        .text_col(row as u16, PROMPT)
        .unwrap_or_else(|| panic!("{screen:#?}"));
    let right = COLS - (col + PROMPT.len() as u16);
    assert!(col.abs_diff(right) <= 1, "prompt not centred: {screen:#?}");
}

fn assert_exits(glyph: &mut Glyph) {
    let status = glyph.wait_exit(WAIT);
    assert!(status.success(), "glyph exited with {status:?}");
}

#[test]
fn ctrl_s_saves() {
    let (dir, mut glyph) = open_a(b"hello\r\nworld");
    assert!(!status_line(&glyph).contains('●'));

    make_dirty(&mut glyph);
    glyph.send_keys("ctrl+s");
    glyph.wait_for_text("saved a.txt", WAIT);
    let status = status_line(&glyph);
    assert!(!status.contains('●'), "still dirty: {status:?}");
    // CRLF and the missing final newline survive the save.
    assert_eq!(read_a(dir.path()), b"xhello\r\nworld");

    // A clean buffer quits without asking.
    glyph.send_keys("ctrl+q");
    assert_exits(&mut glyph);
}

#[test]
fn ctrl_s_on_an_untitled_buffer_asks_for_a_path() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let mut glyph = Glyph::spawn_in(dir.path(), &[]);
    glyph.wait_for_text("untitled", START);
    glyph.type_text("x");
    glyph.wait_for_text("untitled ●", WAIT);
    glyph.send_keys("ctrl+s");
    glyph.wait_for_text("Save as:", WAIT);
    // Esc leaves it unsaved.
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("Save as:", WAIT);
    assert!(status_line(&glyph).contains("untitled ●"));
    assert_eq!(fs::read_dir(dir.path()).expect("list dir").count(), 0);

    glyph.send_keys("ctrl+q");
    glyph.wait_for_text(PROMPT, WAIT);
    glyph.type_text("d");
    assert_exits(&mut glyph);
}

#[test]
fn quit_guard_save() {
    let (dir, mut glyph) = open_a(b"hello\n");
    make_dirty(&mut glyph);
    open_quit_prompt(&mut glyph);
    glyph.type_text("s");
    assert_exits(&mut glyph);
    assert_eq!(read_a(dir.path()), b"xhello\n");
}

#[test]
fn quit_guard_discard() {
    let (dir, mut glyph) = open_a(b"hello\n");
    make_dirty(&mut glyph);
    open_quit_prompt(&mut glyph);
    glyph.type_text("d");
    assert_exits(&mut glyph);
    assert_eq!(read_a(dir.path()), b"hello\n");
}

#[test]
fn quit_guard_cancel() {
    let (dir, mut glyph) = open_a(b"hello\n");
    make_dirty(&mut glyph);

    // C closes the prompt and editing carries on.
    open_quit_prompt(&mut glyph);
    glyph.type_text("c");
    glyph.wait_for_text_gone(PROMPT, WAIT);
    glyph.type_text("y");
    glyph.wait_for_text("xyhello", WAIT);

    // So does Esc.
    open_quit_prompt(&mut glyph);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(PROMPT, WAIT);
    glyph.type_text("z");
    glyph.wait_for_text("xyzhello", WAIT);
    assert_eq!(read_a(dir.path()), b"hello\n");

    open_quit_prompt(&mut glyph);
    glyph.type_text("d");
    assert_exits(&mut glyph);
}
