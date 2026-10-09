mod harness;

use std::path::Path;
use std::time::Duration;

use harness::{ROWS, Tome};
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// `src/lib.rs` has `// TODO` on line 3 and `todo_free` on line 4, `src/main.rs`
/// an indented `// TODO` on line 2. `.gitignore` hides `ignored.log`, and
/// `data.bin` is binary; both hold `TODO` too.
const PROJECT: &str = "tests/fixtures/search";

/// The default theme, hydra: its `accent`, `raised`, `bg`, `warn` and `scrim`.
const ACCENT: Color = Color::Rgb(0xc3, 0xf5, 0x3c);
const RAISED: Color = Color::Rgb(0x0f, 0x18, 0x21);
const BG: Color = Color::Rgb(0x07, 0x0b, 0x10);
const WARN: Color = Color::Rgb(0xff, 0xb5, 0x47);
const SCRIM: Color = Color::Rgb(0x04, 0x06, 0x08);

/// At 100x30 the card is 80 wide and 22 tall, centred (SPEC_V1_LAYOUT §7.3): it
/// starts at column 10 and row 4 with its lit edge, then the Search field, the
/// Replace field, the match count, and the hits from row 8.
const CARD_X: u16 = 10;
const CARD_Y: u16 = 4;
const QUERY_ROW: u16 = 5;
const STATUS_ROW: u16 = 7;
const FIRST_ROW: u16 = 8;
/// Where the Search label starts, after `✦ `, and where its value starts.
const LABEL_X: u16 = 14;
const VALUE_X: u16 = 23;
/// Where a hit's `path:line` starts.
const LIST_X: u16 = 12;
/// `src/main.rs:2`, the widest `path:line`, sets the column's width.
const PLACE_WIDTH: usize = 13;
const FOOTER: &str = "↑↓ select   ⏎ open   tab replace field   alt+a replace all   esc close";

const LIB_TODO: (&str, &str) = ("src/lib.rs:3", "// TODO: write the library");
const LIB_FN: (&str, &str) = ("src/lib.rs:4", "pub fn todo_free() {}");
const MAIN_TODO: (&str, &str) = ("src/main.rs:2", "// TODO tidy");

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

/// A hit row as the list shows it: `path:line` padded to the column, two
/// blanks, then the line.
fn hit_row((place, text): (&str, &str)) -> String {
    format!("{place:<PLACE_WIDTH$}  {text}")
}

/// Tome in `dir` with no file open, so the root is that folder.
fn open_in(dir: &Path) -> Tome {
    let tome = Tome::spawn_in(dir, &[]);
    tome.wait_for_text("Open directory", START);
    tome
}

/// Opens the panel, types `query` and presses Enter.
fn search(tome: &mut Tome, query: &str) {
    tome.send_keys("alt+f");
    tome.wait_for_text(FOOTER, WAIT);
    tome.type_text(query);
    tome.wait_for_text(&format!("Search   {query}"), WAIT);
    tome.send_keys("enter");
}

fn row(tome: &Tome, row: u16) -> String {
    tome.screen()[usize::from(row)].clone()
}

/// Waits for the row under the fields to say exactly `status`, e.g.
/// `3 matches in 2 files`, and not `searching… 3 matches in 2 files`.
fn wait_for_status(tome: &Tome, status: &str) {
    tome.wait_for_screen(&format!("the panel to say {status:?}"), WAIT, |lines| {
        let shown: String = lines[usize::from(STATUS_ROW)]
            .chars()
            .skip(usize::from(CARD_X) + 2)
            .collect();
        shown.trim_end() == status
    });
}

/// The chips on the Search field that are on: `acc_ink` on the accent.
fn chips_on(tome: &Tome) -> String {
    tome.bg_text(QUERY_ROW, ACCENT).replace(' ', "")
}

/// Waits for `hit` to be the selected row `y`: the glow row.
fn wait_for_selected(tome: &Tome, y: u16, hit: (&str, &str)) {
    tome.wait_for_bg(CARD_X, y, glow(), WAIT);
    assert_eq!(tome.text_col(y, &hit_row(hit)), Some(LIST_X), "{hit:?}");
}

/// Waits for the status line to show `Ln <line>, Col <col>` (see `shows_position`).
fn wait_for_position(tome: &Tome, line: usize, col: usize) {
    let position = format!("Ln {line}, Col {col}");
    tome.wait_for_screen(&position, WAIT, |lines| {
        harness::shows_position(&lines[usize::from(ROWS - 1)], &position)
    });
}

#[test]
fn alt_f_lists_hits_as_path_line_text_respecting_gitignore_and_skipping_binaries() {
    let mut tome = open_in(Path::new(PROJECT));
    search(&mut tome, "TODO");
    wait_for_status(&tome, "3 matches in 2 files");
    assert_eq!(tome.text_col(QUERY_ROW, "Search   TODO"), Some(LABEL_X));
    assert!(row(&tome, QUERY_ROW).contains(" Aa   .*"));
    // Sorted by path, then line, the text lined up after the widest place.
    for (i, hit) in [LIB_TODO, LIB_FN, MAIN_TODO].into_iter().enumerate() {
        let y = FIRST_ROW + u16::try_from(i).unwrap();
        assert_eq!(tome.text_col(y, &hit_row(hit)), Some(LIST_X), "{hit:?}");
    }
    let screen = tome.screen().join("\n");
    assert!(!screen.contains("ignored.log"), "{screen}");
    assert!(!screen.contains("data.bin"), "{screen}");
    // The first hit starts selected; the others show their match on
    // `find_match_bg`.
    wait_for_selected(&tome, FIRST_ROW, LIB_TODO);
    let match_bg = mix(BG, WARN, 0.3);
    assert_eq!(tome.bg_text(FIRST_ROW + 1, match_bg), "todo");
    assert_eq!(tome.bg_text(FIRST_ROW + 2, match_bg), "TODO");
    // The cursor stays in the query.
    tome.wait_for_cursor(VALUE_X + "TODO".len() as u16, QUERY_ROW, WAIT);
}

#[test]
fn the_panel_is_a_lit_card_over_a_dimmed_screen() {
    let mut tome = Tome::spawn_in(Path::new(PROJECT), &["src/lib.rs"]);
    tome.wait_for_text("Fixture library", START);
    let y = (0..ROWS)
        .find(|&y| row(&tome, y).contains("Fixture library"))
        .unwrap();
    // Above the card, so it's only dimmed.
    assert!(y < CARD_Y, "the editor's first line is on row {y}");
    let x = tome.text_col(y, "Fixture").unwrap();
    let (fg, bg) = (tome.fg_at(x, y), tome.bg_at(x, y));
    search(&mut tome, "TODO");
    wait_for_status(&tome, "3 matches in 2 files");
    tome.wait_for_fg_at(x, y, mix(fg, SCRIM, 0.6), WAIT);
    tome.wait_for_bg(x, y, mix(bg, SCRIM, 0.6), WAIT);

    // No border: the lit edge across the top, then the card's own rows.
    assert_eq!(
        tome.text_col(CARD_Y, &"▀".repeat(80)),
        Some(CARD_X),
        "{}",
        row(&tome, CARD_Y)
    );
    assert_eq!(tome.fg_at(CARD_X, CARD_Y), ACCENT);
    let right_edge = row(&tome, QUERY_ROW).chars().nth(usize::from(CARD_X + 79));
    assert_ne!(right_edge, Some('│'), "no border");
    assert_eq!(tome.text_col(QUERY_ROW, "✦ Search"), Some(LABEL_X - 2));
    assert_eq!(tome.text_col(QUERY_ROW + 1, "✦ Replace"), Some(LABEL_X - 2));
    assert_eq!(tome.text_col(CARD_Y + 21, FOOTER), Some(CARD_X + 2));
    // The selected row ends in `⏎`.
    assert_eq!(tome.text_col(FIRST_ROW, "⏎"), Some(CARD_X + 80 - 5));
}

#[test]
fn a_click_outside_the_card_closes_it_and_one_inside_does_not() {
    let mut tome = open_in(Path::new(PROJECT));
    tome.send_keys("alt+f");
    tome.wait_for_text(FOOTER, WAIT);
    tome.click(50, 15);
    // Still open: typing goes to the query.
    tome.type_text("x");
    tome.wait_for_text("Search   x", WAIT);
    tome.click(2, 2);
    tome.wait_for_text_gone(FOOTER, WAIT);
}

#[test]
fn mono_dims_with_the_modifier_and_reverses_the_selected_row() {
    let mut tome = Tome::spawn_in_with_config(Path::new(PROJECT), "theme = \"mono\"\n", &[]);
    tome.wait_for_text("Open directory", START);
    search(&mut tome, "TODO");
    wait_for_status(&tome, "3 matches in 2 files");
    // The frame with the count has the selection and the dim too.
    assert!(
        tome.reversed_text(FIRST_ROW).contains(&hit_row(LIB_TODO)),
        "{:?}",
        tome.reversed_text(FIRST_ROW)
    );
    let status = ROWS - 1;
    // The splash is still up under the card, so its keys are in the status line.
    let position = tome.text_col(status, "quit").unwrap();
    assert!(tome.dim_at(position, status), "the status line is dimmed");
    assert!(!tome.dim_at(CARD_X + 2, STATUS_ROW), "the card isn't");
}

#[test]
fn alt_c_and_alt_r_toggle_case_and_regex_and_search_again() {
    let mut tome = open_in(Path::new(PROJECT));
    search(&mut tome, "T.DO");
    wait_for_status(&tome, "no matches");
    // As a regex `.` matches the `O`; case still ignored, so `todo_free` too.
    tome.send_keys("alt+r");
    wait_for_status(&tome, "3 matches in 2 files");
    assert_eq!(chips_on(&tome), ".*");
    tome.send_keys("alt+c");
    wait_for_status(&tome, "2 matches in 2 files");
    assert_eq!(chips_on(&tome), "Aa.*");
    tome.wait_for_text_gone("todo_free", WAIT);
    assert_eq!(tome.text_col(FIRST_ROW, &hit_row(LIB_TODO)), Some(LIST_X));
    assert_eq!(
        tome.text_col(FIRST_ROW + 1, &hit_row(MAIN_TODO)),
        Some(LIST_X)
    );

    // A bad regex says so instead of listing anything.
    tome.type_text("(");
    tome.send_keys("enter");
    wait_for_status(&tome, "invalid regex");
    tome.wait_for_text_gone("src/lib.rs", WAIT);

    tome.send_keys("esc");
    tome.wait_for_text_gone(FOOTER, WAIT);
}

#[test]
fn arrows_move_through_hits_and_enter_opens_the_file_at_the_match() {
    let mut tome = open_in(Path::new(PROJECT));
    search(&mut tome, "TODO");
    wait_for_status(&tome, "3 matches in 2 files");
    tome.send_keys("down");
    tome.send_keys("down");
    wait_for_selected(&tome, FIRST_ROW + 2, MAIN_TODO);
    tome.send_keys("up");
    wait_for_selected(&tome, FIRST_ROW + 1, LIB_FN);
    tome.send_keys("down");
    wait_for_selected(&tome, FIRST_ROW + 2, MAIN_TODO);
    tome.send_keys("enter");
    tome.wait_for_text_gone(FOOTER, WAIT);
    tome.wait_for_text("1  fn main() {", WAIT);
    // `    // TODO`: the match starts in column 8.
    wait_for_position(&tome, 2, 8);
    tome.wait_for_screen("main.rs in the tab bar", WAIT, |screen| {
        screen[1].contains("main.rs")
    });

    // Another hit opens beside it in a second tab.
    search(&mut tome, "TODO");
    wait_for_status(&tome, "3 matches in 2 files");
    tome.send_keys("enter");
    tome.wait_for_text_gone(FOOTER, WAIT);
    tome.wait_for_text("1  //! Fixture library.", WAIT);
    wait_for_position(&tome, 3, 4);
    tome.wait_for_screen("both files in the tab bar", WAIT, |screen| {
        screen[1].contains("main.rs") && screen[1].contains("lib.rs")
    });
}

#[test]
fn unsaved_edits_in_open_buffers_are_searched_from_memory() {
    let mut tome = open_in(Path::new(PROJECT));
    search(&mut tome, "tidy");
    wait_for_status(&tome, "1 match in 1 file");
    tome.send_keys("enter");
    wait_for_position(&tome, 2, 13);
    // Unsaved, so only the buffer has it.
    tome.type_text("XYZZY");
    tome.wait_for_text("// TODO XYZZYtidy", WAIT);
    search(&mut tome, "xyzzy");
    wait_for_status(&tome, "1 match in 1 file");
    tome.wait_for_bg(CARD_X, FIRST_ROW, glow(), WAIT);
    assert_eq!(
        tome.text_col(FIRST_ROW, "src/main.rs:2  // TODO XYZZYtidy"),
        Some(LIST_X)
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
    let mut tome = open_in(dir.path());
    search(&mut tome, "needle");
    // Keys reach the panel straight away, while the search may still be going.
    tome.send_keys("down");
    wait_for_status(&tome, "3000 matches in 3000 files");
    // The frame with the final count has the selection drawn too.
    assert!(
        (FIRST_ROW..ROWS)
            .any(|y| tome.bg_at(CARD_X, y) == glow()
                && row(&tome, y).contains(".txt:2  needle here")),
        "{}",
        tome.screen().join("\n")
    );
    tome.send_keys("esc");
    tome.wait_for_text_gone(FOOTER, WAIT);
    Ok(())
}
