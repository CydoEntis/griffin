mod harness;

use std::time::Duration;

use harness::{Griffin, ROWS};

const START: Duration = Duration::from_secs(10);
const EXIT: Duration = Duration::from_secs(5);

#[test]
fn remapped_quit() {
    let mut griffin = Griffin::spawn_with_config("[keys]\nquit = \"alt+q\"\n", &[]);
    griffin.wait_for_text("griffin", START);

    griffin.send_keys("ctrl+q");
    griffin.assert_running_for(Duration::from_millis(750));

    griffin.send_keys("alt+q");
    let status = griffin.wait_exit(EXIT);
    assert!(status.success(), "griffin exited with {status:?}");
}

#[test]
fn bad_config_shows_error() {
    let mut griffin = Griffin::spawn_with_config("[keys]\nquit = = \"alt+q\"\n", &[]);
    griffin.wait_for_text("config error:", START);
    let screen = griffin.screen();
    let status_line = &screen[usize::from(ROWS) - 1];
    assert!(
        status_line.contains("config error: line 2:"),
        "status line should carry the error: {screen:#?}"
    );

    // Defaults still apply, so the default quit key works.
    griffin.send_keys("ctrl+q");
    let status = griffin.wait_exit(EXIT);
    assert!(status.success(), "griffin exited with {status:?}");
}

#[test]
fn unknown_action_shows_error_and_keeps_defaults() {
    let mut griffin = Griffin::spawn_with_config("[keys]\nfly = \"ctrl+k\"\n", &[]);
    griffin.wait_for_text("config error: [keys]: unknown action \"fly\"", START);

    griffin.send_keys("ctrl+q");
    let status = griffin.wait_exit(EXIT);
    assert!(status.success(), "griffin exited with {status:?}");
}
