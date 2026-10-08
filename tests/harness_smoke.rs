mod harness;

use std::time::Duration;

use harness::{Glyph, ROWS};

#[test]
fn starts_draws_the_status_line_and_quits_on_ctrl_q() {
    let mut glyph = Glyph::spawn(&[]);

    glyph.wait_for_text("glyph", Duration::from_secs(10));
    let screen = glyph.screen();
    assert_eq!(screen.len(), usize::from(ROWS));
    assert!(
        screen[screen.len() - 1].starts_with(" ✦ glyph"),
        "status line should be on the last row: {screen:#?}"
    );
    let (col, row) = glyph.cursor();
    assert!(col < harness::COLS && row < ROWS);

    glyph.send_keys("ctrl+q");
    let status = glyph.wait_exit(Duration::from_secs(5));
    assert!(status.success(), "glyph exited with {status:?}");
}
