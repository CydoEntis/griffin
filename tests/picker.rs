mod harness;

use std::path::Path;
use std::time::Duration;

use harness::{Griffin, ROWS};

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

/// Griffin in the fixture project with no file open, so the root is its folder.
fn open_project() -> Griffin {
    let griffin = Griffin::spawn_in(Path::new(PROJECT), &[]);
    griffin.wait_for_text("Ln 1, Col 1", START);
    griffin
}

fn open_picker(griffin: &mut Griffin) {
    griffin.send_keys("ctrl+p");
    griffin.wait_for_text("Go to file:", WAIT);
}

fn row(griffin: &Griffin, row: u16) -> String {
    griffin.screen()[usize::from(row)].clone()
}

fn status_line(griffin: &Griffin) -> String {
    row(griffin, ROWS - 1)
}

/// What a selected list row reads reversed: the path one space in, padded to the
/// list's width.
fn selected(path: &str) -> String {
    format!(" {path:<width$}", width = LIST_WIDTH - 1)
}

#[test]
fn ctrl_p_lists_project_files_relative_to_the_root_respecting_gitignore() {
    let mut griffin = open_project();
    open_picker(&mut griffin);
    griffin.wait_for_text("src/util/helpers.rs", WAIT);
    assert_eq!(griffin.text_col(QUERY_ROW, "Go to file:"), Some(QUERY_X));
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
        assert_eq!(griffin.text_col(y, path), Some(LIST_X), "{path}");
    }
    let screen = griffin.screen().join("\n");
    assert!(!screen.contains("debug.log"), "{screen}");
    assert!(!screen.contains("out.txt"), "{screen}");
    assert!(!screen.contains("HEAD"), "{screen}");
    // The first file starts selected.
    griffin.wait_for_reversed(FIRST_ROW, &selected(".gitignore"), WAIT);
}

#[test]
fn typing_main_puts_src_main_rs_first_with_the_matched_letters_in_the_accent() {
    let mut griffin = open_project();
    open_picker(&mut griffin);
    griffin.wait_for_text("README.md", WAIT);
    griffin.type_text("main");
    griffin.wait_for_text("Go to file: main", WAIT);
    griffin.wait_for_text_gone("README.md", WAIT);
    assert_eq!(griffin.text_col(FIRST_ROW, "src/main.rs"), Some(LIST_X));
    griffin.wait_for_fg(FIRST_ROW, ACCENT, "main", WAIT);
    // The cursor sits after the query.
    griffin.wait_for_cursor(QUERY_X + "Go to file: main".len() as u16, QUERY_ROW, WAIT);
}

#[test]
fn arrows_move_the_selection_and_enter_opens_it_in_a_tab() {
    let mut griffin = open_project();
    open_picker(&mut griffin);
    griffin.wait_for_reversed(FIRST_ROW, &selected(".gitignore"), WAIT);
    griffin.send_keys("down");
    griffin.send_keys("down");
    griffin.wait_for_reversed(FIRST_ROW + 2, &selected("docs/guide.md"), WAIT);
    griffin.send_keys("up");
    griffin.wait_for_reversed(FIRST_ROW + 1, &selected("README.md"), WAIT);
    assert_eq!(griffin.reversed_text(FIRST_ROW + 2), "");
    griffin.send_keys("enter");
    griffin.wait_for_text_gone("Go to file:", WAIT);
    griffin.wait_for_text("1 │ # Project fixture", WAIT);
    assert!(status_line(&griffin).contains("README.md"));

    // A filtered pick opens beside it in a second tab.
    open_picker(&mut griffin);
    griffin.type_text("helpers");
    griffin.wait_for_reversed(FIRST_ROW, &selected("src/util/helpers.rs"), WAIT);
    griffin.send_keys("enter");
    griffin.wait_for_text("1 │ pub fn help() {}", WAIT);
    griffin.wait_for_screen("both files in the tab bar", WAIT, |screen| {
        screen[0].contains("README.md") && screen[0].contains("helpers.rs")
    });
}

#[test]
fn esc_closes_the_picker_without_opening_anything() {
    let mut griffin = open_project();
    open_picker(&mut griffin);
    griffin.wait_for_text("notes.txt", WAIT);
    griffin.type_text("notes");
    griffin.wait_for_text("Go to file: notes", WAIT);
    griffin.send_keys("esc");
    griffin.wait_for_text_gone("Go to file:", WAIT);
    griffin.wait_for_text_gone("notes.txt", WAIT);
    assert!(status_line(&griffin).contains("untitled"));
    // Typing goes to the editor again, not to a hidden query.
    griffin.type_text("x");
    griffin.wait_for_text("1 │ x", WAIT);
}

#[test]
fn ctrl_g_prompts_for_a_line_and_moves_the_cursor_there() {
    let mut griffin = Griffin::spawn(&[LONG]);
    griffin.wait_for_text("line 1", START);
    griffin.send_keys("ctrl+g");
    griffin.wait_for_text("Go to line:", WAIT);
    griffin.type_text("120");
    griffin.wait_for_text("Go to line: 120", WAIT);
    griffin.send_keys("enter");
    griffin.wait_for_text_gone("Go to line:", WAIT);
    griffin.wait_for_text("Ln 120, Col 1", WAIT);
    griffin.wait_for_text("line 120", WAIT);

    // Something that isn't a number leaves the cursor where it was and says so.
    griffin.send_keys("ctrl+g");
    griffin.wait_for_text("Go to line:", WAIT);
    griffin.type_text("abc");
    griffin.send_keys("enter");
    griffin.wait_for_text("not a line number: abc", WAIT);
    assert!(status_line(&griffin).contains("Ln 120, Col 1"));

    // Esc closes the prompt and moves nothing.
    griffin.send_keys("ctrl+g");
    griffin.wait_for_text("Go to line:", WAIT);
    griffin.type_text("5");
    griffin.send_keys("esc");
    griffin.wait_for_text_gone("Go to line:", WAIT);
    assert!(status_line(&griffin).contains("Ln 120, Col 1"));
}
