mod harness;

use std::fs;
use std::time::Duration;

use harness::{Griffin, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);

/// Waits for the status line to end with `Ln <line>, Col <col>`.
fn wait_for_position(griffin: &Griffin, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    griffin.wait_for_text(&position, WAIT);
    let status = griffin.screen()[usize::from(ROWS - 1)].clone();
    assert!(
        status.trim_end().ends_with(&position),
        "status line {status:?} should end with {position:?}"
    );
}

/// Waits until the editor rows read exactly `lines` (after the gutter), with
/// nothing numbered below them.
fn wait_for_lines(griffin: &Griffin, lines: &[&str]) {
    let last = format!("{} │ {}", lines.len(), lines[lines.len() - 1]);
    griffin.wait_for_text(last.trim_end(), WAIT);
    let next = format!("{} │", lines.len() + 1);
    griffin.wait_for_text_gone(&next, WAIT);
    let screen = griffin.screen();
    for (row, text) in lines.iter().enumerate() {
        let expected = format!("{} │ {text}", row + 1);
        // Row 0 is the tab bar.
        assert_eq!(screen[row + 1].trim(), expected.trim_end(), "{screen:#?}");
    }
}

/// Opens a scratch file holding `text`.
fn open(text: &str) -> (tempfile::TempDir, Griffin) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join("select.txt");
    fs::write(&path, text).expect("write fixture");
    let path = path.to_str().expect("temp path is UTF-8").to_string();
    let griffin = Griffin::spawn_with_config("", &[&path]);
    griffin.wait_for_text("Ln 1, Col 1", START);
    (dir, griffin)
}

#[test]
fn shift_right_selects() {
    let (_dir, mut griffin) = open("hello world");
    wait_for_lines(&griffin, &["hello world"]);
    assert_eq!(griffin.reversed_text(1), "");

    for _ in 0..5 {
        griffin.send_keys("shift+right");
    }
    wait_for_position(&griffin, 1, 6);
    griffin.wait_for_reversed(1, "hello", WAIT);

    // A plain movement drops the selection.
    griffin.send_keys("right");
    wait_for_position(&griffin, 1, 7);
    griffin.wait_for_reversed(1, "", WAIT);

    // Select it again and type over it.
    griffin.send_keys("home");
    for _ in 0..5 {
        griffin.send_keys("shift+right");
    }
    griffin.wait_for_reversed(1, "hello", WAIT);
    griffin.type_text("bye");
    wait_for_lines(&griffin, &["bye world"]);
    griffin.wait_for_reversed(1, "", WAIT);
    wait_for_position(&griffin, 1, 4);

    // Select all covers every line.
    griffin.send_keys("ctrl+a");
    griffin.wait_for_reversed(1, "bye world", WAIT);
}

#[test]
fn paste_is_one_undo_step() {
    let (_dir, mut griffin) = open("ab");
    wait_for_lines(&griffin, &["ab"]);
    griffin.send_keys("end");
    wait_for_position(&griffin, 1, 3);

    // Terminals send a pasted line break as CR, as if Enter were pressed.
    griffin.write(b"\x1b[200~one\rtwo\x1b[201~");
    wait_for_lines(&griffin, &["abone", "two"]);
    wait_for_position(&griffin, 2, 4);

    griffin.send_keys("ctrl+z");
    wait_for_lines(&griffin, &["ab"]);
    wait_for_position(&griffin, 1, 3);

    // A paste replaces the selection, still as one step.
    griffin.send_keys("ctrl+a");
    griffin.wait_for_reversed(1, "ab", WAIT);
    griffin.write(b"\x1b[200~xyz\x1b[201~");
    wait_for_lines(&griffin, &["xyz"]);
    griffin.send_keys("ctrl+z");
    wait_for_lines(&griffin, &["ab"]);
}
