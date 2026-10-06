//! Creating, renaming and refusing names from the file tree, on a temp copy of the
//! project fixture so the real one never changes. Delete goes to the OS trash, so
//! it's covered by unit tests with a fake trash instead.

mod harness;

use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{Griffin, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const TREE: usize = 30;
/// The prompt bar's row: just above the status line.
const BAR: usize = ROWS as usize - 2;
const STATUS: usize = ROWS as usize - 1;

const TOP: &[&str] = &[
    "▸ docs",
    "▸ src",
    "  .gitignore",
    "  notes.txt",
    "  README.md",
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

/// A temp copy of `tests/fixtures/project` with griffin open on it, cwd inside it
/// so paths stay short.
fn open_copy() -> (tempfile::TempDir, Griffin) {
    let dir = tempfile::tempdir().expect("create temp dir");
    copy_dir(Path::new("tests/fixtures/project"), dir.path());
    let griffin = Griffin::spawn_in(dir.path(), &["."]);
    griffin.wait_for_text("README.md", START);
    wait_for_tree(&griffin, TOP);
    (dir, griffin)
}

fn tree_rows(screen: &[String]) -> Vec<String> {
    screen
        .iter()
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

fn wait_for_tree(griffin: &Griffin, expected: &[&str]) {
    griffin.wait_for_screen(&format!("tree rows {expected:?}"), WAIT, |screen| {
        tree_rows(screen) == expected
    });
}

fn wait_for_selected(griffin: &Griffin, row: u16, text: &str) {
    griffin.wait_for_reversed(row, &format!("{text:<TREE$}"), WAIT);
}

fn wait_for_bar(griffin: &Griffin, text: &str) {
    griffin.wait_for_screen(&format!("prompt bar {text:?}"), WAIT, |screen| {
        screen[BAR].trim_end() == text
    });
}

fn wait_for_status(griffin: &Griffin, text: &str) {
    griffin.wait_for_screen(&format!("status with {text:?}"), WAIT, |screen| {
        screen[STATUS].contains(text)
    });
}

#[test]
fn a_creates_a_file_beside_the_selected_file_and_opens_it() {
    let (dir, mut griffin) = open_copy();
    for _ in 0..4 {
        griffin.send_keys("down");
    }
    wait_for_selected(&griffin, 4, "  README.md");

    griffin.send_keys("a");
    wait_for_bar(&griffin, "New file:");
    griffin.wait_for_cursor(10, BAR as u16, WAIT);
    griffin.type_text("notes.md");
    wait_for_bar(&griffin, "New file: notes.md");
    griffin.send_keys("enter");

    wait_for_tree(
        &griffin,
        &[
            "▸ docs",
            "▸ src",
            "  .gitignore",
            "  notes.md",
            "  notes.txt",
            "  README.md",
        ],
    );
    wait_for_selected(&griffin, 3, "  notes.md");
    // The bar is gone and the editor shows the new, empty file.
    griffin.wait_for_screen("the bar closed", WAIT, |screen| {
        !screen[BAR].contains("New file")
    });
    wait_for_status(&griffin, "notes.md  Ln 1, Col 1");
    let screen = griffin.screen();
    let editor: String = screen[0].chars().skip(TREE + 1).collect();
    assert_eq!(editor.trim_end(), " 1 │", "{screen:#?}");
    assert_eq!(
        fs::read(dir.path().join("notes.md")).expect("read new file"),
        b""
    );

    // Typing goes into the new file.
    griffin.type_text("hi");
    griffin.send_keys("ctrl+s");
    griffin.wait_for_files("notes.md saved", WAIT, || {
        fs::read_to_string(dir.path().join("notes.md")).is_ok_and(|text| text == "hi")
    });
}

#[test]
fn a_and_shift_a_create_inside_the_selected_folder() {
    let (dir, mut griffin) = open_copy();
    wait_for_selected(&griffin, 0, "▸ docs");

    griffin.send_keys("a");
    wait_for_bar(&griffin, "New file:");
    griffin.type_text("new.md");
    griffin.send_keys("enter");
    // The folder opens to show the new file, which is selected.
    wait_for_tree(
        &griffin,
        &[
            "▾ docs",
            "    guide.md",
            "    new.md",
            "▸ src",
            "  .gitignore",
            "  notes.txt",
            "  README.md",
        ],
    );
    wait_for_selected(&griffin, 2, "    new.md");
    assert!(dir.path().join("docs").join("new.md").is_file());

    // Back in the tree, Shift+A makes a folder next to the selected file.
    griffin.send_keys("ctrl+e");
    griffin.send_keys("shift+a");
    wait_for_bar(&griffin, "New folder:");
    griffin.type_text("drafts");
    griffin.send_keys("enter");
    wait_for_tree(
        &griffin,
        &[
            "▾ docs",
            "  ▸ drafts",
            "    guide.md",
            "    new.md",
            "▸ src",
            "  .gitignore",
            "  notes.txt",
            "  README.md",
        ],
    );
    wait_for_selected(&griffin, 1, "  ▸ drafts");
    assert!(dir.path().join("docs").join("drafts").is_dir());
}

#[test]
fn r_renames_and_the_open_buffer_follows() {
    let (dir, mut griffin) = open_copy();
    for _ in 0..3 {
        griffin.send_keys("down");
    }
    griffin.send_keys("enter");
    griffin.wait_for_text("notes for the tree test", WAIT);

    griffin.send_keys("ctrl+e");
    griffin.send_keys("r");
    wait_for_bar(&griffin, "Rename: notes.txt");
    for _ in 0..3 {
        griffin.send_keys("backspace");
    }
    griffin.type_text("md");
    wait_for_bar(&griffin, "Rename: notes.md");
    griffin.send_keys("enter");

    wait_for_tree(
        &griffin,
        &[
            "▸ docs",
            "▸ src",
            "  .gitignore",
            "  notes.md",
            "  README.md",
        ],
    );
    wait_for_selected(&griffin, 3, "  notes.md");
    wait_for_status(&griffin, "renamed to notes.md");
    assert!(!dir.path().join("notes.txt").exists());

    // The buffer now belongs to notes.md: saving writes there.
    griffin.send_keys("ctrl+e");
    griffin.type_text("x");
    griffin.send_keys("ctrl+s");
    griffin.wait_for_files("notes.md saved", WAIT, || {
        fs::read_to_string(dir.path().join("notes.md"))
            .is_ok_and(|text| text.starts_with("xnotes for the tree test"))
    });
    assert!(!dir.path().join("notes.txt").exists());
}

#[test]
fn an_existing_or_invalid_name_changes_nothing() {
    let (dir, mut griffin) = open_copy();
    let notes = fs::read(dir.path().join("notes.txt")).expect("read notes");
    for _ in 0..4 {
        griffin.send_keys("down");
    }
    wait_for_selected(&griffin, 4, "  README.md");

    griffin.send_keys("a");
    wait_for_bar(&griffin, "New file:");
    griffin.type_text("notes.txt");
    griffin.send_keys("enter");
    wait_for_status(&griffin, "notes.txt already exists");
    wait_for_tree(&griffin, TOP);
    wait_for_selected(&griffin, 4, "  README.md");
    assert_eq!(
        fs::read(dir.path().join("notes.txt")).expect("read notes"),
        notes
    );

    griffin.send_keys("a");
    wait_for_bar(&griffin, "New file:");
    griffin.type_text("bad/name");
    griffin.send_keys("enter");
    wait_for_status(&griffin, "invalid name");
    wait_for_tree(&griffin, TOP);

    // Renaming onto another entry is refused too.
    griffin.send_keys("r");
    wait_for_bar(&griffin, "Rename: README.md");
    for _ in 0.."README.md".len() {
        griffin.send_keys("backspace");
    }
    griffin.type_text("notes.txt");
    griffin.send_keys("enter");
    wait_for_status(&griffin, "notes.txt already exists");
    wait_for_tree(&griffin, TOP);
    assert_eq!(
        fs::read(dir.path().join("notes.txt")).expect("read notes"),
        notes
    );
    assert!(dir.path().join("README.md").exists());

    // Esc closes the bar without doing anything.
    griffin.send_keys("a");
    wait_for_bar(&griffin, "New file:");
    griffin.type_text("never.md");
    griffin.send_keys("esc");
    griffin.wait_for_screen("the bar closed", WAIT, |screen| {
        !screen[BAR].contains("New file")
    });
    assert!(!dir.path().join("never.md").exists());
}
