mod harness;

use std::path::Path;
use std::time::Duration;

use harness::{Glyph, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// `.gitignore` hides `*.log` and `build/`.
const PROJECT: &str = "tests/fixtures/project";
const LONG: &str = "tests/fixtures/long.txt";
/// The default theme's accent, hydra's lime `#c3f53c`.
const ACCENT: vt100::Color = vt100::Color::Rgb(0xc3, 0xf5, 0x3c);

/// At 100x30 the card is 60 wide and 20 tall, centred: its border starts at
/// column 20 and row 5, the query is on row 6 and the list starts on row 7, one
/// space in from the border.
const QUERY_ROW: u16 = 6;
const FIRST_ROW: u16 = 7;
const QUERY_X: u16 = 21;
const LIST_X: u16 = 22;
/// The list's width inside the border.
const LIST_WIDTH: usize = 58;

/// Glyph in the fixture project with no file open, so the root is its folder.
fn open_project() -> Glyph {
    let glyph = Glyph::spawn_in(Path::new(PROJECT), &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph
}

fn open_picker(glyph: &mut Glyph) {
    glyph.send_keys("ctrl+p");
    glyph.wait_for_text("Go to file:", WAIT);
}

fn row(glyph: &Glyph, row: u16) -> String {
    glyph.screen()[usize::from(row)].clone()
}

fn status_line(glyph: &Glyph) -> String {
    row(glyph, ROWS - 1)
}

/// What a selected list row reads reversed: the path one space in, padded to the
/// list's width.
fn selected(path: &str) -> String {
    format!(" {path:<width$}", width = LIST_WIDTH - 1)
}

#[test]
fn ctrl_p_lists_project_files_relative_to_the_root_respecting_gitignore() {
    let mut glyph = open_project();
    open_picker(&mut glyph);
    glyph.wait_for_text("src/util/helpers.rs", WAIT);
    assert_eq!(glyph.text_col(QUERY_ROW, "Go to file:"), Some(QUERY_X));
    // Path order with an empty query; ignored files and `.git` never appear.
    let expected = [
        ".gitignore",
        "README.md",
        "docs/guide.md",
        "notes.txt",
        "src/main.rs",
        "src/util/helpers.rs",
    ];
    for (i, path) in expected.iter().enumerate() {
        let y = FIRST_ROW + u16::try_from(i).unwrap();
        assert_eq!(glyph.text_col(y, path), Some(LIST_X), "{path}");
    }
    let screen = glyph.screen().join("\n");
    assert!(!screen.contains("debug.log"), "{screen}");
    assert!(!screen.contains("out.txt"), "{screen}");
    assert!(!screen.contains("HEAD"), "{screen}");
    // The first file starts selected.
    glyph.wait_for_reversed(FIRST_ROW, &selected(".gitignore"), WAIT);
}

#[test]
fn typing_main_puts_src_main_rs_first_with_the_matched_letters_in_the_accent() {
    let mut glyph = open_project();
    open_picker(&mut glyph);
    glyph.wait_for_text("README.md", WAIT);
    glyph.type_text("main");
    glyph.wait_for_text("Go to file: main", WAIT);
    glyph.wait_for_text_gone("README.md", WAIT);
    assert_eq!(glyph.text_col(FIRST_ROW, "src/main.rs"), Some(LIST_X));
    glyph.wait_for_fg(FIRST_ROW, ACCENT, "main", WAIT);
    // The cursor sits after the query.
    glyph.wait_for_cursor(QUERY_X + "Go to file: main".len() as u16, QUERY_ROW, WAIT);
}

#[test]
fn arrows_move_the_selection_and_enter_opens_it_in_a_tab() {
    let mut glyph = open_project();
    open_picker(&mut glyph);
    glyph.wait_for_reversed(FIRST_ROW, &selected(".gitignore"), WAIT);
    glyph.send_keys("down");
    glyph.send_keys("down");
    glyph.wait_for_reversed(FIRST_ROW + 2, &selected("docs/guide.md"), WAIT);
    glyph.send_keys("up");
    glyph.wait_for_reversed(FIRST_ROW + 1, &selected("README.md"), WAIT);
    assert_eq!(glyph.reversed_text(FIRST_ROW + 2), "");
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("Go to file:", WAIT);
    glyph.wait_for_text("1 │ # Project fixture", WAIT);
    // The status line is drawn after the editor, so it may lag a moment behind.
    glyph.wait_for_screen("README.md in the status line", WAIT, |screen| {
        screen[usize::from(ROWS) - 1].contains("README.md")
    });

    // A filtered pick opens beside it in a second tab.
    open_picker(&mut glyph);
    glyph.type_text("helpers");
    glyph.wait_for_reversed(FIRST_ROW, &selected("src/util/helpers.rs"), WAIT);
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
    glyph.wait_for_text("Go to file: notes", WAIT);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("Go to file:", WAIT);
    glyph.wait_for_text_gone("notes.txt", WAIT);
    assert!(status_line(&glyph).contains("untitled"));
    // Typing goes to the editor again, not to a hidden query.
    glyph.type_text("x");
    glyph.wait_for_text("1 │ x", WAIT);
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
