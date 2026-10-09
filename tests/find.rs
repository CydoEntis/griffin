mod harness;

use std::time::Duration;

use harness::{COLS, ROWS, Tome};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// Three `foo`s: `one foo`, `two foo three`, `foo`.
const FIND: &str = "tests/fixtures/find.txt";
/// `foo Foo FOO` then `foooo`: four `foo`s ignoring case, two matching it.
const FIND_CASE: &str = "tests/fixtures/find_case.txt";
/// The default theme's accent, hydra's lime `#c3f53c`.
const ACCENT: vt100::Color = vt100::Color::Rgb(0xc3, 0xf5, 0x3c);
/// hydra's `acc_ink`, the text on an `on` chip.
const ACC_INK: vt100::Color = vt100::Color::Rgb(0x0a, 0x12, 0x04);
/// hydra's `raised`, the bar's ground.
const RAISED: vt100::Color = vt100::Color::Rgb(0x0f, 0x18, 0x21);
/// hydra's `strong`, the pattern and the count.
const STRONG: vt100::Color = vt100::Color::Rgb(0xf2, 0xf6, 0xf8);
/// hydra's `muted`, an `off` chip.
const MUTED: vt100::Color = vt100::Color::Rgb(0x71, 0x80, 0x8f);
/// hydra's `err`, a bad pattern and why.
const ERR: vt100::Color = vt100::Color::Rgb(0xff, 0x6b, 0x6b);
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

fn open(path: &str) -> Tome {
    let tome = Tome::spawn(&[path]);
    tome.wait_for_text("Ln 1, Col 1", START);
    tome
}

fn row(tome: &Tome, row: u16) -> String {
    tome.screen()[usize::from(row)].clone()
}

/// Waits for the find bar to end with `count`, as `n/m` or an error.
fn wait_for_count(tome: &Tome, count: &str) {
    tome.wait_for_screen(&format!("the find bar to show {count:?}"), WAIT, |lines| {
        let bar = &lines[usize::from(BAR_ROW)];
        bar.starts_with(" Find  ") && bar.ends_with(&format!("  {count}"))
    });
}

/// Waits for the status line to show `Ln <line>, Col <col>` (see `shows_position`).
fn wait_for_position(tome: &Tome, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    tome.wait_for_screen(&position, WAIT, |lines| {
        harness::shows_position(&lines[usize::from(ROWS - 1)], &position)
    });
}

fn find(tome: &mut Tome, text: &str) {
    tome.send_keys("ctrl+f");
    tome.wait_for_text(" Find  ", WAIT);
    tome.type_text(text);
    // A prefix of the text may already show the same count, so wait for all of it.
    tome.wait_for_text(&format!(" Find  {text}"), WAIT);
}

#[test]
fn ctrl_f_highlights_every_match_as_you_type_and_jumps_to_the_first() {
    let mut tome = open(FIND);
    tome.wait_for_text("3  foo", WAIT);
    find(&mut tome, "foo");
    wait_for_count(&tome, "1/3");
    assert!(row(&tome, BAR_ROW).starts_with(" Find  foo"));
    assert!(row(&tome, BAR_ROW).ends_with(" Aa   .*   1/3"));
    // The whole bar is on `raised`, the pattern and count in `strong`, the
    // chips `muted` while off.
    assert_eq!(
        tome.bg_text(BAR_ROW, RAISED).chars().count(),
        usize::from(COLS)
    );
    assert_eq!(tome.fg_at(7, BAR_ROW), STRONG);
    let count = tome.text_col(BAR_ROW, "1/3").expect("the count shows");
    assert_eq!(tome.fg_at(count, BAR_ROW), STRONG);
    let chip = tome.text_col(BAR_ROW, "Aa").expect("the case chip shows");
    assert_eq!(tome.fg_at(chip, BAR_ROW), MUTED);
    // The bar sits above the status line, which stays on the last row.
    assert!(row(&tome, ROWS - 1).starts_with(" ✦ tome"));
    // The current match is `bg` on `warn`, the others on `find_match_bg`, and
    // nothing else has either.
    tome.wait_for_bg_text(3, WARN, "foo", WAIT);
    tome.wait_for_bg_text(4, MATCH, "foo", WAIT);
    tome.wait_for_bg_text(5, MATCH, "foo", WAIT);
    assert_eq!(tome.bg_text(3, MATCH), "");
    assert_eq!(tome.text_col(3, "foo"), Some(TEXT_X + 4));
    assert_eq!(tome.fg_at(TEXT_X + 4, 3), BG);
    // The other matches keep the text's own colour.
    let plain = tome.fg_at(TEXT_X, 4);
    assert_eq!(tome.fg_at(TEXT_X + 4, 4), plain);
    // Matches aren't drawn as a selection.
    for row in 3..=5 {
        assert_eq!(tome.reversed_text(row), "");
    }
    wait_for_position(&tome, 1, 5);
    // The terminal cursor is in the bar, after the text.
    tome.wait_for_cursor(10, BAR_ROW, WAIT);

    // Matches update while typing: only line 2 has `foo` before a space.
    tome.type_text(" ");
    wait_for_count(&tome, "1/1");
    tome.wait_for_bg_text(4, WARN, "foo ", WAIT);
    tome.wait_for_bg_text(3, MATCH, "", WAIT);
    tome.wait_for_bg_text(3, WARN, "", WAIT);
    tome.wait_for_bg_text(5, MATCH, "", WAIT);
    wait_for_position(&tome, 2, 5);
    tome.send_keys("backspace");
    wait_for_count(&tome, "1/3");
}

#[test]
fn ctrl_f_starts_from_the_selection() {
    let mut tome = open(FIND);
    tome.wait_for_text("3  foo", WAIT);
    tome.send_keys("down");
    for _ in 0..3 {
        tome.send_keys("shift+right");
    }
    tome.wait_for_reversed(4, "two", WAIT);
    tome.send_keys("ctrl+f");
    wait_for_count(&tome, "1/1");
    assert!(row(&tome, BAR_ROW).starts_with(" Find  two"));
    tome.wait_for_bg_text(4, WARN, "two", WAIT);
    wait_for_position(&tome, 2, 1);
}

#[test]
fn enter_and_shift_enter_step_through_matches_wrapping() {
    let mut tome = open(FIND);
    find(&mut tome, "foo");
    wait_for_count(&tome, "1/3");
    wait_for_position(&tome, 1, 5);

    tome.send_keys("enter");
    wait_for_count(&tome, "2/3");
    wait_for_position(&tome, 2, 5);
    tome.send_keys("enter");
    wait_for_count(&tome, "3/3");
    wait_for_position(&tome, 3, 1);
    tome.send_keys("enter");
    wait_for_count(&tome, "1/3");
    wait_for_position(&tome, 1, 5);

    tome.send_keys("shift+enter");
    wait_for_count(&tome, "3/3");
    wait_for_position(&tome, 3, 1);
    tome.send_keys("shift+enter");
    wait_for_count(&tome, "2/3");
    wait_for_position(&tome, 2, 5);
    // Enter in the bar never edits the buffer: still three lines plus the empty
    // one after the final line break.
    tome.wait_for_text("3  foo", WAIT);
    // No fifth line number in the gutter.
    assert!(!tome.screen().join("\n").contains("   5"));
}

/// Waits for the chips that are on to read `chips`: `acc_ink` on `accent`, bold.
fn wait_for_chips(tome: &Tome, chips: &str) {
    tome.wait_for_bg_text(BAR_ROW, ACCENT, chips, WAIT);
    assert_eq!(tome.fg_text(BAR_ROW, ACC_INK), chips);
    assert_eq!(tome.bold_text(BAR_ROW), chips);
}

#[test]
fn alt_c_toggles_case_and_alt_r_toggles_regex_and_a_bad_regex_says_so() {
    let mut tome = open(FIND_CASE);
    tome.wait_for_text("2  foooo", WAIT);
    find(&mut tome, "foo");
    wait_for_count(&tome, "1/4");
    tome.wait_for_bg_text(3, WARN, "foo", WAIT);
    tome.wait_for_bg_text(3, MATCH, "FooFOO", WAIT);
    wait_for_chips(&tome, "");

    tome.send_keys("alt+c");
    wait_for_count(&tome, "1/2");
    wait_for_chips(&tome, " Aa ");
    tome.wait_for_bg_text(3, MATCH, "", WAIT);
    tome.wait_for_bg_text(3, WARN, "foo", WAIT);
    tome.wait_for_bg_text(4, MATCH, "foo", WAIT);
    tome.send_keys("alt+c");
    wait_for_count(&tome, "1/4");
    wait_for_chips(&tome, "");

    // `fo+` is literal text until regex is on.
    for _ in 0..3 {
        tome.send_keys("backspace");
    }
    tome.type_text("fo+");
    wait_for_count(&tome, "0/0");
    tome.wait_for_bg_text(3, WARN, "", WAIT);
    tome.wait_for_bg_text(3, MATCH, "", WAIT);
    tome.send_keys("alt+r");
    wait_for_count(&tome, "1/4");
    wait_for_chips(&tome, " .* ");
    tome.wait_for_bg_text(4, MATCH, "foooo", WAIT);

    // An unclosed group is an error, with nothing highlighted.
    tome.type_text("(");
    wait_for_count(&tome, "invalid regex");
    // Both the pattern and the reason are in `err`.
    tome.wait_for_fg(BAR_ROW, ERR, "fo+(invalid regex", WAIT);
    for row in 3..=4 {
        tome.wait_for_bg_text(row, WARN, "", WAIT);
        tome.wait_for_bg_text(row, MATCH, "", WAIT);
    }
    tome.send_keys("backspace");
    wait_for_count(&tome, "1/4");
}

#[test]
fn esc_closes_the_bar_and_keeps_the_cursor_on_the_current_match() {
    let mut tome = open(FIND);
    find(&mut tome, "foo");
    wait_for_count(&tome, "1/3");
    tome.send_keys("enter");
    wait_for_count(&tome, "2/3");

    tome.send_keys("esc");
    tome.wait_for_text_gone(" Find  ", WAIT);
    for row in 3..=4 {
        tome.wait_for_bg_text(row, WARN, "", WAIT);
        tome.wait_for_bg_text(row, MATCH, "", WAIT);
    }
    wait_for_position(&tome, 2, 5);
    tome.wait_for_cursor(TEXT_X + 4, 4, WAIT);
    // Typing goes into the buffer again, at the match.
    tome.type_text("x");
    tome.wait_for_text("2  two xfoo three", WAIT);
}

#[test]
fn mono_underlines_matches_and_reverses_the_current_one() {
    // `mono` can't blend `find_match_bg`, so matches are told apart by weight.
    let mut tome = Tome::spawn_with_config("theme = \"mono\"\n", &[FIND]);
    tome.wait_for_text("Ln 1, Col 1", START);
    tome.wait_for_text("3  foo", WAIT);
    find(&mut tome, "foo");
    wait_for_count(&tome, "1/3");
    tome.wait_for_reversed(3, "foo", WAIT);
    tome.wait_for_underlined(4, "foo", WAIT);
    tome.wait_for_underlined(5, "foo", WAIT);
    assert_eq!(tome.underlined_text(3), "");
    assert_eq!(tome.reversed_text(4), "");

    tome.send_keys("enter");
    wait_for_count(&tome, "2/3");
    tome.wait_for_reversed(4, "foo", WAIT);
    tome.wait_for_underlined(3, "foo", WAIT);
    tome.wait_for_reversed(3, "", WAIT);
}
