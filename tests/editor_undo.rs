mod harness;

use std::time::Duration;

use harness::{ROWS, Tome};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);

/// Waits for the status line to show `Ln <line>, Col <col>` (see `shows_position`).
fn wait_for_position(tome: &Tome, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    tome.wait_for_text(&position, WAIT);
    let status = tome.screen()[usize::from(ROWS - 1)].clone();
    assert!(
        harness::shows_position(&status, &position),
        "status line {status:?} should show {position:?}"
    );
}

/// Waits until the editor rows read exactly `lines` (after the gutter), with
/// nothing numbered below them.
fn wait_for_lines(tome: &Tome, lines: &[&str]) {
    let last = format!("{}  {}", lines.len(), lines[lines.len() - 1]);
    tome.wait_for_text(last.trim_end(), WAIT);
    // The next line's gutter: its number right-aligned in three cells.
    let next = format!(" {:>3}", lines.len() + 1);
    tome.wait_for_text_gone(&next, WAIT);
    let screen = tome.screen();
    for (row, text) in lines.iter().enumerate() {
        let expected = format!("{}  {text}", row + 1);
        // Rows 0-2 are the tab header.
        assert_eq!(screen[row + 3].trim(), expected.trim_end(), "{screen:#?}");
    }
}

#[test]
fn undo_redo_typing() {
    let mut tome = Tome::spawn_with_config("", &[]);
    tome.wait_for_text("Open directory", START);
    // Leave the splash for the untitled buffer under it.
    tome.send_keys("ctrl+n");

    // Each step waits for the screen: on Windows, keys that arrive in one burst with
    // a line break are taken for a paste, which undoes as a single step.
    tome.type_text("abc");
    wait_for_position(&tome, 1, 4);
    tome.send_keys("enter");
    wait_for_position(&tome, 2, 1);
    tome.type_text("def");
    wait_for_lines(&tome, &["abc", "def"]);
    wait_for_position(&tome, 2, 4);

    // The second run of typing goes first; the cursor returns to where it began.
    tome.send_keys("ctrl+z");
    tome.wait_for_text_gone("def", WAIT);
    wait_for_lines(&tome, &["abc", ""]);
    wait_for_position(&tome, 2, 1);

    // `abc` and the Enter that ended it undo together.
    tome.send_keys("ctrl+z");
    tome.wait_for_text_gone("abc", WAIT);
    wait_for_lines(&tome, &[""]);
    wait_for_position(&tome, 1, 1);

    tome.send_keys("ctrl+y");
    wait_for_lines(&tome, &["abc", ""]);
    wait_for_position(&tome, 2, 1);
    tome.send_keys("ctrl+y");
    wait_for_lines(&tome, &["abc", "def"]);
    wait_for_position(&tome, 2, 4);

    // Undo marked the buffer changed, so quitting still asks.
    tome.send_keys("ctrl+q");
    tome.wait_for_text("has unsaved changes", WAIT);
    tome.type_text("d");
    let status = tome.wait_exit(WAIT);
    assert!(status.success(), "tome exited with {status:?}");
}
