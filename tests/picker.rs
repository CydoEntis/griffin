mod harness;

use std::path::Path;
use std::time::Duration;

use harness::{Glyph, ROWS};
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// `.gitignore` hides `*.log` and `build/`.
const PROJECT: &str = "tests/fixtures/project";
const LONG: &str = "tests/fixtures/long.txt";
/// The default theme, hydra: its `accent`, `accent2` and `raised`.
const ACCENT: Color = Color::Rgb(0xc3, 0xf5, 0x3c);
const ACCENT2: Color = Color::Rgb(0x5a, 0xa9, 0xff);
const RAISED: Color = Color::Rgb(0x0f, 0x18, 0x21);

/// At 100x30 the cast card is 86 wide from column 7, dropping from row 4 with
/// its lit edge (design README §3). `✦` and the query are on row 6, the rule on
/// row 7, `FILES` on row 8 and the file rows from row 9, their text four cells
/// in.
const CARD_X: u16 = 7;
const CARD_Y: u16 = 4;
const QUERY_ROW: u16 = 6;
const RULE_ROW: u16 = 7;
const HEADER_ROW: u16 = 8;
const FIRST_ROW: u16 = 9;
const TEXT_X: u16 = 11;
const QUERY_X: u16 = 13;
/// Where the card's right-aligned text ends: four cells in from its edge.
const RIGHT: u16 = 89;
const SCOPE: &str = "cast · files · commands";
/// With five file rows, a blank, then the footer.
const FOOTER_ROW: u16 = 15;
const PREFIXES: &str = "> commands   : line   / text";
const KEYS: &str = "↑↓  ⏎  esc";

/// `a` moved `t` of the way to `b`, per channel, rounded, as the editor mixes.
fn mix(a: Color, b: Color, t: f64) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (a, b) else {
        panic!("can't mix {a:?} and {b:?}");
    };
    let m = |p: u8, q: u8| (f64::from(p) + (f64::from(q) - f64::from(p)) * t).round() as u8;
    Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
}

/// The left end of the selected row's glow: `raised` lit 30 % towards the accent.
fn glow() -> Color {
    mix(RAISED, ACCENT, 0.3)
}

/// Glyph in the fixture project with no file open, so the root is its folder.
fn open_project() -> Glyph {
    let glyph = Glyph::spawn_in(Path::new(PROJECT), &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph
}

fn open_picker(glyph: &mut Glyph) {
    glyph.send_keys("ctrl+p");
    glyph.wait_for_text(SCOPE, WAIT);
}

fn row(glyph: &Glyph, row: u16) -> String {
    glyph.screen()[usize::from(row)].clone()
}

fn status_line(glyph: &Glyph) -> String {
    row(glyph, ROWS - 1)
}

/// Waits for `file` (its name, two blanks, its folder) to be the selected row
/// `y`: the glow row with `⏎` at its right.
fn wait_for_selected(glyph: &Glyph, y: u16, file: &str) {
    // The text first: the glow may already be on row `y` for the row before.
    glyph.wait_for_screen(&format!("{file:?} on row {y}"), WAIT, |lines| {
        let shown: String = lines[usize::from(y)]
            .chars()
            .skip(usize::from(TEXT_X))
            .collect();
        shown.starts_with(&format!("{file} "))
    });
    glyph.wait_for_bg(CARD_X, y, glow(), WAIT);
    assert_eq!(glyph.text_col(y, "⏎"), Some(RIGHT - 1));
}

#[test]
fn ctrl_p_casts_a_card_of_project_files_respecting_gitignore() {
    let mut glyph = open_project();
    open_picker(&mut glyph);
    glyph.wait_for_text("main.rs  src", WAIT);

    // The lit edge runs accent → accent2 → accent across the card.
    assert_eq!(glyph.text_col(CARD_Y, "▀"), Some(CARD_X));
    assert_eq!(glyph.fg_at(CARD_X, CARD_Y), ACCENT);
    assert_eq!(glyph.fg_at(CARD_X + 85, CARD_Y), ACCENT);
    assert_eq!(
        row(&glyph, CARD_Y).matches('▀').count(),
        86,
        "the edge spans the card"
    );
    assert_eq!(glyph.bg_at(CARD_X + 1, CARD_Y + 1), RAISED);
    // `✦`, then the palette's name right-aligned.
    assert_eq!(glyph.text_col(QUERY_ROW, "✦"), Some(TEXT_X));
    assert_eq!(glyph.fg_at(TEXT_X, QUERY_ROW), ACCENT2);
    assert_eq!(
        glyph.text_col(QUERY_ROW, SCOPE),
        Some(RIGHT - SCOPE.chars().count() as u16)
    );
    assert_eq!(glyph.fg_text(QUERY_ROW, ACCENT).trim_end(), "cast");
    assert_eq!(glyph.text_col(RULE_ROW, "─"), Some(CARD_X + 2));
    assert_eq!(
        row(&glyph, RULE_ROW).matches('─').count(),
        usize::from(86 - 4_u16)
    );
    assert_eq!(glyph.text_col(HEADER_ROW, "FILES"), Some(TEXT_X));
    assert_eq!(glyph.bold_text(HEADER_ROW).trim_end(), "FILES");

    // Path order with an empty query, five at a time: the name, then the folder.
    let expected = [
        ".gitignore",
        "README.md",
        "guide.md  docs",
        "notes.txt",
        "main.rs  src",
    ];
    for (i, file) in expected.iter().enumerate() {
        let y = FIRST_ROW + u16::try_from(i).unwrap();
        assert_eq!(glyph.text_col(y, file), Some(TEXT_X), "{file}");
    }
    let screen = glyph.screen().join("\n");
    assert!(!screen.contains("helpers.rs"), "only five rows: {screen}");
    assert!(!screen.contains("debug.log"), "{screen}");
    assert!(!screen.contains("out.txt"), "{screen}");
    assert!(!screen.contains("HEAD"), "{screen}");
    // The first file starts selected.
    wait_for_selected(&glyph, FIRST_ROW, ".gitignore");

    assert_eq!(glyph.text_col(FOOTER_ROW, PREFIXES), Some(TEXT_X));
    assert_eq!(glyph.bold_text(FOOTER_ROW).replace(' ', ""), ">:/");
    assert_eq!(
        glyph.text_col(FOOTER_ROW, KEYS),
        Some(RIGHT - KEYS.chars().count() as u16)
    );
}

#[test]
fn typing_main_puts_main_rs_first_with_the_matched_letters_lit() {
    let mut glyph = open_project();
    open_picker(&mut glyph);
    glyph.wait_for_text("README.md", WAIT);
    glyph.type_text("main");
    glyph.wait_for_text("✦ main", WAIT);
    glyph.wait_for_text_gone("README.md", WAIT);
    wait_for_selected(&glyph, FIRST_ROW, "main.rs  src");
    // Accent and bold on the matched letters, accent on `⏎`.
    // Blanks are dropped: the terminal may leave a blank in the last colour.
    assert_eq!(glyph.fg_text(FIRST_ROW, ACCENT).replace(' ', ""), "main⏎");
    assert_eq!(glyph.bold_text(FIRST_ROW).replace(' ', ""), "main");
    // The cursor sits after the query.
    glyph.wait_for_cursor(QUERY_X + "main".len() as u16, QUERY_ROW, WAIT);

    // Nothing matching says so.
    glyph.type_text("zzz");
    glyph.wait_for_text("no matching files", WAIT);
    assert_eq!(glyph.text_col(FIRST_ROW, "no matching files"), Some(TEXT_X));
}

#[test]
fn arrows_move_the_selection_and_enter_opens_it_in_a_tab() {
    let mut glyph = open_project();
    open_picker(&mut glyph);
    wait_for_selected(&glyph, FIRST_ROW, ".gitignore");
    glyph.send_keys("down");
    glyph.send_keys("down");
    wait_for_selected(&glyph, FIRST_ROW + 2, "guide.md  docs");
    glyph.send_keys("up");
    wait_for_selected(&glyph, FIRST_ROW + 1, "README.md");
    assert_eq!(glyph.bg_at(CARD_X, FIRST_ROW + 2), RAISED);
    glyph.send_keys("enter");
    glyph.wait_for_text_gone(SCOPE, WAIT);
    glyph.wait_for_text("1 │ # Project fixture", WAIT);
    // The status line is drawn after the editor, so it may lag a moment behind.
    glyph.wait_for_screen("README.md in the status line", WAIT, |screen| {
        screen[usize::from(ROWS) - 1].contains("README.md")
    });

    // Past the fifth row the list scrolls to keep the selection on the last.
    open_picker(&mut glyph);
    wait_for_selected(&glyph, FIRST_ROW, ".gitignore");
    for _ in 0..5 {
        glyph.send_keys("down");
    }
    wait_for_selected(&glyph, FIRST_ROW + 4, "helpers.rs  src/util");
    assert_eq!(glyph.text_col(FIRST_ROW, "README.md"), Some(TEXT_X));
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(SCOPE, WAIT);

    // A filtered pick opens beside it in a second tab.
    open_picker(&mut glyph);
    glyph.type_text("helpers");
    wait_for_selected(&glyph, FIRST_ROW, "helpers.rs  src/util");
    glyph.send_keys("enter");
    glyph.wait_for_text("1 │ pub fn help() {}", WAIT);
    glyph.wait_for_screen("both files in the tab bar", WAIT, |screen| {
        screen[0].contains("README.md") && screen[0].contains("helpers.rs")
    });
}

#[test]
fn esc_closes_the_picker_without_opening_anything() {
    let mut glyph = open_project();
    open_picker(&mut glyph);
    glyph.wait_for_text("notes.txt", WAIT);
    glyph.type_text("notes");
    glyph.wait_for_text("✦ notes", WAIT);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(SCOPE, WAIT);
    glyph.wait_for_text_gone("notes.txt", WAIT);
    assert!(status_line(&glyph).contains("untitled"));
    // Typing goes to the editor again, not to a hidden query.
    glyph.type_text("x");
    glyph.wait_for_text("1 │ x", WAIT);
}

#[test]
fn a_click_outside_the_card_closes_it_and_one_inside_does_not() {
    let mut glyph = open_project();
    open_picker(&mut glyph);
    glyph.wait_for_text("notes.txt", WAIT);
    glyph.click(50, FIRST_ROW + 2);
    // Still open: typing goes to the query.
    glyph.type_text("x");
    glyph.wait_for_text("✦ x", WAIT);
    glyph.click(2, 20);
    glyph.wait_for_text_gone(SCOPE, WAIT);
    assert!(status_line(&glyph).contains("untitled"));
}

#[test]
fn mono_dims_with_the_modifier_and_reverses_the_selected_row() {
    let mut glyph = Glyph::spawn_in_with_config(Path::new(PROJECT), "theme = \"mono\"\n", &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    open_picker(&mut glyph);
    glyph.wait_for_text("main.rs  src", WAIT);
    let selected = glyph.reversed_text(FIRST_ROW);
    assert_eq!(selected.chars().count(), 86, "{selected:?}");
    assert!(selected.contains(".gitignore"), "{selected:?}");
    let status = ROWS - 1;
    let position = glyph.text_col(status, "Ln 1").unwrap();
    assert!(glyph.dim_at(position, status), "the status line is dimmed");
    assert!(!glyph.dim_at(TEXT_X, HEADER_ROW), "the card isn't");
}

#[test]
fn ctrl_g_prompts_for_a_line_and_moves_the_cursor_there() {
    let mut glyph = Glyph::spawn(&[LONG]);
    glyph.wait_for_text("line 1", START);
    glyph.send_keys("ctrl+g");
    glyph.wait_for_text("Go to line:", WAIT);
    glyph.type_text("120");
    glyph.wait_for_text("Go to line: 120", WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("Go to line:", WAIT);
    glyph.wait_for_text("Ln 120, Col 1", WAIT);
    glyph.wait_for_text("line 120", WAIT);

    // Something that isn't a number leaves the cursor where it was and says so.
    glyph.send_keys("ctrl+g");
    glyph.wait_for_text("Go to line:", WAIT);
    glyph.type_text("abc");
    glyph.send_keys("enter");
    glyph.wait_for_text("not a line number: abc", WAIT);
    assert!(status_line(&glyph).contains("Ln 120, Col 1"));

    // Esc closes the prompt and moves nothing.
    glyph.send_keys("ctrl+g");
    glyph.wait_for_text("Go to line:", WAIT);
    glyph.type_text("5");
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("Go to line:", WAIT);
    assert!(status_line(&glyph).contains("Ln 120, Col 1"));
}
