mod harness;

use std::fs;
use std::time::Duration;

use harness::Glyph;
use tempfile::TempDir;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);

/// Opens `name` holding `contents` in a fresh temp dir, with glyph running there.
fn open(name: &str, contents: &str) -> (TempDir, Glyph) {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join(name), contents).expect("write file");
    let glyph = Glyph::spawn_in(dir.path(), &[name]);
    glyph.wait_for_text(name, START);
    glyph.wait_for_text("Ln 1, Col 1", START);
    (dir, glyph)
}

/// Ctrl+/ comments the line and a second press takes the comment off, whichever
/// of the two bound keys sends it.
fn toggles_a_rust_line(key: &str) {
    let (_dir, mut glyph) = open("a.rs", "let x = 1;\n");
    glyph.wait_for_text("1  let x = 1;", WAIT);
    glyph.send_keys(key);
    glyph.wait_for_text("1  // let x = 1;", WAIT);
    glyph.send_keys(key);
    glyph.wait_for_text_gone("//", WAIT);
    glyph.wait_for_text("1  let x = 1;", WAIT);
}

#[test]
fn ctrl_slash_comments_and_uncomments_a_rust_line() {
    toggles_a_rust_line("ctrl+/");
}

#[test]
fn ctrl_7_comments_and_uncomments_a_rust_line() {
    toggles_a_rust_line("ctrl+7");
}

#[test]
fn a_file_with_no_language_has_no_comments() {
    let (_dir, mut glyph) = open("a.txt", "plain words\n");
    glyph.wait_for_text("1  plain words", WAIT);
    glyph.send_keys("ctrl+/");
    glyph.wait_for_text("no comments for this file", WAIT);
    let screen = glyph.screen();
    assert!(
        screen.iter().any(|row| row.contains("1  plain words")),
        "{screen:#?}"
    );
    assert!(
        !screen.iter().any(|row| row.contains("a.txt •")),
        "{screen:#?}"
    );
}
