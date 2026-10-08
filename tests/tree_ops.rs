//! Creating, renaming and refusing names from the file tree, on a temp copy of the
//! project fixture so the real one never changes. Delete goes to the OS trash, so
//! it's covered by unit tests with a fake trash instead.

mod harness;

use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{Glyph, ROWS};
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const TREE: usize = 28;
/// The row of the first node, below the tree's brand row.
const FIRST: usize = 3;
/// The prompt bar's row: just above the status line.
const BAR: usize = ROWS as usize - 2;
const STATUS: usize = ROWS as usize - 1;

const TOP: &[&str] = &[
    "  ▸ docs",
    "  ▸ src",
    "    .gitignore",
    "    notes.txt",
    "    README.md",
];

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create fixture copy");
    for entry in fs::read_dir(from).expect("read fixture") {
        let entry = entry.expect("read fixture entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy fixture file");
        }
    }
}

/// A temp copy of `tests/fixtures/project` with glyph open on it, cwd inside it
/// so paths stay short.
fn open_copy() -> (tempfile::TempDir, Glyph) {
    let dir = tempfile::tempdir().expect("create temp dir");
    copy_dir(Path::new("tests/fixtures/project"), dir.path());
    let glyph = Glyph::spawn_in(dir.path(), &["."]);
    glyph.wait_for_text("README.md", START);
    wait_for_tree(&glyph, TOP);
    (dir, glyph)
}

/// The tree pane's node rows from the first one down to the first empty one.
fn tree_rows(screen: &[String]) -> Vec<String> {
    screen
        .iter()
        .skip(FIRST)
        .map(|line| {
            line.chars()
                .take(TREE)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .take_while(|text| !text.is_empty())
        .collect()
}

fn wait_for_tree(glyph: &Glyph, expected: &[&str]) {
    glyph.wait_for_screen(&format!("tree rows {expected:?}"), WAIT, |screen| {
        tree_rows(screen) == expected
    });
}

/// Waits for `row` to read `text` and glow across the pane, as the focused
/// selection and the open file do: hydra's `mix(surface, accent, .26)` at column
/// 0 fading to `surface` at 27.
fn wait_for_selected(glyph: &Glyph, row: u16, text: &str) {
    glyph.wait_for_screen(&format!("tree row {row} {text:?}"), WAIT, |screen| {
        let line: String = screen[usize::from(row)].chars().take(TREE).collect();
        line.trim_end() == text
    });
    glyph.wait_for_bg(0, row, Color::Rgb(0x3c, 0x4e, 0x24), WAIT);
    glyph.wait_for_bg(27, row, Color::Rgb(0x0d, 0x14, 0x1b), WAIT);
}

fn wait_for_bar(glyph: &Glyph, text: &str) {
    glyph.wait_for_screen(&format!("prompt bar {text:?}"), WAIT, |screen| {
        screen[BAR].trim_end() == text
    });
}

fn wait_for_status(glyph: &Glyph, text: &str) {
    glyph.wait_for_screen(&format!("status with {text:?}"), WAIT, |screen| {
        screen[STATUS].contains(text)
    });
}

#[test]
fn a_creates_a_file_beside_the_selected_file_and_opens_it() {
    let (dir, mut glyph) = open_copy();
    for _ in 0..4 {
        glyph.send_keys("down");
    }
    wait_for_selected(&glyph, 7, "    README.md");

    glyph.send_keys("a");
    wait_for_bar(&glyph, "New file:");
    glyph.wait_for_cursor(10, BAR as u16, WAIT);
    glyph.type_text("notes.md");
    wait_for_bar(&glyph, "New file: notes.md");
    glyph.send_keys("enter");

    wait_for_tree(
        &glyph,
        &[
            "  ▸ docs",
            "  ▸ src",
            "    .gitignore",
            "    notes.md",
            "    notes.txt",
            "    README.md",
        ],
    );
    wait_for_selected(&glyph, 6, "    notes.md");
    // The bar is gone and the editor shows the new, empty file.
    glyph.wait_for_screen("the bar closed", WAIT, |screen| {
        !screen[BAR].contains("New file")
    });
    // The message holds the path slot until the next key.
    wait_for_status(&glyph, "✓ created notes.md");
    wait_for_status(&glyph, "Ln 1, Col 1");
    let screen = glyph.screen();
    let editor: String = screen[3].chars().skip(TREE + 1).collect();
    assert_eq!(editor.trim_end(), " 1 │", "{screen:#?}");
    assert_eq!(
        fs::read(dir.path().join("notes.md")).expect("read new file"),
        b""
    );

    // Typing goes into the new file.
    glyph.type_text("hi");
    glyph.send_keys("ctrl+s");
    glyph.wait_for_files("notes.md saved", WAIT, || {
        fs::read_to_string(dir.path().join("notes.md")).is_ok_and(|text| text == "hi")
    });
}

#[test]
fn a_and_shift_a_create_inside_the_selected_folder() {
    let (dir, mut glyph) = open_copy();
    wait_for_selected(&glyph, 3, "  ▸ docs");

    glyph.send_keys("a");
    wait_for_bar(&glyph, "New file:");
    glyph.type_text("new.md");
    glyph.send_keys("enter");
    // The folder opens to show the new file, which is selected.
    wait_for_tree(
        &glyph,
        &[
            "  ▾ docs",
            "   │  guide.md",
            "   │  new.md",
            "  ▸ src",
            "    .gitignore",
            "    notes.txt",
            "    README.md",
        ],
    );
    wait_for_selected(&glyph, 5, "   │  new.md");
    assert!(dir.path().join("docs").join("new.md").is_file());

    // Back in the tree, Shift+A makes a folder next to the selected file.
    glyph.send_keys("ctrl+e");
    glyph.send_keys("shift+a");
    wait_for_bar(&glyph, "New folder:");
    glyph.type_text("drafts");
    glyph.send_keys("enter");
    wait_for_tree(
        &glyph,
        &[
            "  ▾ docs",
            "   │▸ drafts",
            "   │  guide.md",
            "   │  new.md",
            "  ▸ src",
            "    .gitignore",
            "    notes.txt",
            "    README.md",
        ],
    );
    wait_for_selected(&glyph, 4, "   │▸ drafts");
    assert!(dir.path().join("docs").join("drafts").is_dir());
}

#[test]
fn r_renames_and_the_open_buffer_follows() {
    let (dir, mut glyph) = open_copy();
    for _ in 0..3 {
        glyph.send_keys("down");
    }
    glyph.send_keys("enter");
    glyph.wait_for_text("notes for the tree test", WAIT);

    glyph.send_keys("ctrl+e");
    glyph.send_keys("r");
    wait_for_bar(&glyph, "Rename: notes.txt");
    for _ in 0..3 {
        glyph.send_keys("backspace");
    }
    glyph.type_text("md");
    wait_for_bar(&glyph, "Rename: notes.md");
    glyph.send_keys("enter");

    wait_for_tree(
        &glyph,
        &[
            "  ▸ docs",
            "  ▸ src",
            "    .gitignore",
            "    notes.md",
            "    README.md",
        ],
    );
    wait_for_selected(&glyph, 6, "    notes.md");
    wait_for_status(&glyph, "renamed to notes.md");
    assert!(!dir.path().join("notes.txt").exists());

    // The buffer now belongs to notes.md: saving writes there.
    glyph.send_keys("ctrl+e");
    glyph.type_text("x");
    glyph.send_keys("ctrl+s");
    glyph.wait_for_files("notes.md saved", WAIT, || {
        fs::read_to_string(dir.path().join("notes.md"))
            .is_ok_and(|text| text.starts_with("xnotes for the tree test"))
    });
    assert!(!dir.path().join("notes.txt").exists());
}

#[test]
fn an_existing_or_invalid_name_changes_nothing() {
    let (dir, mut glyph) = open_copy();
    let notes = fs::read(dir.path().join("notes.txt")).expect("read notes");
    for _ in 0..4 {
        glyph.send_keys("down");
    }
    wait_for_selected(&glyph, 7, "    README.md");

    glyph.send_keys("a");
    wait_for_bar(&glyph, "New file:");
    glyph.type_text("notes.txt");
    glyph.send_keys("enter");
    wait_for_status(&glyph, "notes.txt already exists");
    wait_for_tree(&glyph, TOP);
    wait_for_selected(&glyph, 7, "    README.md");
    assert_eq!(
        fs::read(dir.path().join("notes.txt")).expect("read notes"),
        notes
    );

    glyph.send_keys("a");
    wait_for_bar(&glyph, "New file:");
    glyph.type_text("bad/name");
    glyph.send_keys("enter");
    wait_for_status(&glyph, "invalid name");
    wait_for_tree(&glyph, TOP);

    // Renaming onto another entry is refused too.
    glyph.send_keys("r");
    wait_for_bar(&glyph, "Rename: README.md");
    for _ in 0.."README.md".len() {
        glyph.send_keys("backspace");
    }
    glyph.type_text("notes.txt");
    glyph.send_keys("enter");
    wait_for_status(&glyph, "notes.txt already exists");
    wait_for_tree(&glyph, TOP);
    assert_eq!(
        fs::read(dir.path().join("notes.txt")).expect("read notes"),
        notes
    );
    assert!(dir.path().join("README.md").exists());

    // Esc closes the bar without doing anything.
    glyph.send_keys("a");
    wait_for_bar(&glyph, "New file:");
    glyph.type_text("never.md");
    glyph.send_keys("esc");
    glyph.wait_for_screen("the bar closed", WAIT, |screen| {
        !screen[BAR].contains("New file")
    });
    assert!(!dir.path().join("never.md").exists());
}
