mod harness;

use std::fs;
use std::time::Duration;

use harness::Tome;
use tempfile::TempDir;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);

/// Opens `name` holding `contents` in a fresh temp dir, with tome running there.
fn open(name: &str, contents: &str) -> (TempDir, Tome) {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join(name), contents).expect("write file");
    let tome = Tome::spawn_in(dir.path(), &[name]);
    tome.wait_for_text(name, START);
    tome.wait_for_text("Ln 1, Col 1", START);
    (dir, tome)
}

/// Ctrl+/ comments the line and a second press takes the comment off, whichever
/// of the two bound keys sends it.
fn toggles_a_rust_line(key: &str) {
    let (_dir, mut tome) = open("a.rs", "let x = 1;\n");
    tome.wait_for_text("1  let x = 1;", WAIT);
    tome.send_keys(key);
    tome.wait_for_text("1  // let x = 1;", WAIT);
    tome.send_keys(key);
    tome.wait_for_text_gone("//", WAIT);
    tome.wait_for_text("1  let x = 1;", WAIT);
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
    let (_dir, mut tome) = open("a.txt", "plain words\n");
    tome.wait_for_text("1  plain words", WAIT);
    tome.send_keys("ctrl+/");
    tome.wait_for_text("no comments for this file", WAIT);
    let screen = tome.screen();
    assert!(
        screen.iter().any(|row| row.contains("1  plain words")),
        "{screen:#?}"
    );
    assert!(
        !screen.iter().any(|row| row.contains("a.txt •")),
        "{screen:#?}"
    );
}

#[test]
fn ctrl_slash_wraps_an_html_line_and_unwraps_it() {
    let (_dir, mut tome) = open("a.html", "<p>hi</p>\n");
    tome.wait_for_text("1  <p>hi</p>", WAIT);
    tome.send_keys("ctrl+/");
    tome.wait_for_text("1  <!-- <p>hi</p> -->", WAIT);
    tome.send_keys("ctrl+/");
    tome.wait_for_text_gone("<!--", WAIT);
    tome.wait_for_text("1  <p>hi</p>", WAIT);
}

#[test]
fn ctrl_slash_wraps_a_css_line_and_unwraps_it() {
    let (_dir, mut tome) = open("a.css", "p { color: red; }\n");
    tome.wait_for_text("1  p { color: red; }", WAIT);
    tome.send_keys("ctrl+/");
    tome.wait_for_text("1  /* p { color: red; } */", WAIT);
    tome.send_keys("ctrl+/");
    tome.wait_for_text_gone("/*", WAIT);
    tome.wait_for_text("1  p { color: red; }", WAIT);
}
