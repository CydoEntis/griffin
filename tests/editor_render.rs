mod harness;

use std::fs;
use std::time::Duration;

use harness::{COLS, Glyph, ROWS};

const START: Duration = Duration::from_secs(10);
const EXIT: Duration = Duration::from_secs(5);
const FIXTURE: &str = "tests/fixtures/render.txt";

fn status_line(glyph: &Glyph) -> String {
    glyph.screen()[usize::from(ROWS) - 1].clone()
}

fn quit(glyph: &mut Glyph) {
    glyph.send_keys("ctrl+q");
    let status = glyph.wait_exit(EXIT);
    assert!(status.success(), "glyph exited with {status:?}");
}

/// Column of `text` on `row`, panicking with the screen when it's missing.
fn col_of(glyph: &Glyph, row: u16, text: &str) -> u16 {
    glyph
        .text_col(row, text)
        .unwrap_or_else(|| panic!("{text:?} not on row {row}: {:#?}", glyph.screen()))
}

#[test]
fn opens_file() {
    let mut glyph = Glyph::spawn(&[FIXTURE]);
    glyph.wait_for_text("fn main() {", START);

    let text_col = col_of(&glyph, 3, "fn main() {");
    // Each line's number sits in the same right-aligned column, left of the text.
    for (row, number) in (3u16..7).zip(["1", "2", "3", "4"]) {
        let number_col = col_of(&glyph, row, number);
        assert!(number_col < text_col, "row {row}: {:#?}", glyph.screen());
        assert_eq!(
            number_col,
            col_of(&glyph, 3, "1"),
            "gutter misaligned: {:#?}",
            glyph.screen()
        );
    }
    let status = status_line(&glyph);
    assert!(status.contains("render.txt"), "status line: {status:?}");

    quit(&mut glyph);
}

#[test]
fn tabs_and_wide_chars() {
    let mut glyph = Glyph::spawn(&[FIXTURE]);
    glyph.wait_for_text("😀 ok", START);

    let base = col_of(&glyph, 3, "fn main() {");
    // `\tlet`: the tab fills to column 4 of the text area.
    assert_eq!(col_of(&glyph, 4, "let x = 1;"), base + 4);
    // `日本語 ok`: three 2-wide characters and a space.
    assert_eq!(col_of(&glyph, 5, "日本語"), base);
    assert_eq!(col_of(&glyph, 5, "ok"), base + 7);
    // `😀 ok`: one 2-wide emoji and a space.
    assert_eq!(col_of(&glyph, 6, "😀"), base);
    assert_eq!(col_of(&glyph, 6, "ok"), base + 3);

    quit(&mut glyph);
}

#[test]
fn long_line_is_cut() {
    let dir = tempfile::tempdir().unwrap();
    let long = format!("start{}END", "x".repeat(300));
    fs::write(dir.path().join("long.txt"), format!("{long}\nsecond\n")).unwrap();

    let mut glyph = Glyph::spawn_in(dir.path(), &["long.txt"]);
    glyph.wait_for_text("second", START);

    let screen = glyph.screen();
    // The first line fills the row to the right edge and stops there.
    assert_eq!(screen[3].chars().count(), usize::from(COLS), "{screen:#?}");
    assert!(screen[3].ends_with('x'), "{screen:#?}");
    assert!(!screen.iter().any(|row| row.contains("END")), "{screen:#?}");
    // Nothing wrapped: the next row is the file's second line.
    assert!(screen[4].contains("second"), "{screen:#?}");
    assert!(screen[4].trim_start().starts_with('2'), "{screen:#?}");

    quit(&mut glyph);
}

#[test]
fn no_path_opens_untitled() {
    let mut glyph = Glyph::spawn(&[]);
    glyph.wait_for_text("untitled", START);

    let screen = glyph.screen();
    assert!(status_line(&glyph).contains("untitled"), "{screen:#?}");
    // The tab header (blank, pill, thread), one empty line numbered 1, and
    // nothing else.
    assert_eq!(screen[0].trim(), "", "{screen:#?}");
    assert_eq!(screen[1].trim(), "▐ untitled ▌", "{screen:#?}");
    assert!(screen[2].chars().all(|c| c == '─'), "{screen:#?}");
    assert_eq!(screen[3].trim(), "1 │", "{screen:#?}");
    assert!(
        screen[4..usize::from(ROWS) - 1]
            .iter()
            .all(|row| row.is_empty())
    );

    quit(&mut glyph);
}

#[test]
fn missing_file_opens_empty_with_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut glyph = Glyph::spawn_in(dir.path(), &["missing.txt"]);
    glyph.wait_for_text("missing.txt", START);

    let screen = glyph.screen();
    assert!(status_line(&glyph).contains("missing.txt"), "{screen:#?}");
    assert_eq!(screen[3].trim(), "1 │", "{screen:#?}");
    // Opening alone never creates the file; saving (#7) does.
    assert!(!dir.path().join("missing.txt").exists());

    quit(&mut glyph);
}

#[test]
fn non_utf8_file_is_not_opened() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("bad.txt"), [b'o', b'k', 0xff, 0xfe, b'\n']).unwrap();

    let mut glyph = Glyph::spawn_in(dir.path(), &["bad.txt"]);
    glyph.wait_for_text("cannot open bad.txt: not UTF-8", START);

    let screen = glyph.screen();
    assert!(
        status_line(&glyph).contains("cannot open bad.txt: not UTF-8"),
        "{screen:#?}"
    );
    assert_eq!(screen[3].trim(), "1 │", "{screen:#?}");

    quit(&mut glyph);
}
