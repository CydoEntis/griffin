mod harness;

use std::path::Path;
use std::time::Duration;

use harness::{Griffin, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// `src/lib.rs` has `// TODO` on line 3 and `todo_free` on line 4, `src/main.rs`
/// an indented `// TODO` on line 2. `.gitignore` hides `ignored.log`, and
/// `data.bin` is binary; both hold `TODO` too.
const PROJECT: &str = "tests/fixtures/search";
/// The default theme's accent, hydra's lime `#c3f53c`.
const ACCENT: vt100::Color = vt100::Color::Rgb(0xc3, 0xf5, 0x3c);

/// At 100x30 the card is 80 wide and 20 tall, centred: its border starts at
/// column 10 and row 5, the query is on row 6 and the hits start on row 7, one
/// space in from the border.
const QUERY_ROW: u16 = 6;
const FIRST_ROW: u16 = 7;
const QUERY_X: u16 = 11;
const LIST_X: u16 = 12;
/// The list's width inside the border.
const LIST_WIDTH: usize = 78;

const LIB_TODO: &str = "src/lib.rs:3: // TODO: write the library";
const LIB_FN: &str = "src/lib.rs:4: pub fn todo_free() {}";
const MAIN_TODO: &str = "src/main.rs:2: // TODO tidy";

/// Griffin in `dir` with no file open, so the root is that folder.
fn open_in(dir: &Path) -> Griffin {
    let griffin = Griffin::spawn_in(dir, &[]);
    griffin.wait_for_text("Ln 1, Col 1", START);
    griffin
}

/// Opens the panel, types `query` and presses Enter.
fn search(griffin: &mut Griffin, query: &str) {
    griffin.send_keys("alt+f");
    griffin.wait_for_text("Search:", WAIT);
    griffin.type_text(query);
    griffin.wait_for_text(&format!("Search: {query}"), WAIT);
    griffin.send_keys("enter");
}

fn row(griffin: &Griffin, row: u16) -> String {
    griffin.screen()[usize::from(row)].clone()
}

/// Waits for the query line to end with `status`, e.g. `3 hits`.
fn wait_for_status(griffin: &Griffin, status: &str) {
    griffin.wait_for_screen(&format!("the panel to say {status:?}"), WAIT, |lines| {
        // The status ends one space before the card's right border.
        lines[usize::from(QUERY_ROW)].contains(&format!("  {status} │"))
    });
}

/// The accent text on the query line: the toggles that are on. Spaces don't
/// count, as ConPTY may redraw blanks in whatever colour came last.
fn toggles_on(griffin: &Griffin) -> String {
    griffin.fg_text(QUERY_ROW, ACCENT).replace(' ', "")
}

/// What a selected row reads reversed: the hit one space in, padded to the
/// list's width.
fn selected(hit: &str) -> String {
    format!(" {hit:<width$}", width = LIST_WIDTH - 1)
}

/// Waits for the status line to end with `Ln <line>, Col <col>`.
fn wait_for_position(griffin: &Griffin, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    griffin.wait_for_screen(&position, WAIT, |lines| {
        lines[usize::from(ROWS - 1)].ends_with(&position)
    });
}

#[test]
fn alt_f_lists_hits_as_path_line_text_respecting_gitignore_and_skipping_binaries() {
    let mut griffin = open_in(Path::new(PROJECT));
    search(&mut griffin, "TODO");
    wait_for_status(&griffin, "3 hits");
    assert_eq!(griffin.text_col(QUERY_ROW, "Search: TODO"), Some(QUERY_X));
    assert!(row(&griffin, QUERY_ROW).contains("Aa  .*  3 hits"));
    // Sorted by path, then line.
    for (i, hit) in [LIB_TODO, LIB_FN, MAIN_TODO].iter().enumerate() {
        let y = FIRST_ROW + u16::try_from(i).unwrap();
        assert_eq!(griffin.text_col(y, hit), Some(LIST_X), "{hit}");
    }
    let screen = griffin.screen().join("\n");
    assert!(!screen.contains("ignored.log"), "{screen}");
    assert!(!screen.contains("data.bin"), "{screen}");
    // The first hit starts selected; the others show their match in the accent.
    griffin.wait_for_reversed(FIRST_ROW, &selected(LIB_TODO), WAIT);
    griffin.wait_for_fg(FIRST_ROW + 1, ACCENT, "todo", WAIT);
    griffin.wait_for_fg(FIRST_ROW + 2, ACCENT, "TODO", WAIT);
    // The cursor stays in the query.
    griffin.wait_for_cursor(QUERY_X + "Search: TODO".len() as u16, QUERY_ROW, WAIT);
}

#[test]
fn alt_c_and_alt_r_toggle_case_and_regex_and_search_again() {
    let mut griffin = open_in(Path::new(PROJECT));
    search(&mut griffin, "T.DO");
    wait_for_status(&griffin, "no hits");
    // As a regex `.` matches the `O`; case still ignored, so `todo_free` too.
    griffin.send_keys("alt+r");
    wait_for_status(&griffin, "3 hits");
    assert_eq!(toggles_on(&griffin), ".*");
    griffin.send_keys("alt+c");
    wait_for_status(&griffin, "2 hits");
    assert_eq!(toggles_on(&griffin), "Aa.*");
    griffin.wait_for_text_gone("todo_free", WAIT);
    assert_eq!(griffin.text_col(FIRST_ROW, LIB_TODO), Some(LIST_X));
    assert_eq!(griffin.text_col(FIRST_ROW + 1, MAIN_TODO), Some(LIST_X));

    // A bad regex says so instead of listing anything.
    griffin.type_text("(");
    griffin.send_keys("enter");
    wait_for_status(&griffin, "invalid regex");
    griffin.wait_for_text_gone("src/lib.rs", WAIT);

    griffin.send_keys("esc");
    griffin.wait_for_text_gone("Search:", WAIT);
}

#[test]
fn arrows_move_through_hits_and_enter_opens_the_file_at_the_match() {
    let mut griffin = open_in(Path::new(PROJECT));
    search(&mut griffin, "TODO");
    wait_for_status(&griffin, "3 hits");
    griffin.send_keys("down");
    griffin.send_keys("down");
    griffin.wait_for_reversed(FIRST_ROW + 2, &selected(MAIN_TODO), WAIT);
    griffin.send_keys("up");
    griffin.wait_for_reversed(FIRST_ROW + 1, &selected(LIB_FN), WAIT);
    griffin.send_keys("down");
    griffin.wait_for_reversed(FIRST_ROW + 2, &selected(MAIN_TODO), WAIT);
    griffin.send_keys("enter");
    griffin.wait_for_text_gone("Search:", WAIT);
    griffin.wait_for_text("1 │ fn main() {", WAIT);
    // `    // TODO`: the match starts in column 8.
    wait_for_position(&griffin, 2, 8);
    griffin.wait_for_screen("main.rs in the tab bar", WAIT, |screen| {
        screen[0].contains("main.rs")
    });

    // Another hit opens beside it in a second tab.
    search(&mut griffin, "TODO");
    wait_for_status(&griffin, "3 hits");
    griffin.send_keys("enter");
    griffin.wait_for_text_gone("Search:", WAIT);
    griffin.wait_for_text("1 │ //! Fixture library.", WAIT);
    wait_for_position(&griffin, 3, 4);
    griffin.wait_for_screen("both files in the tab bar", WAIT, |screen| {
        screen[0].contains("main.rs") && screen[0].contains("lib.rs")
    });
}

#[test]
fn unsaved_edits_in_open_buffers_are_searched_from_memory() {
    let mut griffin = open_in(Path::new(PROJECT));
    search(&mut griffin, "tidy");
    wait_for_status(&griffin, "1 hit");
    griffin.send_keys("enter");
    wait_for_position(&griffin, 2, 13);
    // Unsaved, so only the buffer has it.
    griffin.type_text("XYZZY");
    griffin.wait_for_text("// TODO XYZZYtidy", WAIT);
    search(&mut griffin, "xyzzy");
    wait_for_status(&griffin, "1 hit");
    griffin.wait_for_reversed(
        FIRST_ROW,
        &selected("src/main.rs:2: // TODO XYZZYtidy"),
        WAIT,
    );
}

#[test]
fn many_files_stream_in_and_the_panel_keeps_taking_keys() -> std::io::Result<()> {
    let dir = tempfile::tempdir()?;
    for n in 0..3000 {
        std::fs::write(
            dir.path().join(format!("f{n:04}.txt")),
            "nothing\nneedle here\n",
        )?;
    }
    let mut griffin = open_in(dir.path());
    search(&mut griffin, "needle");
    // Keys reach the panel straight away, while the search may still be going.
    griffin.send_keys("down");
    wait_for_status(&griffin, "3000 hits");
    // The frame with the final count has the selection drawn too.
    assert!(
        (FIRST_ROW..ROWS).any(|y| griffin.reversed_text(y).contains(".txt:2: needle here")),
        "{}",
        griffin.screen().join(
            "
"
        )
    );
    griffin.send_keys("esc");
    griffin.wait_for_text_gone("Search:", WAIT);
    Ok(())
}
