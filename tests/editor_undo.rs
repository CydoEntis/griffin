mod harness;

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

#[test]
fn undo_redo_typing() {
    let mut griffin = Griffin::spawn_with_config("", &[]);
    griffin.wait_for_text("Ln 1, Col 1", START);

    // Each step waits for the screen: on Windows, keys that arrive in one burst with
    // a line break are taken for a paste, which undoes as a single step.
    griffin.type_text("abc");
    wait_for_position(&griffin, 1, 4);
    griffin.send_keys("enter");
    wait_for_position(&griffin, 2, 1);
    griffin.type_text("def");
    wait_for_lines(&griffin, &["abc", "def"]);
    wait_for_position(&griffin, 2, 4);

    // The second run of typing goes first; the cursor returns to where it began.
    griffin.send_keys("ctrl+z");
    griffin.wait_for_text_gone("def", WAIT);
    wait_for_lines(&griffin, &["abc", ""]);
    wait_for_position(&griffin, 2, 1);

    // `abc` and the Enter that ended it undo together.
    griffin.send_keys("ctrl+z");
    griffin.wait_for_text_gone("abc", WAIT);
    wait_for_lines(&griffin, &[""]);
    wait_for_position(&griffin, 1, 1);

    griffin.send_keys("ctrl+y");
    wait_for_lines(&griffin, &["abc", ""]);
    wait_for_position(&griffin, 2, 1);
    griffin.send_keys("ctrl+y");
    wait_for_lines(&griffin, &["abc", "def"]);
    wait_for_position(&griffin, 2, 4);

    // Undo marked the buffer changed, so quitting still asks.
    griffin.send_keys("ctrl+q");
    griffin.wait_for_text("Unsaved changes", WAIT);
    griffin.type_text("d");
    let status = griffin.wait_exit(WAIT);
    assert!(status.success(), "griffin exited with {status:?}");
}
