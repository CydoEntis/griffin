//! The key list the editor area shows whenever no file is open and the splash
//! isn't up (glyph-splash spec S11): after the last tab closes and after
//! another folder is opened. Where it's drawn, that typing doesn't reach the
//! untitled buffer under it, and what replaces it.

mod harness;

use std::fs;
use std::time::Duration;

use harness::{Glyph, ROWS};
use tempfile::TempDir;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const STATUS: u16 = ROWS - 1;
/// The pills sit on row 1, under a blank row 0.
const PILL_ROW: u16 = 1;

/// Without the tree the editor area is the full width from row 3 (below the
/// tab header) to the status line: 100 × 26. The list's header starts a third
/// of the way down, on row 3 + 26 / 3 = 11, and its rows are two apart. The
/// block is 35 wide (the label column at 10, then `cast · files and
/// commands`), centred: (100 − 35) / 2 = 32.
const HEADER_ROW: u16 = 11;
const KEY_ROWS: [u16; 4] = [13, 15, 17, 19];
const X: u16 = 32;
const LABEL_X: u16 = X + 10;

const KEYS: [(&str, &str); 4] = [
    ("Ctrl+P", "cast · files and commands"),
    ("Ctrl+N", "new file"),
    ("Ctrl+B", "toggle tree"),
    ("Ctrl+Q", "quit"),
];

/// A temp folder holding `proj/a.txt` ("alpha") and `proj/b.txt` ("bravo"),
/// with glyph started on `a.txt` inside `proj`, so the project is `proj` and
/// the tree is hidden.
fn open_file() -> (TempDir, Glyph) {
    let dir = tempfile::tempdir().expect("create temp dir");
    let proj = dir.path().join("proj");
    fs::create_dir(&proj).expect("create proj");
    fs::write(proj.join("a.txt"), "alpha\n").expect("write a.txt");
    fs::write(proj.join("b.txt"), "bravo\n").expect("write b.txt");
    let glyph = Glyph::spawn_in(&proj, &["a.txt"]);
    glyph.wait_for_text("1  alpha", START);
    (dir, glyph)
}

/// Waits for the key list, then checks every line of it is where the design
/// puts it, starting from column `x` with its header on `header`.
fn assert_key_list(glyph: &Glyph, folder: &str, x: u16, header: u16) {
    glyph.wait_for_text("no file open", WAIT);
    let screen = glyph.screen();
    assert_eq!(
        glyph.text_col(header, &format!("{folder}/  no file open")),
        Some(x),
        "{screen:#?}"
    );
    for (i, (key, label)) in (0u16..).zip(KEYS) {
        let row = header + 2 + 2 * i;
        assert_eq!(glyph.text_col(row, key), Some(x), "{screen:#?}");
        assert_eq!(glyph.text_col(row, label), Some(x + 10), "{screen:#?}");
        // The rows are two apart, with nothing between.
        assert_eq!(
            screen[usize::from(row - 1)]
                .chars()
                .skip(usize::from(x))
                .collect::<String>()
                .trim(),
            "",
            "{screen:#?}"
        );
    }
}

/// The pill row shows no tab.
fn assert_no_tabs(glyph: &Glyph) {
    let screen = glyph.screen();
    let pills = &screen[usize::from(PILL_ROW)];
    assert!(!pills.contains('▐'), "{screen:#?}");
    assert!(!pills.contains("untitled"), "{screen:#?}");
}

#[test]
fn closing_the_last_tab_shows_the_key_list_under_an_empty_tab_bar() {
    let (_dir, mut glyph) = open_file();
    glyph.send_keys("ctrl+w");
    glyph.wait_for_text_gone("alpha", WAIT);
    assert_key_list(&glyph, "proj", X, HEADER_ROW);
    for (row, (key, _)) in KEY_ROWS.into_iter().zip(KEYS) {
        assert_eq!(glyph.text_col(row, key), Some(X));
    }
    assert_eq!(glyph.text_col(KEY_ROWS[0], "cast"), Some(LABEL_X));
    assert_no_tabs(&glyph);
    // The status bar names the project, not an untitled buffer.
    let status = glyph.screen()[usize::from(STATUS)].clone();
    assert!(status.contains("proj"), "{status:?}");
    assert!(!status.contains("untitled"), "{status:?}");
}

#[test]
fn typing_on_the_key_list_does_nothing_and_ctrl_n_replaces_it() {
    let (_dir, mut glyph) = open_file();
    glyph.send_keys("ctrl+w");
    glyph.wait_for_text("no file open", WAIT);
    glyph.type_text("zq");
    glyph.send_keys("enter");
    glyph.send_keys("backspace");
    // Ctrl+B shows the tree beside it; the tree's update proves the keys
    // above were handled first.
    glyph.send_keys("ctrl+b");
    glyph.wait_for_text("b.txt", WAIT);
    let screen = glyph.screen().join("\n");
    assert!(screen.contains("no file open"), "{screen}");
    assert!(!screen.contains("zq"), "{screen}");
    assert!(!screen.contains("untitled"), "{screen}");

    // Ctrl+N gives one empty untitled tab, which takes the typing.
    glyph.send_keys("ctrl+n");
    glyph.wait_for_text_gone("no file open", WAIT);
    glyph.wait_for_text("▐ untitled ▌", WAIT);
    glyph.type_text("hi");
    glyph.wait_for_text("1  hi", WAIT);
    let pills = glyph.screen()[usize::from(PILL_ROW)].clone();
    assert_eq!(pills.matches("untitled").count(), 1, "{pills:?}");
}

#[test]
fn opening_a_file_from_ctrl_p_replaces_the_key_list() {
    let (_dir, mut glyph) = open_file();
    glyph.send_keys("ctrl+w");
    glyph.wait_for_text("no file open", WAIT);
    glyph.send_keys("ctrl+p");
    glyph.wait_for_text("cast", WAIT);
    glyph.type_text("b.txt");
    glyph.wait_for_text("✦ b.txt", WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text("1  bravo", WAIT);
    glyph.wait_for_text_gone("no file open", WAIT);
    let pills = glyph.screen()[usize::from(PILL_ROW)].clone();
    assert!(pills.contains("▐ b.txt ▌"), "{pills:?}");
    assert!(!pills.contains("untitled"), "{pills:?}");
}

#[test]
fn opening_a_folder_shows_the_key_list_beside_its_tree() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let other = dir.path().join("other");
    fs::create_dir(&other).expect("create other");
    fs::write(other.join("inside.txt"), "inside\n").expect("write inside.txt");
    let mut glyph = Glyph::spawn_in(dir.path(), &["."]);
    glyph.wait_for_text("Open directory", START);
    open_other(&mut glyph);

    // With the tree, the list sits in the editor area right of it; its lines
    // still start in one column, two rows apart.
    glyph.wait_for_text("inside.txt", WAIT);
    glyph.wait_for_text("no file open", WAIT);
    let header = (0..ROWS)
        .find(|&row| glyph.text_col(row, "no file open").is_some())
        .expect("the header is on screen");
    let x = glyph
        .text_col(header, "other/")
        .expect("the folder heads the list");
    assert_key_list(&glyph, "other", x, header);
    assert_no_tabs(&glyph);

    // The tree has the keys: Enter opens the file it selects in place of the
    // list.
    glyph.send_keys("enter");
    glyph.wait_for_text("1  inside", WAIT);
    glyph.wait_for_text_gone("no file open", WAIT);
}

/// Opens `other`, the one folder in the project, through the splash's folder
/// browser: `o`, ↓ to `other`, Enter into it, Enter on its open row. The
/// browser's header is row 6 and its folders start on row 9.
fn open_other(glyph: &mut Glyph) {
    glyph.type_text("o");
    glyph.wait_for_text("open · folders", WAIT);
    glyph.wait_for_screen("other in the browser", WAIT, |screen| {
        screen[9].contains("▸ other")
    });
    glyph.send_keys("down");
    glyph.send_keys("enter");
    glyph.wait_for_screen("inside other", WAIT, |screen| screen[6].contains("other"));
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("open · folders", WAIT);
}
