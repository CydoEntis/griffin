mod harness;

use std::fs;
use std::time::Duration;

use harness::{COLS, Glyph, ROWS};
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const EXIT: Duration = Duration::from_secs(5);
const WAIT: Duration = Duration::from_secs(5);
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
    assert_eq!(screen[3].trim(), "1", "{screen:#?}");
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
    assert_eq!(screen[3].trim(), "1", "{screen:#?}");
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
    assert_eq!(screen[3].trim(), "1", "{screen:#?}");

    quit(&mut glyph);
}

// The `aurora` roles the gutter and cursor line use (README §4.1).
const AURORA_BG: Color = Color::Rgb(0x0b, 0x0a, 0x10);
const AURORA_ACCENT: Color = Color::Rgb(0xb6, 0x9c, 0xff);
const AURORA_CUR_LINE: Color = Color::Rgb(0x13, 0x11, 0x1c);
const AURORA_GUTTER: Color = Color::Rgb(0x3a, 0x35, 0x50);
const AURORA_TEXT: Color = Color::Rgb(0xa6, 0xa2, 0xbb);
const AURORA_COMMENT: Color = Color::Rgb(0x5c, 0x57, 0x73);
const AURORA: &str = "theme = \"aurora\"\n";
/// The first editor row, below the tab header.
const TOP: u16 = 3;

/// `a` moved `t` of the way to `b`, per channel, rounded, as the editor mixes.
fn mix(a: Color, b: Color, t: f64) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (a, b) else {
        panic!("can't mix {a:?} and {b:?}");
    };
    let m = |p: u8, q: u8| (f64::from(p) + (f64::from(q) - f64::from(p)) * t).round() as u8;
    Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
}

/// The cursor row's bg `dx` cells into an editor `width` wide (README §2.4).
fn glow(dx: u16, width: u16) -> Color {
    let stops = [
        mix(AURORA_BG, AURORA_ACCENT, 0.20),
        mix(AURORA_BG, AURORA_ACCENT, 0.07),
        AURORA_CUR_LINE,
        AURORA_CUR_LINE,
    ];
    let pos = (f64::from(dx) / (f64::from(width) * 0.8)).clamp(0.0, 1.0) * 3.0;
    let i = (pos.floor() as usize).min(2);
    mix(stops[i], stops[i + 1], pos - i as f64)
}

#[test]
fn gutter_is_a_mark_cell_a_three_cell_number_and_two_blanks() {
    let mut glyph = Glyph::spawn(&[FIXTURE]);
    glyph.wait_for_text("fn main() {", START);

    let screen = glyph.screen();
    // One cell for the diagnostic mark, the number right-aligned in three, two
    // blanks, then the text: no divider rule.
    assert!(screen[3].starts_with("   1  fn main() {"), "{screen:#?}");
    assert!(screen[5].starts_with("   3  日本語 ok"), "{screen:#?}");
    assert_eq!(col_of(&glyph, TOP, "fn main() {"), 6);
    let editor = &screen[3..usize::from(ROWS) - 1];
    assert!(editor.iter().all(|row| !row.contains('│')), "{screen:#?}");

    quit(&mut glyph);
}

#[test]
fn the_focused_cursor_row_glows_and_lights_its_number() {
    let mut glyph = Glyph::spawn_with_config(AURORA, &[FIXTURE]);
    glyph.wait_for_text("fn main() {", START);

    // Bright out of the gutter, fading into `cur_line` across the row.
    glyph.wait_for_bg(0, TOP, mix(AURORA_BG, AURORA_ACCENT, 0.20), START);
    assert_eq!(glyph.bg_at(3, TOP), glow(3, COLS));
    assert_eq!(glyph.bg_at(COLS / 2, TOP), glow(COLS / 2, COLS));
    assert_ne!(glyph.bg_at(COLS / 2, TOP), AURORA_CUR_LINE);
    assert_eq!(glyph.bg_at(COLS - 1, TOP), AURORA_CUR_LINE);
    // Other rows stay on the ground.
    assert_eq!(glyph.bg_at(0, TOP + 1), AURORA_BG);
    assert_eq!(glyph.bg_at(COLS / 2, TOP + 1), AURORA_BG);
    // The cursor line's number is `accent` bold; the rest are `gutter`.
    assert_eq!(glyph.fg_at(3, TOP), AURORA_ACCENT);
    assert!(glyph.bold_at(3, TOP));
    assert_eq!(glyph.fg_at(3, TOP + 1), AURORA_GUTTER);
    assert!(!glyph.bold_at(3, TOP + 1));

    // The glow follows the cursor.
    glyph.send_keys("down");
    glyph.wait_for_bg(0, TOP + 1, glow(0, COLS), WAIT);
    assert_eq!(glyph.bg_at(0, TOP), AURORA_BG);
    assert_eq!(glyph.fg_at(3, TOP + 1), AURORA_ACCENT);

    quit(&mut glyph);
}

#[test]
fn an_unfocused_split_has_no_glow_and_a_text_number() {
    let mut glyph = Glyph::spawn_with_config(AURORA, &[FIXTURE]);
    glyph.wait_for_text("fn main() {", START);
    // Two splits: 0..50, the divider, then the new, focused one at 51..100.
    glyph.send_keys("alt+v");
    let right = 51;
    glyph.wait_for_bg(right, TOP, glow(0, COLS - right), WAIT);
    assert_eq!(glyph.bg_at(right + 25, TOP), glow(25, COLS - right));
    assert_eq!(glyph.fg_at(right + 3, TOP), AURORA_ACCENT);
    assert!(glyph.bold_at(right + 3, TOP));

    // The left split shows the same cursor row unlit.
    glyph.wait_for_bg(0, TOP, AURORA_BG, WAIT);
    assert_eq!(glyph.bg_at(25, TOP), AURORA_BG);
    assert_eq!(glyph.fg_at(3, TOP), AURORA_TEXT);
    assert!(!glyph.bold_at(3, TOP));

    quit(&mut glyph);
}

#[test]
fn comments_are_italic() {
    let mut glyph = Glyph::spawn_with_config(AURORA, &["tests/fixtures/highlight/sample.rs"]);
    glyph.wait_for_text("// A sample for the highlight tests.", START);

    let comment = col_of(&glyph, TOP, "// A sample");
    // Highlighting may land a frame after the text; the comment colour says it has.
    glyph.wait_for_fg_at(comment, TOP, AURORA_COMMENT, WAIT);
    assert!(glyph.italic_at(comment, TOP));
    assert!(glyph.italic_at(comment + 5, TOP));
    // Code is upright.
    let code = col_of(&glyph, TOP + 1, "use std::fmt;");
    assert!(!glyph.italic_at(code, TOP + 1));

    quit(&mut glyph);
}

#[test]
fn mono_has_no_glow_but_a_bold_number() {
    let mut glyph = Glyph::spawn_with_config("theme = \"mono\"\n", &[FIXTURE]);
    glyph.wait_for_text("fn main() {", START);

    assert!(glyph.screen()[3].starts_with("   1  fn main() {"));
    assert!(glyph.bold_at(3, TOP));
    assert!(!glyph.bold_at(3, TOP + 1));
    assert_eq!(glyph.bg_at(0, TOP), Color::Default);
    assert_eq!(glyph.bg_at(COLS / 2, TOP), Color::Default);

    quit(&mut glyph);
}
