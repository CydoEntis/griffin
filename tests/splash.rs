//! The splash Glyph opens on with nothing to edit (glyph-splash spec S1–S5,
//! S7): when it shows, how it's drawn, its keys and clicks, *New file*, *New
//! directory* and *Open directory*, and that a project switch from it lands on
//! the tree beside an empty pane rather than on a new splash.

mod harness;

use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{Glyph, ROWS, pill_text};
use tempfile::TempDir;
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const PROJECT: &str = "tests/fixtures/project";
const STATUS: u16 = ROWS - 1;

/// hydra's roles, as the splash uses them.
const ACCENT: Color = Color::Rgb(0xc3, 0xf5, 0x3c);
const ACCENT2: Color = Color::Rgb(0x5a, 0xa9, 0xff);
const RAISED: Color = Color::Rgb(0x0f, 0x18, 0x21);
const MUTED: Color = Color::Rgb(0x71, 0x80, 0x8f);
const TEXT: Color = Color::Rgb(0xa7, 0xb4, 0xc2);
const STRONG: Color = Color::Rgb(0xf2, 0xf6, 0xf8);
const BG: Color = Color::Rgb(0x07, 0x0b, 0x10);
/// The wordmark's letters, accent → accent2, as on the tree's brand row.
const BRAND: [Color; 5] = [
    Color::Rgb(0xc3, 0xf5, 0x3c),
    Color::Rgb(0xa9, 0xe2, 0x6d),
    Color::Rgb(0x8f, 0xcf, 0x9e),
    Color::Rgb(0x74, 0xbc, 0xce),
    Color::Rgb(0x5a, 0xa9, 0xff),
];

/// Without the tree the editor area is the full width from row 3 (below the
/// tab header) to the status line: 100 × 26. The 52 × 11 card is centred in
/// it.
const CARD_X: u16 = 24;
const CARD_Y: u16 = 10;
const CARD_W: u16 = 52;
/// Its rows: the lit edge, a blank, the wordmark, the path, a blank, the three
/// actions, a blank, the footer and a blank.
const BRAND_ROW: u16 = CARD_Y + 2;
const PATH_ROW: u16 = CARD_Y + 3;
const NEW_FILE_ROW: u16 = CARD_Y + 5;
const NEW_DIR_ROW: u16 = CARD_Y + 6;
const OPEN_DIR_ROW: u16 = CARD_Y + 7;
const FOOTER_ROW: u16 = CARD_Y + 9;
/// Text starts four cells into the card; the letters end four cells short of
/// its right edge.
const TEXT_X: u16 = CARD_X + 4;
const KEY_X: u16 = CARD_X + CARD_W - 5;
const FOOTER: &str = "ctrl+p go to file · ctrl+q quit";
/// The folder browser's scope label, right of its header.
const BROWSER: &str = "open · folders";

/// `a` moved `t` of the way to `b`, per channel, rounded, as the editor mixes.
fn mix(a: Color, b: Color, t: f64) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (a, b) else {
        panic!("can't mix {a:?} and {b:?}");
    };
    let m = |p: u8, q: u8| (f64::from(p) + (f64::from(q) - f64::from(p)) * t).round() as u8;
    Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
}

/// The cells of `row` from `x`, `width` wide, trimmed at the end.
fn cells(glyph: &Glyph, row: u16, x: u16, width: u16) -> String {
    glyph.screen()[usize::from(row)]
        .chars()
        .skip(usize::from(x))
        .take(usize::from(width))
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// How the splash names `dir`: absolute, its start cut with `…` past the 44
/// cells the card has for it.
fn shown_path(dir: &Path) -> String {
    let path = std::path::absolute(dir)
        .expect("absolute path")
        .display()
        .to_string();
    let chars: Vec<char> = path.chars().collect();
    if chars.len() <= 44 {
        path
    } else {
        std::iter::once('…')
            .chain(chars[chars.len() - 43..].iter().copied())
            .collect()
    }
}

/// Waits for the `✦` marking the selected action to be on `row`.
fn wait_for_selected(glyph: &Glyph, x: u16, row: u16) {
    glyph.wait_for_screen(&format!("✦ on row {row}"), WAIT, |screen| {
        screen[usize::from(row)].chars().nth(usize::from(x)) == Some('✦')
    });
}

fn wait_for_status(glyph: &Glyph, text: &str) {
    glyph.wait_for_screen(&format!("status {text:?}"), WAIT, |screen| {
        screen[usize::from(STATUS)].contains(text)
    });
}

/// A temp folder holding `a.txt`, with Glyph started in it with no argument.
fn start_bare() -> (TempDir, Glyph) {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join("a.txt"), "alpha\n").expect("write a.txt");
    let glyph = Glyph::spawn_in(dir.path(), &[]);
    glyph.wait_for_text("Open directory", START);
    (dir, glyph)
}

#[test]
fn glyph_alone_and_glyph_on_a_folder_show_the_splash_but_a_file_does_not() {
    let (_dir, _glyph) = start_bare();

    let glyph = Glyph::spawn(&[PROJECT]);
    glyph.wait_for_text("README.md", START);
    glyph.wait_for_text("Open directory", WAIT);

    let glyph = Glyph::spawn(&["tests/fixtures/project/notes.txt"]);
    glyph.wait_for_text("notes for the tree test", START);
    let screen = glyph.screen().join("\n");
    assert!(!screen.contains("Open directory"), "{screen}");
}

#[test]
fn the_card_is_centred_with_the_wordmark_path_actions_and_footer() {
    let (dir, glyph) = start_bare();
    glyph.wait_for_fg_at(TEXT_X, BRAND_ROW, ACCENT2, WAIT);

    // No pills or thread in the header rows.
    let screen = glyph.screen();
    for row in 0..3 {
        assert_eq!(screen[row].trim(), "", "header row {row}: {screen:#?}");
    }

    // The lit edge across the card, on `raised`.
    assert_eq!(cells(&glyph, CARD_Y, CARD_X, CARD_W), "▀".repeat(52));
    assert_eq!(glyph.fg_at(CARD_X, CARD_Y), ACCENT);
    assert_eq!(glyph.bg_at(CARD_X, CARD_Y + 1), RAISED);
    assert_eq!(glyph.bg_at(CARD_X - 1, CARD_Y + 1), BG);

    // `✦ glyph` with the brand ramp, bold.
    assert_eq!(cells(&glyph, BRAND_ROW, TEXT_X, 44), "✦ glyph");
    for (x, color) in (TEXT_X + 2..).zip(BRAND) {
        assert_eq!(glyph.fg_at(x, BRAND_ROW), color, "wordmark column {x}");
        assert!(glyph.bold_at(x, BRAND_ROW));
    }

    // The project folder's absolute path in `muted`.
    assert_eq!(cells(&glyph, PATH_ROW, TEXT_X, 44), shown_path(dir.path()));
    assert_eq!(glyph.fg_at(TEXT_X, PATH_ROW), MUTED);

    // The actions: the first selected on the glow, the letters right-aligned.
    let row =
        |marker: &str, label: &str, key: &str| format!("{:<43}{key}", format!("{marker} {label}"));
    assert_eq!(
        cells(&glyph, NEW_FILE_ROW, TEXT_X, 44),
        row("✦", "New file", "n")
    );
    assert_eq!(
        cells(&glyph, NEW_DIR_ROW, TEXT_X, 44),
        row("▸", "New directory", "d")
    );
    assert_eq!(
        cells(&glyph, OPEN_DIR_ROW, TEXT_X, 44),
        row("▸", "Open directory", "o")
    );
    assert_eq!(glyph.bg_at(CARD_X, NEW_FILE_ROW), mix(RAISED, ACCENT, 0.3));
    assert_eq!(glyph.fg_at(TEXT_X, NEW_FILE_ROW), ACCENT2);
    assert_eq!(glyph.fg_at(TEXT_X + 2, NEW_FILE_ROW), STRONG);
    assert_eq!(glyph.fg_at(KEY_X, NEW_FILE_ROW), MUTED);
    assert_eq!(glyph.bg_at(CARD_X, NEW_DIR_ROW), RAISED);
    assert_eq!(glyph.fg_at(TEXT_X, NEW_DIR_ROW), MUTED);
    assert_eq!(glyph.fg_at(TEXT_X + 2, NEW_DIR_ROW), TEXT);
    assert_eq!(glyph.fg_at(KEY_X, NEW_DIR_ROW), MUTED);

    // The footer in `muted`, and nothing below the card.
    assert_eq!(cells(&glyph, FOOTER_ROW, TEXT_X, 44), FOOTER);
    assert_eq!(glyph.fg_at(TEXT_X, FOOTER_ROW), MUTED);
    assert_eq!(glyph.bg_at(CARD_X, CARD_Y + 10), RAISED);
    assert_eq!(glyph.bg_at(CARD_X, CARD_Y + 11), BG);
}

#[test]
fn mono_reverses_the_selected_row() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let mut glyph = Glyph::spawn_in_with_config(dir.path(), "theme = \"mono\"\n", &[]);
    glyph.wait_for_text("Open directory", START);
    let selected = format!("{:<4}{:<43}n    ", "", "✦ New file");
    glyph.wait_for_reversed(NEW_FILE_ROW, &selected, WAIT);
    assert_eq!(glyph.reversed_text(NEW_DIR_ROW), "");

    glyph.send_keys("down");
    let selected = format!("{:<4}{:<43}d    ", "", "✦ New directory");
    glyph.wait_for_reversed(NEW_DIR_ROW, &selected, WAIT);
    assert_eq!(glyph.reversed_text(NEW_FILE_ROW), "");
}

#[test]
fn arrows_move_and_wrap_and_enter_or_a_letter_runs_a_row() {
    let (_dir, mut glyph) = start_bare();
    wait_for_selected(&glyph, TEXT_X, NEW_FILE_ROW);
    glyph.send_keys("up");
    wait_for_selected(&glyph, TEXT_X, OPEN_DIR_ROW);
    glyph.send_keys("down");
    wait_for_selected(&glyph, TEXT_X, NEW_FILE_ROW);
    glyph.send_keys("down");
    wait_for_selected(&glyph, TEXT_X, NEW_DIR_ROW);

    // Enter on New directory opens the tree's `A` prompt; Esc comes back.
    glyph.send_keys("enter");
    glyph.wait_for_text(" New folder  ", WAIT);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(" New folder  ", WAIT);
    glyph.send_keys("down");
    wait_for_selected(&glyph, TEXT_X, OPEN_DIR_ROW);
    // Enter on Open directory opens the folder browser; Esc comes back.
    glyph.send_keys("enter");
    glyph.wait_for_text(BROWSER, WAIT);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(BROWSER, WAIT);

    // The letters run their rows whatever is selected.
    glyph.type_text("d");
    glyph.wait_for_text(" New folder  ", WAIT);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(" New folder  ", WAIT);
    glyph.type_text("o");
    glyph.wait_for_text(BROWSER, WAIT);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(BROWSER, WAIT);
    glyph.type_text("n");
    glyph.wait_for_text(" New file  ", WAIT);
    // No placeholder is left.
    let screen = glyph.screen().join("\n");
    assert!(!screen.contains("coming soon"), "{screen}");
}

#[test]
fn a_click_on_a_row_runs_it() {
    let (_dir, mut glyph) = start_bare();
    glyph.click(TEXT_X + 4, NEW_DIR_ROW);
    glyph.wait_for_text(" New folder  ", WAIT);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(" New folder  ", WAIT);
    wait_for_selected(&glyph, TEXT_X, NEW_DIR_ROW);
    glyph.click(TEXT_X + 4, OPEN_DIR_ROW);
    glyph.wait_for_text(BROWSER, WAIT);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(BROWSER, WAIT);
    wait_for_selected(&glyph, TEXT_X, OPEN_DIR_ROW);
    // Off the rows a click does nothing: no cursor is placed in the hidden
    // buffer, and the selection stays.
    glyph.click(5, 5);
    glyph.click(TEXT_X + 4, BRAND_ROW);
    wait_for_selected(&glyph, TEXT_X, OPEN_DIR_ROW);
    glyph.wait_for_text("Open directory", WAIT);
    glyph.click(TEXT_X + 4, NEW_FILE_ROW);
    glyph.wait_for_text(" New file  ", WAIT);
}

#[test]
fn esc_or_ctrl_n_leaves_the_splash_for_the_untitled_buffer() {
    for key in ["esc", "ctrl+n"] {
        let (_dir, mut glyph) = start_bare();
        glyph.send_keys(key);
        glyph.wait_for_text_gone("Open directory", WAIT);
        // One untitled tab, the one the splash stood in for, taking the typing.
        glyph.wait_for_text(&pill_text(&["untitled"], 0), WAIT);
        glyph.type_text("hi");
        glyph.wait_for_text("1  hi", WAIT);
    }
}

#[test]
fn opening_a_file_with_ctrl_p_replaces_the_splash() {
    let mut glyph = Glyph::spawn_in(Path::new(PROJECT), &[]);
    glyph.wait_for_text("Open directory", START);
    glyph.send_keys("ctrl+p");
    glyph.type_text("notes.txt");
    glyph.wait_for_text("✦ notes.txt", WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text("notes for the tree test", WAIT);
    glyph.wait_for_text(&pill_text(&["notes.txt"], 0), WAIT);
    let screen = glyph.screen().join("\n");
    assert!(!screen.contains("Open directory"), "{screen}");
}

#[test]
fn with_the_tree_open_the_splash_has_focus_and_ctrl_e_moves_it_to_the_tree() {
    let mut glyph = Glyph::spawn(&[PROJECT]);
    glyph.wait_for_text("README.md", START);
    // Beside the 28-column tree and its blank column the editor area is 71
    // wide from column 29, so the card starts at 38 and its text at 42.
    let text_x = 42;
    wait_for_selected(&glyph, text_x, NEW_FILE_ROW);
    // ↓ moves the splash's selection, not the tree's.
    glyph.send_keys("down");
    wait_for_selected(&glyph, text_x, NEW_DIR_ROW);

    glyph.send_keys("ctrl+e");
    // In the tree ↓ moves from `docs` past `src` and `.gitignore` to
    // `notes.txt`; Enter opens it in place of the splash.
    for _ in 0..3 {
        glyph.send_keys("down");
    }
    glyph.send_keys("enter");
    glyph.wait_for_text("notes for the tree test", WAIT);
    glyph.wait_for_text_gone("Open directory", WAIT);
}

#[test]
fn new_file_creates_in_the_project_folder_and_opens_it() {
    let parent = tempfile::tempdir().expect("create temp dir");
    let root = parent.path().join("proj");
    fs::create_dir(&root).expect("create proj");
    fs::write(root.join("a.txt"), "alpha\n").expect("write a.txt");
    // cwd is the parent, so the project folder isn't where Glyph runs.
    let mut glyph = Glyph::spawn_in(parent.path(), &["proj"]);
    glyph.wait_for_text("Open directory", START);

    // Esc in the name prompt comes back to the splash, which takes keys again.
    glyph.type_text("n");
    glyph.wait_for_text("⏎ create   esc cancel", WAIT);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("⏎ create   esc cancel", WAIT);
    glyph.wait_for_text("Open directory", WAIT);

    // A name that's taken, or not a plain name, is refused as in the tree.
    glyph.send_keys("enter");
    glyph.wait_for_text("⏎ create   esc cancel", WAIT);
    glyph.type_text("a.txt");
    glyph.send_keys("enter");
    wait_for_status(&glyph, "a.txt already exists");
    glyph.wait_for_text("Open directory", WAIT);
    glyph.type_text("n");
    glyph.wait_for_text("⏎ create   esc cancel", WAIT);
    glyph.type_text("sub/b.txt");
    glyph.send_keys("enter");
    wait_for_status(&glyph, "invalid name");
    glyph.wait_for_text("Open directory", WAIT);
    assert_eq!(fs::read_dir(&root).expect("list proj").count(), 1);

    glyph.type_text("n");
    glyph.wait_for_text("⏎ create   esc cancel", WAIT);
    glyph.type_text("b.txt");
    glyph.send_keys("enter");
    wait_for_status(&glyph, "created b.txt");
    glyph.wait_for_files("b.txt in the project", WAIT, || {
        root.join("b.txt").is_file()
    });
    assert!(!parent.path().join("b.txt").exists());
    glyph.wait_for_text_gone("Open directory", WAIT);
    glyph.wait_for_text(&pill_text(&["b.txt"], 0), WAIT);
    glyph.type_text("hi");
    glyph.wait_for_text("1  hi", WAIT);
}

/// The last part of `dir`'s path, which the brand row ends with.
fn name(dir: &Path) -> String {
    dir.file_name()
        .expect("temp folders have names")
        .to_string_lossy()
        .into_owned()
}

/// Waits for the tree's brand row to show the end of `dir`'s name (a long
/// name is cut at its start).
fn wait_for_brand(glyph: &Glyph, dir: &Path) {
    let name = name(dir);
    let tail: String = name
        .chars()
        .skip(name.chars().count().saturating_sub(6))
        .collect();
    glyph.wait_for_screen(&format!("brand ending {tail:?}"), WAIT, |screen| {
        screen[1]
            .chars()
            .take(28)
            .collect::<String>()
            .contains(&tail)
    });
}

#[test]
fn new_directory_creates_the_folder_and_opens_it_as_the_project() {
    let parent = tempfile::tempdir().expect("create temp dir");
    let root = parent.path().join("proj");
    fs::create_dir(&root).expect("create proj");
    fs::write(root.join("a.txt"), "alpha\n").expect("write a.txt");
    // cwd is the parent, so the project folder isn't where Glyph runs.
    let mut glyph = Glyph::spawn_in(parent.path(), &["proj"]);
    glyph.wait_for_text("Open directory", START);
    glyph.wait_for_text("a.txt", WAIT);

    // A taken name is refused as in the tree, and nothing switches.
    glyph.type_text("d");
    glyph.wait_for_text(" New folder  ", WAIT);
    glyph.type_text("a.txt");
    glyph.send_keys("enter");
    wait_for_status(&glyph, "a.txt already exists");
    glyph.wait_for_text("Open directory", WAIT);
    glyph.wait_for_text("a.txt", WAIT);

    glyph.type_text("d");
    glyph.wait_for_text(" New folder  ", WAIT);
    glyph.type_text("fresh-dir");
    glyph.send_keys("enter");
    wait_for_status(&glyph, "created fresh-dir");
    let fresh = root.join("fresh-dir");
    glyph.wait_for_files("fresh-dir in the project", WAIT, || fresh.is_dir());
    assert!(!parent.path().join("fresh-dir").exists());

    // The tree and brand show the new, empty folder beside an empty untitled
    // pane; the splash is gone.
    wait_for_brand(&glyph, &fresh);
    glyph.wait_for_screen("the old folder's files gone", WAIT, |screen| {
        !screen.iter().any(|line| line.contains("a.txt"))
    });
    glyph.wait_for_text_gone("Open directory", WAIT);
    glyph.wait_for_text(&pill_text(&["untitled"], 0), WAIT);

    // The tree has the keys: its `a` makes a file in the new folder.
    glyph.type_text("a");
    glyph.wait_for_text("⏎ create   esc cancel", WAIT);
    glyph.type_text("b.txt");
    glyph.send_keys("enter");
    glyph.wait_for_files("b.txt in the new folder", WAIT, || {
        fresh.join("b.txt").is_file()
    });
}

#[test]
fn open_directory_switches_and_lands_on_the_tree_and_an_empty_pane() {
    let dir = tempfile::tempdir().expect("create temp dir");
    fs::write(dir.path().join("a.txt"), "alpha\n").expect("write a.txt");
    let other = dir.path().join("other");
    fs::create_dir(&other).expect("create other");
    fs::write(other.join("inside.txt"), "").expect("write inside.txt");
    let mut glyph = Glyph::spawn_in(dir.path(), &["."]);
    glyph.wait_for_text("Open directory", START);

    // `o`, ↓ to `other`, Enter into it, Enter on its open row.
    glyph.type_text("o");
    glyph.wait_for_text(BROWSER, WAIT);
    // The browser's header is row 6 and its folders start on row 9 (the
    // tree beside it lists `other` too, so the rows are named).
    glyph.wait_for_screen("other in the browser", WAIT, |screen| {
        screen[9].contains("▸ other")
    });
    glyph.send_keys("down");
    glyph.send_keys("enter");
    glyph.wait_for_screen("inside other", WAIT, |screen| screen[6].contains("other"));
    glyph.send_keys("enter");
    glyph.wait_for_text_gone(BROWSER, WAIT);

    // The tree and brand show `other` beside an empty untitled pane, and the
    // tree takes the keys: Enter opens the file it selects.
    glyph.wait_for_text("inside.txt", WAIT);
    wait_for_brand(&glyph, &other);
    glyph.wait_for_text_gone("Open directory", WAIT);
    glyph.wait_for_text(&pill_text(&["untitled"], 0), WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text(&pill_text(&["inside.txt"], 0), WAIT);
}
