mod harness;

use std::time::Duration;

use harness::{Griffin, ROWS};

#[test]
fn starts_draws_the_status_line_and_quits_on_ctrl_q() {
    let mut griffin = Griffin::spawn(&[]);

    griffin.wait_for_text("griffin", Duration::from_secs(10));
    let screen = griffin.screen();
    assert_eq!(screen.len(), usize::from(ROWS));
    assert!(
        screen[screen.len() - 1].starts_with("griffin"),
        "status line should be on the last row: {screen:#?}"
    );
    let (col, row) = griffin.cursor();
    assert!(col < harness::COLS && row < ROWS);

    griffin.send_keys("ctrl+q");
    let status = griffin.wait_exit(Duration::from_secs(5));
    assert!(status.success(), "griffin exited with {status:?}");
}
