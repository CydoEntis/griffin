mod harness;

use std::fs;
use std::time::Duration;

use harness::{Griffin, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const PROJECT: &str = "tests/fixtures/project";
/// The tree's width; the divider sits in the next column.
const TREE: usize = 30;

/// What the tree pane shows on `line`: its first 30 columns, trailing spaces cut.
fn tree_text(line: &str) -> String {
    line.chars()
        .take(TREE)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The tree pane's rows from the top (row 1, below the tab bar's row), up to the
/// first empty one.
fn tree_rows(screen: &[String]) -> Vec<String> {
    screen
        .iter()
        .skip(1)
        .map(|line| tree_text(line))
        .take_while(|text| !text.is_empty())
        .collect()
}

fn wait_for_tree(griffin: &Griffin, expected: &[&str]) {
    griffin.wait_for_screen(&format!("tree rows {expected:?}"), WAIT, |screen| {
        tree_rows(screen) == expected
    });
}

/// The tree's selected row as the harness reads it: the reversed cells on `row`,
/// which span the whole pane.
fn wait_for_selected(griffin: &Griffin, row: u16, text: &str) {
    griffin.wait_for_reversed(row, &format!("{text:<TREE$}"), WAIT);
}

fn open_project() -> Griffin {
    let griffin = Griffin::spawn(&[PROJECT]);
    griffin.wait_for_text("README.md", START);
    griffin
}

const TOP: &[&str] = &[
    "▸ docs",
    "▸ src",
    "  .gitignore",
    "  notes.txt",
    "  README.md",
];

#[test]
fn folder_opens_with_a_30_column_tree_folders_first() {
    let griffin = open_project();
    wait_for_tree(&griffin, TOP);
    // The first row starts selected, highlighted across the pane.
    wait_for_selected(&griffin, 1, "▸ docs");

    let screen = griffin.screen();
    for (y, line) in screen.iter().enumerate().take(usize::from(ROWS - 1)) {
        assert_eq!(
            line.chars().nth(TREE),
            Some('│'),
            "row {y} should have the divider in column 30: {line:?}"
        );
    }
    // The empty editor starts right of the divider.
    assert!(
        screen[1]
            .chars()
            .skip(TREE + 1)
            .collect::<String>()
            .starts_with(" 1 │"),
        "{:?}",
        screen[1]
    );
}

#[test]
fn gitignored_files_are_hidden() {
    // The fixture's `.gitignore` names `*.log` and `build/`, both present on disk.
    assert!(fs::metadata(format!("{PROJECT}/debug.log")).is_ok());
    assert!(fs::metadata(format!("{PROJECT}/build/out.txt")).is_ok());
    let griffin = open_project();
    wait_for_tree(&griffin, TOP);
    let contents = griffin.screen().join("\n");
    assert!(!contents.contains("debug.log"), "{contents}");
    assert!(!contents.contains("build"), "{contents}");
}

#[test]
fn dot_git_and_nested_ignores_are_hidden() {
    let dir = tempfile::tempdir().expect("create temp dir");
    let root = dir.path();
    fs::create_dir(root.join(".git")).expect("create .git");
    fs::write(root.join(".git").join("HEAD"), "ref: refs/heads/main\n").expect("write HEAD");
    fs::write(root.join(".gitignore"), "secret.txt\n").expect("write .gitignore");
    fs::write(root.join("secret.txt"), "").expect("write secret");
    fs::write(root.join("visible.txt"), "").expect("write visible");
    fs::create_dir(root.join("lib")).expect("create lib");
    fs::write(root.join("lib").join("secret.txt"), "").expect("write nested secret");
    fs::write(root.join("lib").join("mod.rs"), "").expect("write mod.rs");

    let path = root.to_str().expect("temp path is UTF-8");
    let mut griffin = Griffin::spawn(&[path]);
    griffin.wait_for_text("visible.txt", START);
    wait_for_tree(&griffin, &["▸ lib", "  .gitignore", "  visible.txt"]);
    griffin.send_keys("enter");
    wait_for_tree(
        &griffin,
        &["▾ lib", "    mod.rs", "  .gitignore", "  visible.txt"],
    );
}

#[test]
fn arrows_browse_and_enter_opens_a_file() {
    let mut griffin = open_project();
    wait_for_selected(&griffin, 1, "▸ docs");

    griffin.send_keys("down");
    wait_for_selected(&griffin, 2, "▸ src");
    griffin.send_keys("right");
    wait_for_tree(
        &griffin,
        &[
            "▸ docs",
            "▾ src",
            "  ▸ util",
            "    main.rs",
            "  .gitignore",
            "  notes.txt",
            "  README.md",
        ],
    );
    // ← collapses, Enter expands again.
    griffin.send_keys("left");
    wait_for_tree(&griffin, TOP);
    griffin.send_keys("enter");
    wait_for_tree(
        &griffin,
        &[
            "▸ docs",
            "▾ src",
            "  ▸ util",
            "    main.rs",
            "  .gitignore",
            "  notes.txt",
            "  README.md",
        ],
    );
    griffin.send_keys("down");
    wait_for_selected(&griffin, 3, "  ▸ util");
    griffin.send_keys("down");
    wait_for_selected(&griffin, 4, "    main.rs");
    griffin.send_keys("up");
    wait_for_selected(&griffin, 3, "  ▸ util");
    griffin.send_keys("down");
    wait_for_selected(&griffin, 4, "    main.rs");

    griffin.send_keys("enter");
    griffin.wait_for_text("hello from main", WAIT);
    let status = griffin.screen()[usize::from(ROWS - 1)].clone();
    assert!(status.contains("main.rs"), "{status:?}");
    // The editor has focus now: arrows move its cursor, not the tree selection.
    griffin.send_keys("down");
    griffin.wait_for_text("Ln 2, Col 1", WAIT);
    wait_for_selected(&griffin, 4, "    main.rs");
}

#[test]
fn clicking_selects_rows_and_opens_files() {
    let mut griffin = open_project();
    wait_for_tree(&griffin, TOP);

    // A file row: selected and opened.
    griffin.click(5, 4);
    griffin.wait_for_text("notes for the tree test", WAIT);
    wait_for_selected(&griffin, 4, "  notes.txt");

    // A folder row: selected and expanded.
    griffin.click(3, 2);
    wait_for_tree(
        &griffin,
        &[
            "▸ docs",
            "▾ src",
            "  ▸ util",
            "    main.rs",
            "  .gitignore",
            "  notes.txt",
            "  README.md",
        ],
    );
    wait_for_selected(&griffin, 2, "▾ src");

    griffin.click(10, 4);
    griffin.wait_for_text("hello from main", WAIT);
    wait_for_selected(&griffin, 4, "    main.rs");

    // Clicking the editor gives it focus back: typing edits the file.
    let x = griffin
        .text_col(1, "fn main")
        .expect("main.rs is showing on row 0");
    griffin.click(x, 1);
    griffin.type_text("x");
    griffin.wait_for_text("xfn main", WAIT);
}

#[test]
fn opening_another_file_keeps_unsaved_changes_in_their_tab() {
    let notes_path = format!("{PROJECT}/notes.txt");
    let before = fs::read(&notes_path).expect("read notes");
    let mut griffin = open_project();
    wait_for_tree(&griffin, TOP);
    griffin.click(5, 4);
    griffin.wait_for_text("notes for the tree test", WAIT);
    griffin.type_text("x");
    griffin.wait_for_text("xnotes for the tree test", WAIT);

    // The other file opens in a tab of its own, without asking.
    griffin.click(5, 5);
    griffin.wait_for_text("# Project fixture", WAIT);
    griffin.wait_for_text(" notes.txt ●  README.md ", WAIT);
    assert!(
        !griffin
            .screen()
            .join(
                "
"
            )
            .contains("Unsaved changes")
    );
    // Back on the first tab, the edit is still there and still unsaved.
    griffin.click(5, 4);
    griffin.wait_for_text("xnotes for the tree test", WAIT);
    assert_eq!(fs::read(&notes_path).expect("read notes"), before);
}

#[test]
fn ctrl_b_toggles_the_tree_and_ctrl_e_switches_focus() {
    let mut griffin = open_project();
    wait_for_tree(&griffin, TOP);
    griffin.click(5, 5);
    griffin.wait_for_text("# Project fixture", WAIT);
    let editor_x = griffin
        .text_col(1, "# Project")
        .expect("README.md is showing on row 0");
    assert!(usize::from(editor_x) > TREE, "editor at column {editor_x}");

    // Ctrl+B hides the tree: the editor starts at column 0.
    griffin.send_keys("ctrl+b");
    griffin.wait_for_screen("the editor at column 0", WAIT, |screen| {
        screen[1].starts_with(" 1 │ # Project fixture")
    });
    griffin.send_keys("ctrl+b");
    wait_for_tree(&griffin, TOP);

    // Ctrl+E moves focus to the tree: the cursor sits on the selected row and
    // arrows move the selection.
    griffin.send_keys("ctrl+e");
    griffin.wait_for_cursor(0, 5, WAIT);
    griffin.send_keys("up");
    wait_for_selected(&griffin, 4, "  notes.txt");
    // And back to the editor, where typing edits.
    griffin.send_keys("ctrl+e");
    griffin.type_text("y");
    griffin.wait_for_text("y# Project fixture", WAIT);

    // With the tree hidden, Ctrl+E brings it back focused.
    griffin.send_keys("ctrl+b");
    griffin.wait_for_screen("the editor at column 0", WAIT, |screen| {
        screen[1].starts_with(" 1 │ y# Project fixture")
    });
    griffin.send_keys("ctrl+e");
    wait_for_tree(&griffin, TOP);
    griffin.wait_for_cursor(0, 4, WAIT);
}

#[test]
fn tree_keys_are_remappable_by_name() {
    let toml = "[keys]\ntoggle_tree = \"alt+b\"\nfocus_tree = \"alt+e\"\n";
    let mut griffin = Griffin::spawn_with_config(toml, &[PROJECT]);
    griffin.wait_for_text("README.md", START);
    griffin.send_keys("alt+b");
    griffin.wait_for_text_gone("README.md", WAIT);
    griffin.send_keys("alt+e");
    wait_for_tree(&griffin, TOP);
    griffin.send_keys("ctrl+b");
    // Ctrl+B is no longer bound: the tree stays.
    griffin.assert_running_for(Duration::from_millis(300));
    wait_for_tree(&griffin, TOP);
}
