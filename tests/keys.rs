mod harness;

use std::time::Duration;

use harness::{Glyph, ROWS};

const START: Duration = Duration::from_secs(10);
const EXIT: Duration = Duration::from_secs(5);

#[test]
fn remapped_quit() {
    let mut glyph = Glyph::spawn_with_config("[keys]\nquit = \"alt+q\"\n", &[]);
    glyph.wait_for_text("glyph", START);

    glyph.send_keys("ctrl+q");
    glyph.assert_running_for(Duration::from_millis(750));

    glyph.send_keys("alt+q");
    let status = glyph.wait_exit(EXIT);
    assert!(status.success(), "glyph exited with {status:?}");
}

#[test]
fn bad_config_shows_error() {
    let mut glyph = Glyph::spawn_with_config("[keys]\nquit = = \"alt+q\"\n", &[]);
    glyph.wait_for_text("config error:", START);
    let screen = glyph.screen();
    let status_line = &screen[usize::from(ROWS) - 1];
    assert!(
        status_line.contains("config error: line 2:"),
        "status line should carry the error: {screen:#?}"
    );

    // Defaults still apply, so the default quit key works.
    glyph.send_keys("ctrl+q");
    let status = glyph.wait_exit(EXIT);
    assert!(status.success(), "glyph exited with {status:?}");
}

#[test]
fn unknown_action_shows_error_and_keeps_defaults() {
    let mut glyph = Glyph::spawn_with_config("[keys]\nfly = \"ctrl+k\"\n", &[]);
    glyph.wait_for_text("config error: [keys]: unknown action \"fly\"", START);

    glyph.send_keys("ctrl+q");
    let status = glyph.wait_exit(EXIT);
    assert!(status.success(), "glyph exited with {status:?}");
}
