mod harness;

use std::time::Duration;

use harness::{Glyph, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// Three `foo`s: `one foo`, `two foo three`, `foo`.
const FIND: &str = "tests/fixtures/find.txt";
/// `foo Foo FOO` then `foooo`: four `foo`s ignoring case, two matching it.
const FIND_CASE: &str = "tests/fixtures/find_case.txt";
/// The default theme's accent, hydra's lime `#c3f53c`.
const ACCENT: vt100::Color = vt100::Color::Rgb(0xc3, 0xf5, 0x3c);
/// hydra's `bg`, which the current match's text is drawn in.
const BG: vt100::Color = vt100::Color::Rgb(0x07, 0x0b, 0x10);
/// hydra's `warn`, behind the current match.
const WARN: vt100::Color = vt100::Color::Rgb(0xff, 0xb5, 0x47);
/// hydra's `find_match_bg`, `mix(bg, warn, .3)` (SPEC_V1_LAYOUT §13), behind
/// the other matches.
const MATCH: vt100::Color = vt100::Color::Rgb(0x51, 0x3e, 0x21);
/// The find bar sits on the row above the status line.
const BAR_ROW: u16 = ROWS - 2;
/// Text starts after the gutter `   1  `.
const TEXT_X: u16 = 6;

fn open(path: &str) -> Glyph {
    let glyph = Glyph::spawn(&[path]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph
}

fn row(glyph: &Glyph, row: u16) -> String {
    glyph.screen()[usize::from(row)].clone()
}

/// Waits for the find bar to end with `count`, as `n/m` or an error.
fn wait_for_count(glyph: &Glyph, count: &str) {
    glyph.wait_for_screen(&format!("the find bar to show {count:?}"), WAIT, |lines| {
        let bar = &lines[usize::from(BAR_ROW)];
        bar.starts_with("Find:") && bar.ends_with(&format!("  {count}"))
    });
}

/// Waits for the status line to show `Ln <line>, Col <col>` (see `shows_position`).
fn wait_for_position(glyph: &Glyph, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    glyph.wait_for_screen(&position, WAIT, |lines| {
        harness::shows_position(&lines[usize::from(ROWS - 1)], &position)
    });
}

fn find(glyph: &mut Glyph, text: &str) {
    glyph.send_keys("ctrl+f");
    glyph.wait_for_text("Find:", WAIT);
    glyph.type_text(text);
    // A prefix of the text may already show the same count, so wait for all of it.
    glyph.wait_for_text(&format!("Find: {text}"), WAIT);
}

#[test]
fn ctrl_f_highlights_every_match_as_you_type_and_jumps_to_the_first() {
    let mut glyph = open(FIND);
    glyph.wait_for_text("3  foo", WAIT);
    find(&mut glyph, "foo");
    wait_for_count(&glyph, "1/3");
    assert!(row(&glyph, BAR_ROW).starts_with("Find: foo"));
    // The bar sits above the status line, which stays on the last row.
    assert!(row(&glyph, ROWS - 1).starts_with(" ✦ glyph"));
    // The current match is `bg` on `warn`, the others on `find_match_bg`, and
    // nothing else has either.
    glyph.wait_for_bg_text(3, WARN, "foo", WAIT);
    glyph.wait_for_bg_text(4, MATCH, "foo", WAIT);
    glyph.wait_for_bg_text(5, MATCH, "foo", WAIT);
    assert_eq!(glyph.bg_text(3, MATCH), "");
    assert_eq!(glyph.text_col(3, "foo"), Some(TEXT_X + 4));
    assert_eq!(glyph.fg_at(TEXT_X + 4, 3), BG);
    // The other matches keep the text's own colour.
    let plain = glyph.fg_at(TEXT_X, 4);
    assert_eq!(glyph.fg_at(TEXT_X + 4, 4), plain);
    // Matches aren't drawn as a selection.
    for row in 3..=5 {
        assert_eq!(glyph.reversed_text(row), "");
    }
    wait_for_position(&glyph, 1, 5);
    // The terminal cursor is in the bar, after the text.
    glyph.wait_for_cursor(9, BAR_ROW, WAIT);

    // Matches update while typing: only line 2 has `foo` before a space.
    glyph.type_text(" ");
    wait_for_count(&glyph, "1/1");
    glyph.wait_for_bg_text(4, WARN, "foo ", WAIT);
    glyph.wait_for_bg_text(3, MATCH, "", WAIT);
    glyph.wait_for_bg_text(3, WARN, "", WAIT);
    glyph.wait_for_bg_text(5, MATCH, "", WAIT);
    wait_for_position(&glyph, 2, 5);
    glyph.send_keys("backspace");
    wait_for_count(&glyph, "1/3");
}

#[test]
fn ctrl_f_starts_from_the_selection() {
    let mut glyph = open(FIND);
    glyph.wait_for_text("3  foo", WAIT);
    glyph.send_keys("down");
    for _ in 0..3 {
        glyph.send_keys("shift+right");
    }
    glyph.wait_for_reversed(4, "two", WAIT);
    glyph.send_keys("ctrl+f");
    wait_for_count(&glyph, "1/1");
    assert!(row(&glyph, BAR_ROW).starts_with("Find: two"));
    glyph.wait_for_bg_text(4, WARN, "two", WAIT);
    wait_for_position(&glyph, 2, 1);
}

#[test]
fn enter_and_shift_enter_step_through_matches_wrapping() {
    let mut glyph = open(FIND);
    find(&mut glyph, "foo");
    wait_for_count(&glyph, "1/3");
    wait_for_position(&glyph, 1, 5);

    glyph.send_keys("enter");
    wait_for_count(&glyph, "2/3");
    wait_for_position(&glyph, 2, 5);
    glyph.send_keys("enter");
    wait_for_count(&glyph, "3/3");
    wait_for_position(&glyph, 3, 1);
    glyph.send_keys("enter");
    wait_for_count(&glyph, "1/3");
    wait_for_position(&glyph, 1, 5);

    glyph.send_keys("shift+enter");
    wait_for_count(&glyph, "3/3");
    wait_for_position(&glyph, 3, 1);
    glyph.send_keys("shift+enter");
    wait_for_count(&glyph, "2/3");
    wait_for_position(&glyph, 2, 5);
    // Enter in the bar never edits the buffer: still three lines plus the empty
    // one after the final line break.
    glyph.wait_for_text("3  foo", WAIT);
    // No fifth line number in the gutter.
    assert!(!glyph.screen().join("\n").contains("   5"));
}

#[test]
fn alt_c_toggles_case_and_alt_r_toggles_regex_and_a_bad_regex_says_so() {
    let mut glyph = open(FIND_CASE);
    glyph.wait_for_text("2  foooo", WAIT);
    find(&mut glyph, "foo");
    wait_for_count(&glyph, "1/4");
    glyph.wait_for_bg_text(3, WARN, "foo", WAIT);
    glyph.wait_for_bg_text(3, MATCH, "FooFOO", WAIT);
    glyph.wait_for_fg(BAR_ROW, ACCENT, "", WAIT);

    glyph.send_keys("alt+c");
    wait_for_count(&glyph, "1/2");
    glyph.wait_for_fg(BAR_ROW, ACCENT, "Aa", WAIT);
    glyph.wait_for_bg_text(3, MATCH, "", WAIT);
    glyph.wait_for_bg_text(3, WARN, "foo", WAIT);
    glyph.wait_for_bg_text(4, MATCH, "foo", WAIT);
    glyph.send_keys("alt+c");
    wait_for_count(&glyph, "1/4");
    glyph.wait_for_fg(BAR_ROW, ACCENT, "", WAIT);

    // `fo+` is literal text until regex is on.
    for _ in 0..3 {
        glyph.send_keys("backspace");
    }
    glyph.type_text("fo+");
    wait_for_count(&glyph, "0/0");
    glyph.wait_for_bg_text(3, WARN, "", WAIT);
    glyph.wait_for_bg_text(3, MATCH, "", WAIT);
    glyph.send_keys("alt+r");
    wait_for_count(&glyph, "1/4");
    glyph.wait_for_fg(BAR_ROW, ACCENT, ".*", WAIT);
    glyph.wait_for_bg_text(4, MATCH, "foooo", WAIT);

    // An unclosed group is an error, with nothing highlighted.
    glyph.type_text("(");
    wait_for_count(&glyph, "invalid regex");
    for row in 3..=4 {
        glyph.wait_for_bg_text(row, WARN, "", WAIT);
        glyph.wait_for_bg_text(row, MATCH, "", WAIT);
    }
    glyph.send_keys("backspace");
    wait_for_count(&glyph, "1/4");
}

#[test]
fn esc_closes_the_bar_and_keeps_the_cursor_on_the_current_match() {
    let mut glyph = open(FIND);
    find(&mut glyph, "foo");
    wait_for_count(&glyph, "1/3");
    glyph.send_keys("enter");
    wait_for_count(&glyph, "2/3");

    glyph.send_keys("esc");
    glyph.wait_for_text_gone("Find:", WAIT);
    for row in 3..=4 {
        glyph.wait_for_bg_text(row, WARN, "", WAIT);
        glyph.wait_for_bg_text(row, MATCH, "", WAIT);
    }
    wait_for_position(&glyph, 2, 5);
    glyph.wait_for_cursor(TEXT_X + 4, 4, WAIT);
    // Typing goes into the buffer again, at the match.
    glyph.type_text("x");
    glyph.wait_for_text("2  two xfoo three", WAIT);
}

#[test]
fn mono_underlines_matches_and_reverses_the_current_one() {
    // `mono` can't blend `find_match_bg`, so matches are told apart by weight.
    let mut glyph = Glyph::spawn_with_config("theme = \"mono\"\n", &[FIND]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph.wait_for_text("3  foo", WAIT);
    find(&mut glyph, "foo");
    wait_for_count(&glyph, "1/3");
    glyph.wait_for_reversed(3, "foo", WAIT);
    glyph.wait_for_underlined(4, "foo", WAIT);
    glyph.wait_for_underlined(5, "foo", WAIT);
    assert_eq!(glyph.underlined_text(3), "");
    assert_eq!(glyph.reversed_text(4), "");

    glyph.send_keys("enter");
    wait_for_count(&glyph, "2/3");
    glyph.wait_for_reversed(4, "foo", WAIT);
    glyph.wait_for_underlined(3, "foo", WAIT);
    glyph.wait_for_reversed(3, "", WAIT);
}
