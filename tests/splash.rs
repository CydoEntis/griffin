//! The splash Tome opens on with nothing to edit (tome-splash spec S1–S5,
//! S7): when it shows, how it's drawn, its keys and clicks, *New file*, *New
//! directory* and *Open directory*, its status bar, `q`, the tree it hides, and
//! that a project switch from it lands on the tree beside an empty pane rather
//! than on a new splash.

mod harness;

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{COLS, ROWS, Tome, pill_text};
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

/// Without the tree the editor area is the full width from row 3 (below the
/// tab header) to the status line: 100 × 26. The 13-row splash is centred in
/// it: the wordmark on rows 9–12, the rule on 13, the path on 15 and the
/// actions on 17, 19 and 21.
const WORD_ROW: u16 = 9;
const RULE_ROW: u16 = 13;
const PATH_ROW: u16 = 15;
const NEW_FILE_ROW: u16 = 17;
const NEW_DIR_ROW: u16 = 19;
const OPEN_DIR_ROW: u16 = 21;
/// The 24-wide wordmark and 63-wide rule, centred.
const WORD_W: u16 = 24;
const WORD_X: u16 = 38;
const RULE_X: u16 = 18;
/// The 56-wide action rows, centred: `✦` two cells in, the label four, the
/// hint 22, and the key two short of the row's right edge.
const ROW_X: u16 = 22;
const TEXT_X: u16 = ROW_X + 2;
const LABEL_X: u16 = ROW_X + 4;
const HINT_X: u16 = ROW_X + 22;
const KEY_X: u16 = ROW_X + 56 - 2;
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
fn cells(tome: &Tome, row: u16, x: u16, width: u16) -> String {
    tome.screen()[usize::from(row)]
        .chars()
        .skip(usize::from(x))
        .take(usize::from(width))
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// Waits for the `✦` marking the selected action to be on `row`.
fn wait_for_selected(tome: &Tome, x: u16, row: u16) {
    tome.wait_for_screen(&format!("✦ on row {row}"), WAIT, |screen| {
        screen[usize::from(row)].chars().nth(usize::from(x)) == Some('✦')
    });
}

fn wait_for_status(tome: &Tome, text: &str) {
    tome.wait_for_screen(&format!("status {text:?}"), WAIT, |screen| {
        screen[usize::from(STATUS)].contains(text)
    });
}

/// A temp folder standing in for the home folder, holding `src` with `a.txt`
/// in it, and Tome started in `src` with no argument, so the splash names
/// the project `~/src` as in the design.
fn start_bare_with(toml: &str) -> (TempDir, Tome) {
    let home = tempfile::tempdir().expect("create temp dir");
    let src = home.path().join("src");
    fs::create_dir(&src).expect("create src");
    fs::write(src.join("a.txt"), "alpha\n").expect("write a.txt");
    // `HOME` on Unix and `USERPROFILE` on Windows are where Tome reads the
    // home folder from.
    let env = [
        ("HOME", OsString::from(home.path())),
        ("USERPROFILE", OsString::from(home.path())),
    ];
    let tome = Tome::spawn_in_with_config_and_env(&src, toml, &env, &[]);
    tome.wait_for_text("Open directory", START);
    (home, tome)
}

fn start_bare() -> (TempDir, Tome) {
    start_bare_with("")
}

#[test]
fn tome_alone_and_tome_on_a_folder_show_the_splash_but_a_file_does_not() {
    let (_dir, _tome) = start_bare();

    let tome = Tome::spawn(&[PROJECT]);
    tome.wait_for_text("Open directory", START);

    let tome = Tome::spawn(&["tests/fixtures/project/notes.txt"]);
    tome.wait_for_text("notes for the tree test", START);
    let screen = tome.screen().join("\n");
    assert!(!screen.contains("Open directory"), "{screen}");
}

#[test]
fn the_splash_has_the_wordmark_rule_path_and_actions_with_no_card() {
    let (_home, tome) = start_bare();
    tome.wait_for_fg_at(LABEL_X, NEW_FILE_ROW, STRONG, WAIT);

    // No pills or thread in the header rows.
    let screen = tome.screen();
    for row in 0..3 {
        assert_eq!(screen[row].trim(), "", "header row {row}: {screen:#?}");
    }

    // The block-letter wordmark `tome`, four rows of half blocks (no
    // descender row: the rule comes straight after), accent at its left and
    // accent2 at its right, on a glow that's gone by the screen's edge.
    let wordmark: Vec<String> = (WORD_ROW..WORD_ROW + 4)
        .map(|row| cells(&tome, row, WORD_X, WORD_W))
        .collect();
    assert_eq!(
        wordmark,
        [
            " ▄",
            "▀█▀  ▄▀▀▀▄  █▀▄▀▄  ▄▀▀▀▄",
            " █   █   █  █ █ █  █▀▀▀▀",
            " █▄  ▀▄▄▄▀  █ █ █  ▀▄▄▄▄",
        ]
    );
    assert_eq!(tome.text_col(WORD_ROW + 1, "▀█▀"), Some(WORD_X));
    assert_eq!(tome.fg_at(WORD_X, WORD_ROW + 1), ACCENT);
    assert_eq!(tome.fg_at(WORD_X + WORD_W - 1, WORD_ROW + 1), ACCENT2);
    assert_ne!(tome.bg_at(WORD_X + 15, WORD_ROW + 2), BG);
    assert_eq!(tome.bg_at(0, WORD_ROW + 2), BG);

    // The rule under it, 63 wide, fading into the ground at both ends.
    assert_eq!(cells(&tome, RULE_ROW, RULE_X, 63), "─".repeat(63));
    assert_eq!(tome.text_col(RULE_ROW, "─"), Some(RULE_X));
    assert_eq!(tome.fg_at(RULE_X, RULE_ROW), tome.bg_at(RULE_X, RULE_ROW));

    // The path: `~/` in muted, the last folder in bold strong, centred.
    assert_eq!(cells(&tome, PATH_ROW, 0, 100).trim(), "~/src");
    assert_eq!(tome.text_col(PATH_ROW, "~/src"), Some(47));
    assert_eq!(tome.fg_at(47, PATH_ROW), MUTED);
    assert_eq!(tome.fg_at(49, PATH_ROW), STRONG);
    assert!(tome.bold_at(49, PATH_ROW));

    // The actions one blank row apart, with hints and keys; the first
    // selected on the dialog glow.
    let row = |label: &str, hint: &str, key: &str| format!("{label:<18}{hint:<32}{key}");
    assert_eq!(
        cells(&tome, NEW_FILE_ROW, TEXT_X, 56),
        format!("✦ {}", row("New file", "in ~/src", "n"))
    );
    assert_eq!(
        cells(&tome, NEW_DIR_ROW, LABEL_X, 56),
        row("New directory", "in ~/src", "d")
    );
    assert_eq!(
        cells(&tome, OPEN_DIR_ROW, LABEL_X, 56),
        row("Open directory", "choose a folder", "o")
    );
    for blank in [NEW_FILE_ROW + 1, NEW_DIR_ROW + 1, OPEN_DIR_ROW + 1] {
        assert_eq!(cells(&tome, blank, 0, 100), "", "row {blank}");
    }
    assert_eq!(tome.bg_at(ROW_X, NEW_FILE_ROW), mix(RAISED, ACCENT, 0.3));
    assert_eq!(tome.fg_at(TEXT_X, NEW_FILE_ROW), ACCENT2);
    assert_eq!(tome.fg_at(LABEL_X, NEW_FILE_ROW), STRONG);
    assert!(tome.bold_at(LABEL_X, NEW_FILE_ROW));
    assert_eq!(tome.fg_at(HINT_X, NEW_FILE_ROW), MUTED);
    assert_eq!(tome.fg_at(KEY_X, NEW_FILE_ROW), ACCENT);
    assert!(tome.bold_at(KEY_X, NEW_FILE_ROW));
    assert_eq!(tome.bg_at(ROW_X, NEW_DIR_ROW), BG);
    assert_eq!(tome.fg_at(LABEL_X, NEW_DIR_ROW), TEXT);
    assert!(!tome.bold_at(LABEL_X, NEW_DIR_ROW));
    assert_eq!(tome.fg_at(HINT_X, NEW_DIR_ROW), MUTED);
    assert_eq!(tome.fg_at(KEY_X, NEW_DIR_ROW), MUTED);

    // No card: no lit edge and no footer, and the ground below the rows.
    let screen = tome.screen().join("\n");
    assert!(!screen.contains("ctrl+p go to file"), "{screen}");
    assert_eq!(tome.bg_at(ROW_X, OPEN_DIR_ROW + 2), BG);
}

#[test]
fn mono_has_no_glow_and_reverses_the_selected_row() {
    let (_home, mut tome) = start_bare_with("theme = \"mono\"\n");
    let selected = format!("  ✦ {:<18}{:<32}n ", "New file", "in ~/src");
    tome.wait_for_reversed(NEW_FILE_ROW, &selected, WAIT);
    assert_eq!(tome.reversed_text(NEW_DIR_ROW), "");
    // The wordmark and rule in the terminal's own colours, on its own ground.
    assert_eq!(tome.fg_at(WORD_X, WORD_ROW + 1), Color::Default);
    assert_eq!(tome.bg_at(WORD_X + 15, WORD_ROW + 2), Color::Default);
    assert_eq!(tome.fg_at(50, RULE_ROW), Color::Default);

    tome.send_keys("down");
    let selected = format!("  ✦ {:<18}{:<32}d ", "New directory", "in ~/src");
    tome.wait_for_reversed(NEW_DIR_ROW, &selected, WAIT);
    assert_eq!(tome.reversed_text(NEW_FILE_ROW), "");
}

#[test]
fn arrows_move_and_wrap_and_enter_or_a_letter_runs_a_row() {
    let (_dir, mut tome) = start_bare();
    wait_for_selected(&tome, TEXT_X, NEW_FILE_ROW);
    tome.send_keys("up");
    wait_for_selected(&tome, TEXT_X, OPEN_DIR_ROW);
    tome.send_keys("down");
    wait_for_selected(&tome, TEXT_X, NEW_FILE_ROW);
    tome.send_keys("down");
    wait_for_selected(&tome, TEXT_X, NEW_DIR_ROW);

    // Enter on New directory opens the tree's `A` prompt; Esc comes back.
    tome.send_keys("enter");
    tome.wait_for_text(" New folder  ", WAIT);
    tome.send_keys("esc");
    tome.wait_for_text_gone(" New folder  ", WAIT);
    tome.send_keys("down");
    wait_for_selected(&tome, TEXT_X, OPEN_DIR_ROW);
    // Enter on Open directory opens the folder browser; Esc comes back.
    tome.send_keys("enter");
    tome.wait_for_text(BROWSER, WAIT);
    tome.send_keys("esc");
    tome.wait_for_text_gone(BROWSER, WAIT);

    // The letters run their rows whatever is selected.
    tome.type_text("d");
    tome.wait_for_text(" New folder  ", WAIT);
    tome.send_keys("esc");
    tome.wait_for_text_gone(" New folder  ", WAIT);
    tome.type_text("o");
    tome.wait_for_text(BROWSER, WAIT);
    tome.send_keys("esc");
    tome.wait_for_text_gone(BROWSER, WAIT);
    tome.type_text("n");
    tome.wait_for_text(" New file  ", WAIT);
    // No placeholder is left.
    let screen = tome.screen().join("\n");
    assert!(!screen.contains("coming soon"), "{screen}");
}

#[test]
fn a_click_on_a_row_runs_it() {
    let (_dir, mut tome) = start_bare();
    tome.click(TEXT_X + 4, NEW_DIR_ROW);
    tome.wait_for_text(" New folder  ", WAIT);
    tome.send_keys("esc");
    tome.wait_for_text_gone(" New folder  ", WAIT);
    wait_for_selected(&tome, TEXT_X, NEW_DIR_ROW);
    tome.click(TEXT_X + 4, OPEN_DIR_ROW);
    tome.wait_for_text(BROWSER, WAIT);
    tome.send_keys("esc");
    tome.wait_for_text_gone(BROWSER, WAIT);
    wait_for_selected(&tome, TEXT_X, OPEN_DIR_ROW);
    // Off the rows a click does nothing: no cursor is placed in the hidden
    // buffer, and the selection stays.
    tome.click(5, 5);
    tome.click(TEXT_X + 4, WORD_ROW + 2);
    wait_for_selected(&tome, TEXT_X, OPEN_DIR_ROW);
    tome.wait_for_text("Open directory", WAIT);
    tome.click(TEXT_X + 4, NEW_FILE_ROW);
    tome.wait_for_text(" New file  ", WAIT);
}

#[test]
fn esc_or_ctrl_n_leaves_the_splash_for_the_untitled_buffer() {
    for key in ["esc", "ctrl+n"] {
        let (_dir, mut tome) = start_bare();
        tome.send_keys(key);
        tome.wait_for_text_gone("Open directory", WAIT);
        // One untitled tab, the one the splash stood in for, taking the typing.
        tome.wait_for_text(&pill_text(&["untitled"], 0), WAIT);
        tome.type_text("hi");
        tome.wait_for_text("1  hi", WAIT);
    }
}

#[test]
fn opening_a_file_with_ctrl_p_replaces_the_splash() {
    let mut tome = Tome::spawn_in(Path::new(PROJECT), &[]);
    tome.wait_for_text("Open directory", START);
    tome.send_keys("ctrl+p");
    tome.type_text("notes.txt");
    tome.wait_for_text("✦ notes.txt", WAIT);
    tome.send_keys("enter");
    tome.wait_for_text("notes for the tree test", WAIT);
    tome.wait_for_text(&pill_text(&["notes.txt"], 0), WAIT);
    let screen = tome.screen().join("\n");
    assert!(!screen.contains("Open directory"), "{screen}");
}

#[test]
fn with_the_tree_open_the_splash_has_focus_and_ctrl_e_moves_it_to_the_tree() {
    let mut tome = Tome::spawn(&[PROJECT]);
    tome.wait_for_text("Open directory", START);
    // Ctrl+B shows the tree the splash hides, leaving the keys on the splash.
    tome.send_keys("ctrl+b");
    tome.wait_for_text("README.md", WAIT);
    // Beside the 28-column tree and its blank column the editor area is 71
    // wide from column 29, so the action rows start at 36 and `✦` is at 38.
    let text_x = 38;
    wait_for_selected(&tome, text_x, NEW_FILE_ROW);
    // ↓ moves the splash's selection, not the tree's.
    tome.send_keys("down");
    wait_for_selected(&tome, text_x, NEW_DIR_ROW);

    tome.send_keys("ctrl+e");
    // In the tree ↓ moves from `docs` past `src` and `.gitignore` to
    // `notes.txt`; Enter opens it in place of the splash.
    for _ in 0..3 {
        tome.send_keys("down");
    }
    tome.send_keys("enter");
    tome.wait_for_text("notes for the tree test", WAIT);
    tome.wait_for_text_gone("Open directory", WAIT);
}

#[test]
fn new_file_creates_in_the_project_folder_and_opens_it() {
    let parent = tempfile::tempdir().expect("create temp dir");
    let root = parent.path().join("proj");
    fs::create_dir(&root).expect("create proj");
    fs::write(root.join("a.txt"), "alpha\n").expect("write a.txt");
    // cwd is the parent, so the project folder isn't where Tome runs.
    let mut tome = Tome::spawn_in(parent.path(), &["proj"]);
    tome.wait_for_text("Open directory", START);

    // Esc in the name prompt comes back to the splash, which takes keys again.
    tome.type_text("n");
    tome.wait_for_text("⏎ create   esc cancel", WAIT);
    tome.send_keys("esc");
    tome.wait_for_text_gone("⏎ create   esc cancel", WAIT);
    tome.wait_for_text("Open directory", WAIT);

    // A name that's taken, or not a plain name, is refused as in the tree.
    tome.send_keys("enter");
    tome.wait_for_text("⏎ create   esc cancel", WAIT);
    tome.type_text("a.txt");
    tome.send_keys("enter");
    wait_for_status(&tome, "a.txt already exists");
    tome.wait_for_text("Open directory", WAIT);
    tome.type_text("n");
    tome.wait_for_text("⏎ create   esc cancel", WAIT);
    tome.type_text("sub/b.txt");
    tome.send_keys("enter");
    wait_for_status(&tome, "invalid name");
    tome.wait_for_text("Open directory", WAIT);
    assert_eq!(fs::read_dir(&root).expect("list proj").count(), 1);

    tome.type_text("n");
    tome.wait_for_text("⏎ create   esc cancel", WAIT);
    tome.type_text("b.txt");
    tome.send_keys("enter");
    wait_for_status(&tome, "created b.txt");
    tome.wait_for_files("b.txt in the project", WAIT, || {
        root.join("b.txt").is_file()
    });
    assert!(!parent.path().join("b.txt").exists());
    tome.wait_for_text_gone("Open directory", WAIT);
    tome.wait_for_text(&pill_text(&["b.txt"], 0), WAIT);
    tome.type_text("hi");
    tome.wait_for_text("1  hi", WAIT);
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
fn wait_for_brand(tome: &Tome, dir: &Path) {
    let name = name(dir);
    let tail: String = name
        .chars()
        .skip(name.chars().count().saturating_sub(6))
        .collect();
    tome.wait_for_screen(&format!("brand ending {tail:?}"), WAIT, |screen| {
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
    // cwd is the parent, so the project folder isn't where Tome runs.
    let mut tome = Tome::spawn_in(parent.path(), &["proj"]);
    tome.wait_for_text("Open directory", START);
    tome.send_keys("ctrl+b");
    tome.wait_for_text("a.txt", WAIT);

    // A taken name is refused as in the tree, and nothing switches.
    tome.type_text("d");
    tome.wait_for_text(" New folder  ", WAIT);
    tome.type_text("a.txt");
    tome.send_keys("enter");
    wait_for_status(&tome, "a.txt already exists");
    tome.wait_for_text("Open directory", WAIT);
    tome.wait_for_text("a.txt", WAIT);

    tome.type_text("d");
    tome.wait_for_text(" New folder  ", WAIT);
    tome.type_text("fresh-dir");
    tome.send_keys("enter");
    wait_for_status(&tome, "created fresh-dir");
    let fresh = root.join("fresh-dir");
    tome.wait_for_files("fresh-dir in the project", WAIT, || fresh.is_dir());
    assert!(!parent.path().join("fresh-dir").exists());

    // The tree and brand show the new, empty folder beside the no file open
    // key list; the splash is gone.
    wait_for_brand(&tome, &fresh);
    tome.wait_for_screen("the old folder's files gone", WAIT, |screen| {
        !screen.iter().any(|line| line.contains("a.txt"))
    });
    tome.wait_for_text_gone("Open directory", WAIT);
    tome.wait_for_text("no file open", WAIT);

    // The tree has the keys: its `a` makes a file in the new folder.
    tome.type_text("a");
    tome.wait_for_text("⏎ create   esc cancel", WAIT);
    tome.type_text("b.txt");
    tome.send_keys("enter");
    tome.wait_for_files("b.txt in the new folder", WAIT, || {
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
    let mut tome = Tome::spawn_in(dir.path(), &["."]);
    tome.wait_for_text("Open directory", START);

    // `o`, ↓ to `other`, Enter into it, Enter on its open row.
    tome.type_text("o");
    tome.wait_for_text(BROWSER, WAIT);
    // The browser's header is row 6 and its folders start on row 9 (the
    // tree beside it lists `other` too, so the rows are named).
    tome.wait_for_screen("other in the browser", WAIT, |screen| {
        screen[9].contains("▸ other")
    });
    tome.send_keys("down");
    tome.send_keys("enter");
    tome.wait_for_screen("inside other", WAIT, |screen| screen[6].contains("other"));
    tome.send_keys("enter");
    tome.wait_for_text_gone(BROWSER, WAIT);

    // The tree and brand show `other` beside the no file open key list, and
    // the tree takes the keys: Enter opens the file it selects.
    tome.wait_for_text("inside.txt", WAIT);
    wait_for_brand(&tome, &other);
    tome.wait_for_text_gone("Open directory", WAIT);
    tome.wait_for_text("no file open", WAIT);
    tome.send_keys("enter");
    tome.wait_for_text(&pill_text(&["inside.txt"], 0), WAIT);
}

#[test]
fn the_splash_hides_the_tree_until_ctrl_b_or_ctrl_e_shows_it() {
    let (_home, mut tome) = start_bare();
    let screen = tome.screen().join("\n");
    assert!(!screen.contains("a.txt"), "{screen}");
    tome.send_keys("ctrl+b");
    tome.wait_for_text("a.txt", WAIT);
    // The keys stay on the splash beside it: the rows moved right of the tree.
    tome.send_keys("down");
    wait_for_selected(&tome, 38, NEW_DIR_ROW);

    let mut tome = Tome::spawn(&[PROJECT]);
    tome.wait_for_text("Open directory", START);
    let screen = tome.screen().join("\n");
    assert!(!screen.contains("README.md"), "{screen}");
    tome.send_keys("ctrl+e");
    tome.wait_for_text("README.md", WAIT);
}

#[test]
fn leaving_the_splash_gives_back_the_tree_a_folder_launch_shows() {
    for key in ["esc", "ctrl+n"] {
        let mut tome = Tome::spawn(&[PROJECT]);
        tome.wait_for_text("Open directory", START);
        let screen = tome.screen().join("\n");
        assert!(!screen.contains("README.md"), "{screen}");
        tome.send_keys(key);
        tome.wait_for_text_gone("Open directory", WAIT);
        tome.wait_for_text("README.md", WAIT);
    }

    // A file opened from Ctrl+P replaces the splash with the tree beside it.
    let mut tome = Tome::spawn(&[PROJECT]);
    tome.wait_for_text("Open directory", START);
    tome.send_keys("ctrl+p");
    tome.type_text("notes.txt");
    tome.wait_for_text("✦ notes.txt", WAIT);
    tome.send_keys("enter");
    tome.wait_for_text("notes for the tree test", WAIT);
    tome.wait_for_text("README.md", WAIT);
}

#[test]
fn a_bare_start_keeps_the_tree_hidden_after_the_splash() {
    let (_home, mut tome) = start_bare();
    tome.send_keys("esc");
    tome.wait_for_text_gone("Open directory", WAIT);
    tome.wait_for_text(&pill_text(&["untitled"], 0), WAIT);
    let screen = tome.screen().join("\n");
    assert!(!screen.contains("a.txt"), "{screen}");
}

#[test]
fn the_status_bar_names_the_project_and_the_splash_keys_and_version() {
    let (_home, mut tome) = start_bare();
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    wait_for_status(&tome, &version);
    let status = tome.screen()[usize::from(STATUS)].clone();
    // The project where `untitled` would be, and no cursor or language.
    assert_eq!(tome.text_col(STATUS, "~/src"), Some(20), "{status:?}");
    // All of it muted, `src` too, unlike a file's lit name.
    for x in 20..25 {
        assert_eq!(tome.fg_at(x, STATUS), MUTED, "column {x}");
    }
    assert!(!status.contains("untitled"), "{status:?}");
    assert!(!status.contains("Ln 1"), "{status:?}");
    assert!(
        status
            .trim_end()
            .ends_with(&format!("↑↓ select  ⏎ choose  q quit      {version}")),
        "{status:?}"
    );
    // Right-aligned to end two cells short of the edge.
    let version_x = COLS - 2 - u16::try_from(version.len()).expect("short version");
    assert_eq!(tome.text_col(STATUS, &version), Some(version_x));
    // Keys lit; their labels and the version muted.
    let q = tome.text_col(STATUS, "q quit").expect("q quit shown");
    assert_eq!(tome.fg_at(q, STATUS), TEXT);
    assert_eq!(tome.fg_at(q + 2, STATUS), MUTED);
    assert_eq!(tome.fg_at(version_x, STATUS), MUTED);

    // Leaving the splash gives the untitled buffer's status back.
    tome.send_keys("esc");
    wait_for_status(&tome, "Ln 1, Col 1");
    assert!(!tome.screen()[usize::from(STATUS)].contains("q quit"));
}

#[test]
fn q_on_the_splash_quits() {
    let (_home, mut tome) = start_bare();
    tome.send_keys("q");
    assert!(tome.wait_exit(WAIT).success());
}

#[test]
fn q_typed_in_a_buffer_inserts_q() {
    let (_home, mut tome) = start_bare();
    tome.send_keys("ctrl+n");
    tome.wait_for_text_gone("Open directory", WAIT);
    tome.type_text("q");
    tome.wait_for_text("1  q", WAIT);
    tome.assert_running_for(Duration::from_millis(300));
}
