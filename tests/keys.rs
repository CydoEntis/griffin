mod harness;

use std::time::Duration;

use harness::{ROWS, Tome};

const START: Duration = Duration::from_secs(10);
const EXIT: Duration = Duration::from_secs(5);

#[test]
fn remapped_quit() {
    let mut tome = Tome::spawn_with_config("[keys]\nquit = \"alt+q\"\n", &[]);
    tome.wait_for_text("tome", START);

    tome.send_keys("ctrl+q");
    tome.assert_running_for(Duration::from_millis(750));

    tome.send_keys("alt+q");
    let status = tome.wait_exit(EXIT);
    assert!(status.success(), "tome exited with {status:?}");
}

#[test]
fn bad_config_shows_error() {
    let mut tome = Tome::spawn_with_config("[keys]\nquit = = \"alt+q\"\n", &[]);
    tome.wait_for_text("config error:", START);
    let screen = tome.screen();
    let status_line = &screen[usize::from(ROWS) - 1];
    assert!(
        status_line.contains("config error: line 2:"),
        "status line should carry the error: {screen:#?}"
    );

    // Defaults still apply, so the default quit key works.
    tome.send_keys("ctrl+q");
    let status = tome.wait_exit(EXIT);
    assert!(status.success(), "tome exited with {status:?}");
}

#[test]
fn unknown_action_shows_error_and_keeps_defaults() {
    let mut tome = Tome::spawn_with_config("[keys]\nfly = \"ctrl+k\"\n", &[]);
    tome.wait_for_text("config error: [keys]: unknown action \"fly\"", START);

    tome.send_keys("ctrl+q");
    let status = tome.wait_exit(EXIT);
    assert!(status.success(), "tome exited with {status:?}");
}
