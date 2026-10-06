mod harness;

use std::fs;
use std::time::Duration;

use harness::{COLS, Griffin, ROWS};

const START: Duration = Duration::from_secs(10);
const EXIT: Duration = Duration::from_secs(5);
const FIXTURE: &str = "tests/fixtures/render.txt";

fn status_line(griffin: &Griffin) -> String {
    griffin.screen()[usize::from(ROWS) - 1].clone()
}

fn quit(griffin: &mut Griffin) {
    griffin.send_keys("ctrl+q");
    let status = griffin.wait_exit(EXIT);
    assert!(status.success(), "griffin exited with {status:?}");
}

/// Column of `text` on `row`, panicking with the screen when it's missing.
fn col_of(griffin: &Griffin, row: u16, text: &str) -> u16 {
    griffin
        .text_col(row, text)
        .unwrap_or_else(|| panic!("{text:?} not on row {row}: {:#?}", griffin.screen()))
}

#[test]
fn opens_file() {
    let mut griffin = Griffin::spawn(&[FIXTURE]);
    griffin.wait_for_text("fn main() {", START);

    let text_col = col_of(&griffin, 0, "fn main() {");
    // Each line's number sits in the same right-aligned column, left of the text.
    for (row, number) in (0u16..4).zip(["1", "2", "3", "4"]) {
        let number_col = col_of(&griffin, row, number);
        assert!(number_col < text_col, "row {row}: {:#?}", griffin.screen());
        assert_eq!(
            number_col,
            col_of(&griffin, 0, "1"),
            "gutter misaligned: {:#?}",
            griffin.screen()
        );
    }
    let status = status_line(&griffin);
    assert!(status.contains("render.txt"), "status line: {status:?}");

    quit(&mut griffin);
}

#[test]
fn tabs_and_wide_chars() {
    let mut griffin = Griffin::spawn(&[FIXTURE]);
    griffin.wait_for_text("😀 ok", START);

    let base = col_of(&griffin, 0, "fn main() {");
    // `\tlet`: the tab fills to column 4 of the text area.
    assert_eq!(col_of(&griffin, 1, "let x = 1;"), base + 4);
    // `日本語 ok`: three 2-wide characters and a space.
    assert_eq!(col_of(&griffin, 2, "日本語"), base);
    assert_eq!(col_of(&griffin, 2, "ok"), base + 7);
    // `😀 ok`: one 2-wide emoji and a space.
    assert_eq!(col_of(&griffin, 3, "😀"), base);
    assert_eq!(col_of(&griffin, 3, "ok"), base + 3);

    quit(&mut griffin);
}

#[test]
fn long_line_is_cut() {
    let dir = tempfile::tempdir().unwrap();
    let long = format!("start{}END", "x".repeat(300));
    fs::write(dir.path().join("long.txt"), format!("{long}\nsecond\n")).unwrap();

    let mut griffin = Griffin::spawn_in(dir.path(), &["long.txt"]);
    griffin.wait_for_text("second", START);

    let screen = griffin.screen();
    // The first line fills the row to the right edge and stops there.
    assert_eq!(screen[0].chars().count(), usize::from(COLS), "{screen:#?}");
    assert!(screen[0].ends_with('x'), "{screen:#?}");
    assert!(!screen.iter().any(|row| row.contains("END")), "{screen:#?}");
    // Nothing wrapped: the next row is the file's second line.
    assert!(screen[1].contains("second"), "{screen:#?}");
    assert!(screen[1].trim_start().starts_with('2'), "{screen:#?}");

    quit(&mut griffin);
}

#[test]
fn no_path_opens_untitled() {
    let mut griffin = Griffin::spawn(&[]);
    griffin.wait_for_text("untitled", START);

    let screen = griffin.screen();
    assert!(status_line(&griffin).contains("untitled"), "{screen:#?}");
    // One empty line, numbered 1, and nothing else.
    assert_eq!(screen[0].trim(), "1 │", "{screen:#?}");
    assert!(
        screen[1..usize::from(ROWS) - 1]
            .iter()
            .all(|row| row.is_empty())
    );

    quit(&mut griffin);
}

#[test]
fn missing_file_opens_empty_with_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut griffin = Griffin::spawn_in(dir.path(), &["missing.txt"]);
    griffin.wait_for_text("missing.txt", START);

    let screen = griffin.screen();
    assert!(status_line(&griffin).contains("missing.txt"), "{screen:#?}");
    assert_eq!(screen[0].trim(), "1 │", "{screen:#?}");
    // Opening alone never creates the file; saving (#7) does.
    assert!(!dir.path().join("missing.txt").exists());

    quit(&mut griffin);
}

#[test]
fn non_utf8_file_is_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("bad.txt"), [b'o', b'k', 0xff, 0xfe, b'\n']).unwrap();

    let mut griffin = Griffin::spawn_in(dir.path(), &["bad.txt"]);
    griffin.wait_for_text("cannot open bad.txt: not UTF-8", START);

    let screen = griffin.screen();
    assert!(
        status_line(&griffin).contains("cannot open bad.txt: not UTF-8"),
        "{screen:#?}"
    );
    assert_eq!(screen[0].trim(), "1 │", "{screen:#?}");

    quit(&mut griffin);
}
