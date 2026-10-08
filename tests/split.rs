mod harness;

use std::fs;
use std::time::Duration;

use harness::{Glyph, ROWS, pill_text};
use tempfile::TempDir;
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// With the tree hidden the two splits share 100 columns: 0..50, the `│` in
/// column 50, then 51..100.
const DIVIDER: usize = 50;
/// With the tree showing they share the 71 columns right of it and the blank
/// column after it: 29..64, the `│` in column 64, then 65..100.
const TREE_DIVIDER: usize = 64;
const LEFT_X: u16 = 29;
const LEFT: usize = LEFT_X as usize;
/// The tree rows of `a.txt` and `b.txt`, below its brand row.
const A_ROW: u16 = 3;
const B_ROW: u16 = 4;
/// Each split's header: a blank row, the pills, the aurora thread; then its
/// editor.
const PILL_ROW: u16 = 1;
const THREAD_ROW: u16 = 2;
const TOP: usize = 3;

// The `aurora` roles the splits use (README §4.1).
const BG: Color = Color::Rgb(0x0b, 0x0a, 0x10);
const RAISED: Color = Color::Rgb(0x16, 0x14, 0x1f);
const RAISED2: Color = Color::Rgb(0x1e, 0x1b, 0x2a);
const GUIDE: Color = Color::Rgb(0x1f, 0x1c, 0x2b);
const MUTED: Color = Color::Rgb(0x67, 0x62, 0x7d);
const STRONG: Color = Color::Rgb(0xf4, 0xf2, 0xfb);
const ACCENT: Color = Color::Rgb(0xb6, 0x9c, 0xff);
const ACCENT2: Color = Color::Rgb(0x6e, 0xe7, 0xd8);

/// `a` moved `t` of the way to `b`, per channel, rounded, as the editor mixes.
fn mix(a: Color, b: Color, t: f64) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (a, b) else {
        panic!("can't mix {a:?} and {b:?}");
    };
    let m = |p: u8, q: u8| (f64::from(p) + (f64::from(q) - f64::from(p)) * t).round() as u8;
    Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
}

/// The thread's colour in column `x` of a split spanning `from..to`, its active
/// pill's middle at `pc` (README §2.3, measured from the column before the
/// split). `reach` is .92 in the focused split and .97 in the other (§5.6).
fn thread(x: u16, from: u16, to: u16, pc: f64, reach: f64) -> Color {
    let hue = mix(
        ACCENT,
        ACCENT2,
        f64::from(x + 1 - from) / f64::from(to + 1 - from),
    );
    let d = (f64::from(x) - pc).abs();
    mix(hue, BG, (d / 46.0).clamp(0.0, 1.0) * reach + 0.04)
}

/// A temp folder holding `a.txt` ("alpha") and `b.txt` ("bravo").
fn project() -> TempDir {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join("a.txt"), "alpha\n").expect("write a.txt");
    fs::write(dir.path().join("b.txt"), "bravo\n").expect("write b.txt");
    dir
}

/// `a.txt` opened on its own, so the tree is hidden.
fn open_file(dir: &TempDir) -> Glyph {
    let glyph = Glyph::spawn_in(dir.path(), &["a.txt"]);
    glyph.wait_for_text("1  alpha", START);
    glyph
}

/// The folder opened with the tree showing and a.txt open in the left split.
fn open_folder(dir: &TempDir) -> Glyph {
    let mut glyph = Glyph::spawn_in(dir.path(), &["."]);
    glyph.wait_for_text("b.txt", START);
    glyph.click(3, A_ROW);
    glyph.wait_for_text("1  alpha", WAIT);
    glyph
}

/// The columns `from..to` of `line`, trailing spaces cut.
fn cols(line: &str, from: usize, to: usize) -> String {
    line.chars()
        .skip(from)
        .take(to.saturating_sub(from))
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// What the left and right splits show on `row`, either side of `divider`; the
/// left split starts at `left`.
fn halves(screen: &[String], row: usize, left: usize, divider: usize) -> (String, String) {
    (
        cols(&screen[row], left, divider),
        cols(&screen[row], divider + 1, usize::MAX),
    )
}

/// Whether `divider` holds a `│` on every row above the status line.
fn divided(screen: &[String], divider: usize) -> bool {
    screen[..usize::from(ROWS - 1)]
        .iter()
        .all(|line| line.chars().nth(divider) == Some('│'))
}

/// Waits for the two pill rows to read `left` and `right`, each a split's tab
/// names with its active one (see `pill_text`).
fn wait_for_bars(
    glyph: &Glyph,
    left_x: usize,
    divider: usize,
    left: (&[&str], usize),
    right: (&[&str], usize),
) {
    // Each split's first pill starts one column in.
    let left = format!(" {}", pill_text(left.0, left.1));
    let right = format!(" {}", pill_text(right.0, right.1));
    glyph.wait_for_screen(&format!("pills {left:?} | {right:?}"), WAIT, |screen| {
        halves(screen, usize::from(PILL_ROW), left_x, divider) == (left.clone(), right.clone())
    });
}

/// Waits for the focused split's active tab, the only bold name on the pill
/// row, to be `name`. With the tree showing, its `glyph` brand is bold too.
fn wait_for_focused(glyph: &Glyph, tree: bool, name: &str) {
    let brand = if tree { "glyph" } else { "" };
    glyph.wait_for_bold(PILL_ROW, &format!("{brand}{name}"), WAIT);
}

#[test]
fn alt_v_shows_two_editors_side_by_side_and_alt_v_again_closes_the_right_one() {
    let dir = project();
    let mut glyph = open_file(&dir);
    glyph.send_keys("alt+v");

    // The visual check: two editors of about equal width split by a `│`, each with
    // its own pills, and only the focused (new, right) split's name lit.
    glyph.wait_for_screen("two splits on a.txt", WAIT, |screen| {
        divided(screen, DIVIDER)
            && halves(screen, usize::from(PILL_ROW), 0, DIVIDER)
                == (" ▐ a.txt ▌".into(), " ▐ a.txt ▌".into())
            && halves(screen, TOP, 0, DIVIDER) == ("   1  alpha".into(), "   1  alpha".into())
    });
    wait_for_focused(&glyph, false, "a.txt");
    assert_eq!(glyph.text_col(PILL_ROW, "a.txt"), Some(3));
    let cursor = glyph.cursor();
    assert!(
        usize::from(cursor.0) > DIVIDER,
        "cursor {cursor:?} should be in the right split\n{:#?}",
        glyph.screen()
    );

    // Each split has its own tabs and active tab: a new file opens on the right.
    glyph.send_keys("ctrl+n");
    wait_for_bars(
        &glyph,
        0,
        DIVIDER,
        (&["a.txt"], 0),
        (&["a.txt", "untitled"], 1),
    );
    wait_for_focused(&glyph, false, "untitled");
    glyph.wait_for_screen("right split empty", WAIT, |screen| {
        halves(screen, TOP, 0, DIVIDER) == ("   1  alpha".into(), "   1".into())
    });

    // Alt+V again: one editor, keeping the tab only the right split had.
    glyph.send_keys("alt+v");
    glyph.wait_for_screen("one editor", WAIT, |screen| {
        cols(&screen[usize::from(PILL_ROW)], 0, 100)
            == format!(" {}", pill_text(&["a.txt", "untitled"], 0))
            && screen[TOP].matches("alpha").count() == 1
            && !divided(screen, DIVIDER)
    });
    wait_for_focused(&glyph, false, "a.txt");
    glyph.send_keys("ctrl+q");
    assert!(glyph.wait_exit(WAIT).success());
}

#[test]
fn aurora_splits_have_a_guide_rule_and_the_unfocused_one_is_dimmed() {
    let dir = project();
    let mut glyph = Glyph::spawn_in_with_config(dir.path(), "theme = \"aurora\"\n", &["a.txt"]);
    glyph.wait_for_text("1  alpha", START);
    glyph.send_keys("alt+v");
    wait_for_focused(&glyph, false, "a.txt");
    glyph.wait_for_screen("two splits", WAIT, |screen| divided(screen, DIVIDER));

    // One `│` in `guide` between the editors, the only vertical rule.
    let divider = u16::try_from(DIVIDER).expect("fits");
    for row in [0, PILL_ROW, THREAD_ROW, 3, ROWS - 2] {
        assert_eq!(glyph.fg_at(divider, row), GUIDE, "row {row}");
    }

    // The left split lost focus: its pill is `muted` on `raised`, its thread
    // fainter (.97). The right one has focus: `raised2`, the name lit, .92.
    assert_eq!(glyph.fg_at(1, PILL_ROW), RAISED);
    assert_eq!(glyph.bg_at(2, PILL_ROW), RAISED);
    assert_eq!(glyph.fg_at(3, PILL_ROW), MUTED);
    assert_eq!(glyph.fg_at(52, PILL_ROW), RAISED2);
    assert_eq!(glyph.bg_at(53, PILL_ROW), RAISED2);
    assert_eq!(glyph.fg_at(54, PILL_ROW), STRONG);
    // ` a.txt ` is 7 wide: each pill's middle is 1 + 3.5 past its left cap.
    let (left_pc, right_pc) = (1.0 + 4.5, 52.0 + 4.5);
    for x in [5, 6, 30, 49] {
        assert_eq!(
            glyph.fg_at(x, THREAD_ROW),
            thread(x, 0, divider, left_pc, 0.97),
            "left column {x}"
        );
    }
    for x in [51, 56, 57, 99] {
        assert_eq!(
            glyph.fg_at(x, THREAD_ROW),
            thread(x, divider + 1, 100, right_pc, 0.92),
            "right column {x}"
        );
    }

    // F6 swaps which split is lit.
    glyph.send_keys("f6");
    glyph.wait_for_fg_at(3, PILL_ROW, STRONG, WAIT);
    glyph.wait_for_fg_at(54, PILL_ROW, MUTED, WAIT);
    glyph.wait_for_fg_at(
        52,
        THREAD_ROW,
        thread(52, divider + 1, 100, right_pc, 0.97),
        WAIT,
    );
    glyph.wait_for_fg_at(5, THREAD_ROW, thread(5, 0, divider, left_pc, 0.92), WAIT);
}

#[test]
fn f6_cycles_tree_left_right_and_skips_the_hidden_tree() {
    let dir = project();
    let mut glyph = open_folder(&dir);
    glyph.send_keys("alt+v");
    // b.txt opens in the focused right split, which then shows it.
    glyph.click(3, B_ROW);
    wait_for_bars(
        &glyph,
        LEFT,
        TREE_DIVIDER,
        (&["a.txt"], 0),
        (&["a.txt", "b.txt"], 1),
    );
    glyph.wait_for_text("1  bravo", WAIT);

    // From the right split F6 wraps to the tree, where typing edits nothing...
    glyph.send_keys("f6");
    glyph.wait_for_cursor(2, B_ROW, WAIT);
    glyph.type_text("0");
    // ...then the left split, then the right one.
    glyph.send_keys("f6");
    glyph.type_text("1");
    glyph.wait_for_text("1  1alpha", WAIT);
    glyph.send_keys("f6");
    glyph.type_text("2");
    glyph.wait_for_text("1  2bravo", WAIT);
    glyph.send_keys("f6");
    glyph.wait_for_cursor(2, B_ROW, WAIT);

    // With the tree hidden F6 goes left, right, left.
    glyph.send_keys("ctrl+b");
    glyph.send_keys("f6");
    glyph.type_text("3");
    glyph.wait_for_screen("typed on the left", WAIT, |screen| {
        halves(screen, TOP, 0, DIVIDER) == ("   1  13alpha".into(), "   1  2bravo".into())
    });
    glyph.send_keys("f6");
    glyph.type_text("4");
    glyph.wait_for_text("1  24bravo", WAIT);
    glyph.send_keys("f6");
    glyph.type_text("5");
    glyph.wait_for_text("1  135alpha", WAIT);
}

#[test]
fn a_click_focuses_its_split_and_tab_and_open_actions_go_there() {
    let dir = project();
    let mut glyph = open_folder(&dir);
    glyph.send_keys("alt+v");
    wait_for_bars(&glyph, LEFT, TREE_DIVIDER, (&["a.txt"], 0), (&["a.txt"], 0));

    // A click in the left editor focuses it: a new tab opens there.
    glyph.click(LEFT_X + 10, 5);
    glyph.send_keys("ctrl+n");
    wait_for_bars(
        &glyph,
        LEFT,
        TREE_DIVIDER,
        (&["a.txt", "untitled"], 1),
        (&["a.txt"], 0),
    );
    wait_for_focused(&glyph, true, "untitled");

    // A click in the right editor focuses that one: the tree opens b.txt there.
    glyph.click(80, 5);
    wait_for_focused(&glyph, true, "a.txt");
    glyph.click(3, B_ROW);
    wait_for_bars(
        &glyph,
        LEFT,
        TREE_DIVIDER,
        (&["a.txt", "untitled"], 1),
        (&["a.txt", "b.txt"], 1),
    );
    wait_for_focused(&glyph, true, "b.txt");

    // Tab keys move along the focused split's tabs only.
    glyph.send_keys("alt+,");
    wait_for_focused(&glyph, true, "a.txt");
    glyph.wait_for_screen("right split back on a.txt", WAIT, |screen| {
        halves(screen, TOP, LEFT, TREE_DIVIDER) == ("   1".into(), "   1  alpha".into())
    });
    // Ctrl+W closes the right split's tab; the left split keeps its own a.txt tab.
    glyph.send_keys("ctrl+w");
    wait_for_bars(
        &glyph,
        LEFT,
        TREE_DIVIDER,
        (&["a.txt", "untitled"], 1),
        (&["b.txt"], 0),
    );

    // A click on the left split's pills focuses the left split too.
    let a = glyph.text_col(PILL_ROW, "a.txt").expect("left a.txt pill");
    glyph.click(a, PILL_ROW);
    wait_for_focused(&glyph, true, "a.txt");
    // Typing lands at the left split's own cursor, where the click below the text
    // left it on the last line.
    glyph.type_text("z");
    glyph.wait_for_text("2  z", WAIT);
    wait_for_bars(
        &glyph,
        LEFT,
        TREE_DIVIDER,
        (&["a.txt •", "untitled"], 0),
        (&["b.txt"], 0),
    );
}

#[test]
fn edits_to_a_buffer_open_in_both_splits_show_in_both() {
    let dir = project();
    let mut glyph = open_file(&dir);
    glyph.send_keys("alt+v");
    wait_for_focused(&glyph, false, "a.txt");

    // Typed on the right, shown on both sides, both tabs marked dirty.
    glyph.send_keys("end");
    glyph.type_text("!");
    glyph.wait_for_screen("edit in both", WAIT, |screen| {
        halves(screen, TOP, 0, DIVIDER) == ("   1  alpha!".into(), "   1  alpha!".into())
            && halves(screen, usize::from(PILL_ROW), 0, DIVIDER)
                == (" ▐ a.txt • ▌".into(), " ▐ a.txt • ▌".into())
    });

    // Typed on the left, at the left split's own cursor (still at the start).
    glyph.click(10, 3);
    glyph.send_keys("home");
    glyph.type_text("<");
    glyph.wait_for_screen("second edit in both", WAIT, |screen| {
        halves(screen, TOP, 0, DIVIDER) == ("   1  <alpha!".into(), "   1  <alpha!".into())
    });

    // Saving from either split saves the one buffer.
    glyph.send_keys("ctrl+s");
    wait_for_bars(&glyph, 0, DIVIDER, (&["a.txt"], 0), (&["a.txt"], 0));
    assert_eq!(
        fs::read_to_string(dir.path().join("a.txt")).expect("read a.txt"),
        "<alpha!\n"
    );
    glyph.send_keys("ctrl+q");
    assert!(glyph.wait_exit(WAIT).success());
}
