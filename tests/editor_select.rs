mod harness;

use std::fs;
use std::time::Duration;

use harness::{Glyph, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);

/// Waits for the status line to show `Ln <line>, Col <col>` (see `shows_position`).
fn wait_for_position(glyph: &Glyph, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    glyph.wait_for_text(&position, WAIT);
    let status = glyph.screen()[usize::from(ROWS - 1)].clone();
    assert!(
        harness::shows_position(&status, &position),
        "status line {status:?} should show {position:?}"
    );
}

/// Waits until the editor rows read exactly `lines` (after the gutter), with
/// nothing numbered below them.
fn wait_for_lines(glyph: &Glyph, lines: &[&str]) {
    let last = format!("{} │ {}", lines.len(), lines[lines.len() - 1]);
    glyph.wait_for_text(last.trim_end(), WAIT);
    let next = format!("{} │", lines.len() + 1);
    glyph.wait_for_text_gone(&next, WAIT);
    let screen = glyph.screen();
    for (row, text) in lines.iter().enumerate() {
        let expected = format!("{} │ {text}", row + 1);
        // Row 0 is the tab bar.
        assert_eq!(screen[row + 1].trim(), expected.trim_end(), "{screen:#?}");
    }
}

/// Opens a scratch file holding `text`.
fn open(text: &str) -> (tempfile::TempDir, Glyph) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("select.txt");
    fs::write(&path, text).expect("write fixture");
    let path = path.to_str().expect("temp path is UTF-8").to_string();
    let glyph = Glyph::spawn_with_config("", &[&path]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    (dir, glyph)
}

#[test]
fn shift_right_selects() {
    let (_dir, mut glyph) = open("hello world");
    wait_for_lines(&glyph, &["hello world"]);
    assert_eq!(glyph.reversed_text(1), "");

    for _ in 0..5 {
        glyph.send_keys("shift+right");
    }
    wait_for_position(&glyph, 1, 6);
    glyph.wait_for_reversed(1, "hello", WAIT);

    // A plain movement drops the selection.
    glyph.send_keys("right");
    wait_for_position(&glyph, 1, 7);
    glyph.wait_for_reversed(1, "", WAIT);

    // Select it again and type over it.
    glyph.send_keys("home");
    for _ in 0..5 {
        glyph.send_keys("shift+right");
    }
    glyph.wait_for_reversed(1, "hello", WAIT);
    glyph.type_text("bye");
    wait_for_lines(&glyph, &["bye world"]);
    glyph.wait_for_reversed(1, "", WAIT);
    wait_for_position(&glyph, 1, 4);

    // Select all covers every line.
    glyph.send_keys("ctrl+a");
    glyph.wait_for_reversed(1, "bye world", WAIT);
}

#[test]
fn paste_is_one_undo_step() {
    let (_dir, mut glyph) = open("ab");
    wait_for_lines(&glyph, &["ab"]);
    glyph.send_keys("end");
    wait_for_position(&glyph, 1, 3);

    // Terminals send a pasted line break as CR, as if Enter were pressed.
    glyph.write(b"\x1b[200~one\rtwo\x1b[201~");
    wait_for_lines(&glyph, &["abone", "two"]);
    wait_for_position(&glyph, 2, 4);

    glyph.send_keys("ctrl+z");
    wait_for_lines(&glyph, &["ab"]);
    wait_for_position(&glyph, 1, 3);

    // A paste replaces the selection, still as one step.
    glyph.send_keys("ctrl+a");
    glyph.wait_for_reversed(1, "ab", WAIT);
    glyph.write(b"\x1b[200~xyz\x1b[201~");
    wait_for_lines(&glyph, &["xyz"]);
    glyph.send_keys("ctrl+z");
    wait_for_lines(&glyph, &["ab"]);
}
