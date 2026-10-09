mod harness;

use std::time::Duration;

use harness::{ROWS, Tome};

#[test]
fn starts_draws_the_status_line_and_quits_on_ctrl_q() {
    let mut tome = Tome::spawn(&[]);

    tome.wait_for_text("tome", Duration::from_secs(10));
    let screen = tome.screen();
    assert_eq!(screen.len(), usize::from(ROWS));
    assert!(
        screen[screen.len() - 1].starts_with(" ✦ tome"),
        "status line should be on the last row: {screen:#?}"
    );
    let (col, row) = tome.cursor();
    assert!(col < harness::COLS && row < ROWS);

    tome.send_keys("ctrl+q");
    let status = tome.wait_exit(Duration::from_secs(5));
    assert!(status.success(), "tome exited with {status:?}");
}
