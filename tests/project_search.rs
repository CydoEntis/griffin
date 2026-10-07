mod harness;

use std::path::Path;
use std::time::Duration;

use harness::{Glyph, ROWS};

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

/// Glyph in `dir` with no file open, so the root is that folder.
fn open_in(dir: &Path) -> Glyph {
    let glyph = Glyph::spawn_in(dir, &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph
}

/// Opens the panel, types `query` and presses Enter.
fn search(glyph: &mut Glyph, query: &str) {
    glyph.send_keys("alt+f");
    glyph.wait_for_text("Search:", WAIT);
    glyph.type_text(query);
    glyph.wait_for_text(&format!("Search: {query}"), WAIT);
    glyph.send_keys("enter");
}

fn row(glyph: &Glyph, row: u16) -> String {
    glyph.screen()[usize::from(row)].clone()
}

/// Waits for the query line to end with `status`, e.g. `3 hits`.
fn wait_for_status(glyph: &Glyph, status: &str) {
    glyph.wait_for_screen(&format!("the panel to say {status:?}"), WAIT, |lines| {
        // The status ends one space before the card's right border.
        lines[usize::from(QUERY_ROW)].contains(&format!("  {status} │"))
    });
}

/// The accent text on the query line: the toggles that are on. Spaces don't
/// count, as ConPTY may redraw blanks in whatever colour came last.
fn toggles_on(glyph: &Glyph) -> String {
    glyph.fg_text(QUERY_ROW, ACCENT).replace(' ', "")
}

/// What a selected row reads reversed: the hit one space in, padded to the
/// list's width.
fn selected(hit: &str) -> String {
    format!(" {hit:<width$}", width = LIST_WIDTH - 1)
}

/// Waits for the status line to end with `Ln <line>, Col <col>`.
fn wait_for_position(glyph: &Glyph, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    glyph.wait_for_screen(&position, WAIT, |lines| {
        lines[usize::from(ROWS - 1)].ends_with(&position)
    });
}

#[test]
fn alt_f_lists_hits_as_path_line_text_respecting_gitignore_and_skipping_binaries() {
    let mut glyph = open_in(Path::new(PROJECT));
    search(&mut glyph, "TODO");
    wait_for_status(&glyph, "3 hits");
    assert_eq!(glyph.text_col(QUERY_ROW, "Search: TODO"), Some(QUERY_X));
    assert!(row(&glyph, QUERY_ROW).contains("Aa  .*  3 hits"));
    // Sorted by path, then line.
    for (i, hit) in [LIB_TODO, LIB_FN, MAIN_TODO].iter().enumerate() {
        let y = FIRST_ROW + u16::try_from(i).unwrap();
        assert_eq!(glyph.text_col(y, hit), Some(LIST_X), "{hit}");
    }
    let screen = glyph.screen().join("\n");
    assert!(!screen.contains("ignored.log"), "{screen}");
    assert!(!screen.contains("data.bin"), "{screen}");
    // The first hit starts selected; the others show their match in the accent.
    glyph.wait_for_reversed(FIRST_ROW, &selected(LIB_TODO), WAIT);
    glyph.wait_for_fg(FIRST_ROW + 1, ACCENT, "todo", WAIT);
    glyph.wait_for_fg(FIRST_ROW + 2, ACCENT, "TODO", WAIT);
    // The cursor stays in the query.
    glyph.wait_for_cursor(QUERY_X + "Search: TODO".len() as u16, QUERY_ROW, WAIT);
}

#[test]
fn alt_c_and_alt_r_toggle_case_and_regex_and_search_again() {
    let mut glyph = open_in(Path::new(PROJECT));
    search(&mut glyph, "T.DO");
    wait_for_status(&glyph, "no hits");
    // As a regex `.` matches the `O`; case still ignored, so `todo_free` too.
    glyph.send_keys("alt+r");
    wait_for_status(&glyph, "3 hits");
    assert_eq!(toggles_on(&glyph), ".*");
    glyph.send_keys("alt+c");
    wait_for_status(&glyph, "2 hits");
    assert_eq!(toggles_on(&glyph), "Aa.*");
    glyph.wait_for_text_gone("todo_free", WAIT);
    assert_eq!(glyph.text_col(FIRST_ROW, LIB_TODO), Some(LIST_X));
    assert_eq!(glyph.text_col(FIRST_ROW + 1, MAIN_TODO), Some(LIST_X));

    // A bad regex says so instead of listing anything.
    glyph.type_text("(");
    glyph.send_keys("enter");
    wait_for_status(&glyph, "invalid regex");
    glyph.wait_for_text_gone("src/lib.rs", WAIT);

    glyph.send_keys("esc");
    glyph.wait_for_text_gone("Search:", WAIT);
}

#[test]
fn arrows_move_through_hits_and_enter_opens_the_file_at_the_match() {
    let mut glyph = open_in(Path::new(PROJECT));
    search(&mut glyph, "TODO");
    wait_for_status(&glyph, "3 hits");
    glyph.send_keys("down");
    glyph.send_keys("down");
    glyph.wait_for_reversed(FIRST_ROW + 2, &selected(MAIN_TODO), WAIT);
    glyph.send_keys("up");
    glyph.wait_for_reversed(FIRST_ROW + 1, &selected(LIB_FN), WAIT);
    glyph.send_keys("down");
    glyph.wait_for_reversed(FIRST_ROW + 2, &selected(MAIN_TODO), WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("Search:", WAIT);
    glyph.wait_for_text("1 │ fn main() {", WAIT);
    // `    // TODO`: the match starts in column 8.
    wait_for_position(&glyph, 2, 8);
    glyph.wait_for_screen("main.rs in the tab bar", WAIT, |screen| {
        screen[0].contains("main.rs")
    });

    // Another hit opens beside it in a second tab.
    search(&mut glyph, "TODO");
    wait_for_status(&glyph, "3 hits");
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("Search:", WAIT);
    glyph.wait_for_text("1 │ //! Fixture library.", WAIT);
    wait_for_position(&glyph, 3, 4);
    glyph.wait_for_screen("both files in the tab bar", WAIT, |screen| {
        screen[0].contains("main.rs") && screen[0].contains("lib.rs")
    });
}

#[test]
fn unsaved_edits_in_open_buffers_are_searched_from_memory() {
    let mut glyph = open_in(Path::new(PROJECT));
    search(&mut glyph, "tidy");
    wait_for_status(&glyph, "1 hit");
    glyph.send_keys("enter");
    wait_for_position(&glyph, 2, 13);
    // Unsaved, so only the buffer has it.
    glyph.type_text("XYZZY");
    glyph.wait_for_text("// TODO XYZZYtidy", WAIT);
    search(&mut glyph, "xyzzy");
    wait_for_status(&glyph, "1 hit");
    glyph.wait_for_reversed(
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
    let mut glyph = open_in(dir.path());
    search(&mut glyph, "needle");
    // Keys reach the panel straight away, while the search may still be going.
    glyph.send_keys("down");
    wait_for_status(&glyph, "3000 hits");
    // The frame with the final count has the selection drawn too.
    assert!(
        (FIRST_ROW..ROWS).any(|y| glyph.reversed_text(y).contains(".txt:2: needle here")),
        "{}",
        glyph.screen().join(
            "
"
        )
    );
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("Search:", WAIT);
    Ok(())
}
