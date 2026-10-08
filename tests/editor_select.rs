mod harness;

use std::fs;
use std::time::Duration;

use harness::{Glyph, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);

/// Waits for the status line to show `Ln <line>, Col <col>` (see `shows_position`).
fn wait_for_position(glyph: &Glyph, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    glyph.wait_for_text(&position, WAIT);
    let status = glyph.screen()[usize::from(ROWS - 1)].clone();
    assert!(
        harness::shows_position(&status, &position),
        "status line {status:?} should show {position:?}"
    );
}

/// Waits until the editor rows read exactly `lines` (after the gutter), with
/// nothing numbered below them.
fn wait_for_lines(glyph: &Glyph, lines: &[&str]) {
    let last = format!("{}  {}", lines.len(), lines[lines.len() - 1]);
    glyph.wait_for_text(last.trim_end(), WAIT);
    // The next line's gutter: its number right-aligned in three cells.
    let next = format!(" {:>3}", lines.len() + 1);
    glyph.wait_for_text_gone(&next, WAIT);
    let screen = glyph.screen();
    for (row, text) in lines.iter().enumerate() {
        let expected = format!("{}  {text}", row + 1);
        // Rows 0-2 are the tab header.
        assert_eq!(screen[row + 3].trim(), expected.trim_end(), "{screen:#?}");
    }
}

/// Opens a scratch file holding `text`.
fn open(text: &str) -> (tempfile::TempDir, Glyph) {
    open_as("select.txt", text)
}

/// Opens a scratch file called `name` holding `text`.
fn open_as(name: &str, text: &str) -> (tempfile::TempDir, Glyph) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let path = dir.path().join(name);
    fs::write(&path, text).expect("write fixture");
    let path = path.to_str().expect("temp path is UTF-8").to_string();
    let glyph = Glyph::spawn_with_config("", &[&path]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    (dir, glyph)
}

#[test]
fn shift_right_selects() {
    let (_dir, mut glyph) = open("hello world");
    wait_for_lines(&glyph, &["hello world"]);
    assert_eq!(glyph.reversed_text(3), "");

    for _ in 0..5 {
        glyph.send_keys("shift+right");
    }
    wait_for_position(&glyph, 1, 6);
    glyph.wait_for_reversed(3, "hello", WAIT);

    // A plain movement drops the selection.
    glyph.send_keys("right");
    wait_for_position(&glyph, 1, 7);
    glyph.wait_for_reversed(3, "", WAIT);

    // Select it again and type over it.
    glyph.send_keys("home");
    for _ in 0..5 {
        glyph.send_keys("shift+right");
    }
    glyph.wait_for_reversed(3, "hello", WAIT);
    glyph.type_text("bye");
    wait_for_lines(&glyph, &["bye world"]);
    glyph.wait_for_reversed(3, "", WAIT);
    wait_for_position(&glyph, 1, 4);

    // Select all covers every line.
    glyph.send_keys("ctrl+a");
    glyph.wait_for_reversed(3, "bye world", WAIT);
}

#[test]
fn paste_is_one_undo_step() {
    let (_dir, mut glyph) = open("ab");
    wait_for_lines(&glyph, &["ab"]);
    glyph.send_keys("end");
    wait_for_position(&glyph, 1, 3);

    // Terminals send a pasted line break as CR, as if Enter were pressed.
    glyph.write(b"\x1b[200~one\rtwo\x1b[201~");
    wait_for_lines(&glyph, &["abone", "two"]);
    wait_for_position(&glyph, 2, 4);

    glyph.send_keys("ctrl+z");
    wait_for_lines(&glyph, &["ab"]);
    wait_for_position(&glyph, 1, 3);

    // A paste replaces the selection, still as one step.
    glyph.send_keys("ctrl+a");
    glyph.wait_for_reversed(3, "ab", WAIT);
    glyph.write(b"\x1b[200~xyz\x1b[201~");
    wait_for_lines(&glyph, &["xyz"]);
    glyph.send_keys("ctrl+z");
    wait_for_lines(&glyph, &["ab"]);
}

#[test]
fn selection_keeps_the_syntax_colours_on_sel() {
    /// hydra's `sel` and `keyword`.
    const SEL: vt100::Color = vt100::Color::Rgb(0x2a, 0x3a, 0x4c);
    const KEYWORD: vt100::Color = vt100::Color::Rgb(0xa5, 0x93, 0xff);
    /// Text starts after the gutter `   1  `.
    const TEXT_X: u16 = 6;
    let (_dir, mut glyph) = open_as("select.rs", "fn main() {}\nx");
    glyph.wait_for_fg_at(TEXT_X, 3, KEYWORD, WAIT);

    // The first line and its line break.
    glyph.send_keys("shift+down");
    wait_for_position(&glyph, 2, 1);
    glyph.wait_for_reversed(3, "fn main() {} ", WAIT);
    assert_eq!(glyph.reversed_text(4), "");
    // Selection is reverse video over swapped colours, so a cell's background
    // slot holds the colour its text shows in and the foreground slot `sel`:
    // `fn` keeps the keyword colour on `sel`.
    assert_eq!(glyph.fg_at(TEXT_X, 3), SEL);
    assert_eq!(glyph.bg_at(TEXT_X, 3), KEYWORD);
    // The line break is one `sel` cell past the line's end. Its text colour
    // isn't checked: a blank shows none, and ConPTY doesn't keep it for blanks.
    let past_end = TEXT_X + 12;
    assert_eq!(glyph.fg_at(past_end, 3), SEL);
}
