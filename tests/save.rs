mod harness;

use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{Glyph, ROWS};
use tempfile::TempDir;
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const PROMPT: &str = "a.txt has unsaved changes";
const EXPLANATION: &str = "Closing discards them unless you save.";

/// The default theme, hydra: `bg`, `scrim`, `accent`, `accent2`, `acc_ink`,
/// `strong` and `raised2`.
const BG: Color = Color::Rgb(0x07, 0x0b, 0x10);
const SCRIM: Color = Color::Rgb(0x04, 0x06, 0x08);
const ACCENT: Color = Color::Rgb(0xc3, 0xf5, 0x3c);
const ACCENT2: Color = Color::Rgb(0x5a, 0xa9, 0xff);
const ACC_INK: Color = Color::Rgb(0x0a, 0x12, 0x04);
const STRONG: Color = Color::Rgb(0xf2, 0xf6, 0xf8);
const RAISED2: Color = Color::Rgb(0x18, 0x24, 0x2f);

/// At 100x30 the card is 46 wide (the explanation's 38 cells + 8) and 7 tall,
/// centred (SPEC_V1_LAYOUT §7.4): its lit edge on row 11 from column 27, the
/// question and explanation at column 30, and the buttons on row 15:
/// ` Save ` at 30, ` Discard ` at 38, ` Cancel ` at 49, and `esc` at 67.
const CARD_X: u16 = 27;
const CARD_Y: u16 = 11;
const TEXT_X: u16 = 30;
const BUTTON_ROW: u16 = 15;
const SAVE_X: u16 = 30;
const DISCARD_X: u16 = 38;
const CANCEL_X: u16 = 49;

/// `a` moved `t` of the way to `b`, per channel, rounded, as the editor mixes.
fn mix(a: Color, b: Color, t: f64) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (a, b) else {
        panic!("can't mix {a:?} and {b:?}");
    };
    let m = |p: u8, q: u8| (f64::from(p) + (f64::from(q) - f64::from(p)) * t).round() as u8;
    Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
}

fn status_line(glyph: &Glyph) -> String {
    glyph.screen()[usize::from(ROWS - 1)].clone()
}

/// Opens `a.txt` holding `contents` in a fresh temp dir, with glyph running there.
fn open_a(contents: &[u8]) -> (TempDir, Glyph) {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join("a.txt"), contents).expect("write a.txt");
    let glyph = Glyph::spawn_in(dir.path(), &["a.txt"]);
    glyph.wait_for_text("a.txt", START);
    glyph.wait_for_text("Ln 1, Col 1", START);
    (dir, glyph)
}

fn read_a(dir: &Path) -> Vec<u8> {
    fs::read(dir.join("a.txt")).expect("read a.txt")
}

/// Types `x` and waits for the dirty marker.
fn make_dirty(glyph: &mut Glyph) {
    glyph.type_text("x");
    glyph.wait_for_text("a.txt •", WAIT);
}

/// Ctrl+Q, then checks the card's copy and buttons sit where §7.4 puts them
/// over a dimmed screen.
fn open_quit_prompt(glyph: &mut Glyph) {
    glyph.send_keys("ctrl+q");
    glyph.wait_for_text(PROMPT, WAIT);
    let screen = glyph.screen();
    assert_eq!(
        glyph.text_col(CARD_Y, &"▀".repeat(46)),
        Some(CARD_X),
        "{screen:#?}"
    );
    assert_eq!(
        glyph.text_col(CARD_Y + 1, PROMPT),
        Some(TEXT_X),
        "{screen:#?}"
    );
    assert_eq!(
        glyph.text_col(CARD_Y + 2, EXPLANATION),
        Some(TEXT_X),
        "{screen:#?}"
    );
    assert_eq!(
        glyph.text_col(BUTTON_ROW, "Save    Discard    Cancel"),
        Some(SAVE_X + 1),
        "{screen:#?}"
    );
    assert_eq!(
        glyph.text_col(BUTTON_ROW, "esc"),
        Some(CARD_X + 46 - 6),
        "{screen:#?}"
    );
    // Below the text the editor is dimmed towards the scrim.
    assert_eq!(glyph.bg_at(50, 22), mix(BG, SCRIM, 0.6));
}

fn assert_exits(glyph: &mut Glyph) {
    let status = glyph.wait_exit(WAIT);
    assert!(status.success(), "glyph exited with {status:?}");
}

#[test]
fn the_save_message_holds_the_path_slot_until_the_next_key() {
    // aurora's `ok` and `strong`.
    const OK: vt100::Color = vt100::Color::Rgb(0x7f, 0xe3, 0xc9);
    const STRONG: vt100::Color = vt100::Color::Rgb(0xf4, 0xf2, 0xfb);
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join("a.txt"), "hello").expect("write a.txt");
    let mut glyph = Glyph::spawn_in_with_config(dir.path(), "theme = \"aurora\"\n", &["a.txt"]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    let status_row = ROWS - 1;
    assert_eq!(glyph.text_col(status_row, "a.txt"), Some(20));

    glyph.type_text("x");
    glyph.send_keys("ctrl+s");
    glyph.wait_for_text("✓ saved a.txt", WAIT);
    assert_eq!(glyph.text_col(status_row, "✓ saved a.txt"), Some(20));
    assert_eq!(glyph.fg_at(20, status_row), OK);

    // Any key gives the slot back to the path.
    glyph.send_keys("left");
    glyph.wait_for_text("Ln 1, Col 1", WAIT);
    glyph.wait_for_text_gone("saved a.txt", WAIT);
    assert_eq!(glyph.text_col(status_row, "a.txt"), Some(20));
    assert_eq!(glyph.fg_at(20, status_row), STRONG);
}

#[test]
fn ctrl_s_saves() {
    let (dir, mut glyph) = open_a(b"hello\r\nworld");
    assert!(!status_line(&glyph).contains('●'));

    make_dirty(&mut glyph);
    glyph.send_keys("ctrl+s");
    glyph.wait_for_text("saved a.txt", WAIT);
    let status = status_line(&glyph);
    assert!(!status.contains('●'), "still dirty: {status:?}");
    // CRLF and the missing final newline survive the save.
    assert_eq!(read_a(dir.path()), b"xhello\r\nworld");

    // A clean buffer quits without asking.
    glyph.send_keys("ctrl+q");
    assert_exits(&mut glyph);
}

#[test]
fn ctrl_s_on_an_untitled_buffer_asks_for_a_path() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let mut glyph = Glyph::spawn_in(dir.path(), &[]);
    glyph.wait_for_text("untitled", START);
    glyph.type_text("x");
    glyph.wait_for_text("untitled •", WAIT);
    glyph.send_keys("ctrl+s");
    glyph.wait_for_text("Save as:", WAIT);
    // Esc leaves it unsaved.
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("Save as:", WAIT);
    assert!(status_line(&glyph).contains("untitled •"));
    assert_eq!(fs::read_dir(dir.path()).expect("list dir").count(), 0);

    glyph.send_keys("ctrl+q");
    glyph.wait_for_text("untitled has unsaved changes", WAIT);
    glyph.type_text("d");
    assert_exits(&mut glyph);
}

#[test]
fn quit_guard_save() {
    let (dir, mut glyph) = open_a(b"hello\n");
    make_dirty(&mut glyph);
    open_quit_prompt(&mut glyph);
    glyph.type_text("s");
    assert_exits(&mut glyph);
    assert_eq!(read_a(dir.path()), b"xhello\n");
}

#[test]
fn quit_guard_discard() {
    let (dir, mut glyph) = open_a(b"hello\n");
    make_dirty(&mut glyph);
    open_quit_prompt(&mut glyph);
    glyph.type_text("d");
    assert_exits(&mut glyph);
    assert_eq!(read_a(dir.path()), b"hello\n");
}

#[test]
fn quit_guard_cancel() {
    let (dir, mut glyph) = open_a(b"hello\n");
    make_dirty(&mut glyph);

    // C closes the prompt and editing carries on.
    open_quit_prompt(&mut glyph);
    glyph.type_text("c");
    glyph.wait_for_text_gone(PROMPT, WAIT);
    glyph.type_text("y");
    glyph.wait_for_text("xyhello", WAIT);

    // So does Esc.
    open_quit_prompt(&mut glyph);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(PROMPT, WAIT);
    glyph.type_text("z");
    glyph.wait_for_text("xyzhello", WAIT);
    assert_eq!(read_a(dir.path()), b"hello\n");

    open_quit_prompt(&mut glyph);
    glyph.type_text("d");
    assert_exits(&mut glyph);
}

#[test]
fn the_first_button_is_lit_and_key_letters_are_underlined() {
    let (_dir, mut glyph) = open_a(b"hello\n");
    make_dirty(&mut glyph);
    open_quit_prompt(&mut glyph);
    // ` Save ` runs accent → accent2 across its six cells, in acc_ink bold.
    assert_eq!(glyph.bg_at(SAVE_X, BUTTON_ROW), ACCENT);
    assert_eq!(glyph.bg_at(SAVE_X + 5, BUTTON_ROW), ACCENT2);
    assert_eq!(glyph.fg_at(SAVE_X + 1, BUTTON_ROW), ACC_INK);
    assert!(glyph.bold_at(SAVE_X + 1, BUTTON_ROW));
    // The others are strong on raised2.
    for x in [DISCARD_X, CANCEL_X] {
        assert_eq!(glyph.bg_at(x, BUTTON_ROW), RAISED2);
        assert_eq!(glyph.fg_at(x + 1, BUTTON_ROW), STRONG);
        assert!(!glyph.bold_at(x + 1, BUTTON_ROW));
    }
    assert_eq!(glyph.underlined_text(BUTTON_ROW), "SDC");
    glyph.type_text("d");
    assert_exits(&mut glyph);
}

#[test]
fn enter_presses_the_focused_button_which_starts_on_save() {
    let (dir, mut glyph) = open_a(b"hello\n");
    make_dirty(&mut glyph);
    open_quit_prompt(&mut glyph);
    glyph.send_keys("enter");
    assert_exits(&mut glyph);
    assert_eq!(read_a(dir.path()), b"xhello\n");
}

#[test]
fn arrows_and_tab_move_the_focus() {
    let (dir, mut glyph) = open_a(b"hello\n");
    make_dirty(&mut glyph);
    open_quit_prompt(&mut glyph);

    // Right lights Discard; Left goes back, and stops at the first.
    glyph.send_keys("right");
    glyph.wait_for_bg(DISCARD_X, BUTTON_ROW, ACCENT, WAIT);
    assert_eq!(glyph.bg_at(SAVE_X, BUTTON_ROW), RAISED2);
    glyph.send_keys("left");
    glyph.wait_for_bg(SAVE_X, BUTTON_ROW, ACCENT, WAIT);
    glyph.send_keys("left");
    glyph.send_keys("tab");
    glyph.wait_for_bg(DISCARD_X, BUTTON_ROW, ACCENT, WAIT);
    // Tab past the last wraps to the first.
    glyph.send_keys("tab");
    glyph.wait_for_bg(CANCEL_X, BUTTON_ROW, ACCENT, WAIT);
    glyph.send_keys("tab");
    glyph.wait_for_bg(SAVE_X, BUTTON_ROW, ACCENT, WAIT);
    glyph.send_keys("tab");
    glyph.wait_for_bg(DISCARD_X, BUTTON_ROW, ACCENT, WAIT);
    glyph.send_keys("tab");
    glyph.wait_for_bg(CANCEL_X, BUTTON_ROW, ACCENT, WAIT);

    // Enter on Cancel closes the card and editing carries on.
    glyph.send_keys("enter");
    glyph.wait_for_text_gone(PROMPT, WAIT);
    glyph.type_text("y");
    glyph.wait_for_text("xyhello", WAIT);

    // The card opens again with Save lit; Right, Enter discards.
    open_quit_prompt(&mut glyph);
    assert_eq!(glyph.bg_at(SAVE_X, BUTTON_ROW), ACCENT);
    glyph.send_keys("right");
    glyph.wait_for_bg(DISCARD_X, BUTTON_ROW, ACCENT, WAIT);
    glyph.send_keys("enter");
    assert_exits(&mut glyph);
    assert_eq!(read_a(dir.path()), b"hello\n");
}

#[test]
fn clicking_a_button_presses_it_and_outside_cancels() {
    let (dir, mut glyph) = open_a(b"hello\n");
    make_dirty(&mut glyph);

    // A click inside the card but off the buttons does nothing.
    open_quit_prompt(&mut glyph);
    glyph.click(TEXT_X, CARD_Y + 3);
    glyph.assert_running_for(Duration::from_millis(300));
    assert!(glyph.screen().iter().any(|line| line.contains(PROMPT)));
    // Outside it is Esc.
    glyph.click(5, 25);
    glyph.wait_for_text_gone(PROMPT, WAIT);

    open_quit_prompt(&mut glyph);
    glyph.click(DISCARD_X + 3, BUTTON_ROW);
    assert_exits(&mut glyph);
    assert_eq!(read_a(dir.path()), b"hello\n");
}
