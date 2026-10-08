//! The splash Glyph opens on with nothing to edit (glyph-splash spec S1–S5,
//! S7): when it shows, how it's drawn, its keys and clicks, *New file*, *New
//! directory* and *Open directory*, its status bar, `q`, the tree it hides, and
//! that a project switch from it lands on the tree beside an empty pane rather
//! than on a new splash.

mod harness;

use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{COLS, Glyph, ROWS, pill_text};
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
/// tab header) to the status line: 100 × 26. The 14-row splash is centred in
/// it: the wordmark on rows 9–13, the rule on 14, the path on 16 and the
/// actions on 18, 20 and 22.
const WORD_ROW: u16 = 9;
const RULE_ROW: u16 = 14;
const PATH_ROW: u16 = 16;
const NEW_FILE_ROW: u16 = 18;
const NEW_DIR_ROW: u16 = 20;
const OPEN_DIR_ROW: u16 = 22;
/// The 31-wide wordmark and 63-wide rule, centred.
const WORD_X: u16 = 34;
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
fn cells(glyph: &Glyph, row: u16, x: u16, width: u16) -> String {
    glyph.screen()[usize::from(row)]
        .chars()
        .skip(usize::from(x))
        .take(usize::from(width))
        .collect::<String>()
        .trim_end()
        .to_string()
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

/// A temp folder standing in for the home folder, holding `src` with `a.txt`
/// in it, and Glyph started in `src` with no argument, so the splash names
/// the project `~/src` as in the design.
fn start_bare_with(toml: &str) -> (TempDir, Glyph) {
    let home = tempfile::tempdir().expect("create temp dir");
    let src = home.path().join("src");
    fs::create_dir(&src).expect("create src");
    fs::write(src.join("a.txt"), "alpha\n").expect("write a.txt");
    // `HOME` on Unix and `USERPROFILE` on Windows are where Glyph reads the
    // home folder from.
    let env = [
        ("HOME", OsString::from(home.path())),
        ("USERPROFILE", OsString::from(home.path())),
    ];
    let glyph = Glyph::spawn_in_with_config_and_env(&src, toml, &env, &[]);
    glyph.wait_for_text("Open directory", START);
    (home, glyph)
}

fn start_bare() -> (TempDir, Glyph) {
    start_bare_with("")
}

#[test]
fn glyph_alone_and_glyph_on_a_folder_show_the_splash_but_a_file_does_not() {
    let (_dir, _glyph) = start_bare();

    let glyph = Glyph::spawn(&[PROJECT]);
    glyph.wait_for_text("Open directory", START);

    let glyph = Glyph::spawn(&["tests/fixtures/project/notes.txt"]);
    glyph.wait_for_text("notes for the tree test", START);
    let screen = glyph.screen().join("\n");
    assert!(!screen.contains("Open directory"), "{screen}");
}

#[test]
fn the_splash_has_the_wordmark_rule_path_and_actions_with_no_card() {
    let (_home, glyph) = start_bare();
    glyph.wait_for_fg_at(LABEL_X, NEW_FILE_ROW, STRONG, WAIT);

    // No pills or thread in the header rows.
    let screen = glyph.screen();
    for row in 0..3 {
        assert_eq!(screen[row].trim(), "", "header row {row}: {screen:#?}");
    }

    // The block-letter wordmark, five rows of half blocks, accent at its left
    // and accent2 at its right, on a glow that's gone by the screen's edge.
    let wordmark: Vec<String> = (WORD_ROW..WORD_ROW + 5)
        .map(|row| cells(&glyph, row, WORD_X, 31))
        .collect();
    assert_eq!(
        wordmark,
        [
            "       ▀█                 █",
            "▄▀▀▀█   █   █   █  █▀▀▀▄  █▀▀▀▄",
            "█   █   █   █   █  █   █  █   █",
            "▀▄▄▄█   █▄  ▀▄▄▄█  █▄▄▄▀  █   █",
            "▄▄▄▄▀       ▄▄▄▄▀  █",
        ]
    );
    assert_eq!(glyph.text_col(WORD_ROW + 1, "▄▀▀▀█"), Some(WORD_X));
    assert_eq!(glyph.fg_at(WORD_X, WORD_ROW + 1), ACCENT);
    assert_eq!(glyph.fg_at(WORD_X + 30, WORD_ROW + 1), ACCENT2);
    assert_ne!(glyph.bg_at(WORD_X + 15, WORD_ROW + 2), BG);
    assert_eq!(glyph.bg_at(0, WORD_ROW + 2), BG);

    // The rule under it, 63 wide, fading into the ground at both ends.
    assert_eq!(cells(&glyph, RULE_ROW, RULE_X, 63), "─".repeat(63));
    assert_eq!(glyph.text_col(RULE_ROW, "─"), Some(RULE_X));
    assert_eq!(glyph.fg_at(RULE_X, RULE_ROW), glyph.bg_at(RULE_X, RULE_ROW));

    // The path: `~/` in muted, the last folder in bold strong, centred.
    assert_eq!(cells(&glyph, PATH_ROW, 0, 100).trim(), "~/src");
    assert_eq!(glyph.text_col(PATH_ROW, "~/src"), Some(47));
    assert_eq!(glyph.fg_at(47, PATH_ROW), MUTED);
    assert_eq!(glyph.fg_at(49, PATH_ROW), STRONG);
    assert!(glyph.bold_at(49, PATH_ROW));

    // The actions one blank row apart, with hints and keys; the first
    // selected on the dialog glow.
    let row = |label: &str, hint: &str, key: &str| format!("{label:<18}{hint:<32}{key}");
    assert_eq!(
        cells(&glyph, NEW_FILE_ROW, TEXT_X, 56),
        format!("✦ {}", row("New file", "in ~/src", "n"))
    );
    assert_eq!(
        cells(&glyph, NEW_DIR_ROW, LABEL_X, 56),
        row("New directory", "in ~/src", "d")
    );
    assert_eq!(
        cells(&glyph, OPEN_DIR_ROW, LABEL_X, 56),
        row("Open directory", "choose a folder", "o")
    );
    for blank in [NEW_FILE_ROW + 1, NEW_DIR_ROW + 1, OPEN_DIR_ROW + 1] {
        assert_eq!(cells(&glyph, blank, 0, 100), "", "row {blank}");
    }
    assert_eq!(glyph.bg_at(ROW_X, NEW_FILE_ROW), mix(RAISED, ACCENT, 0.3));
    assert_eq!(glyph.fg_at(TEXT_X, NEW_FILE_ROW), ACCENT2);
    assert_eq!(glyph.fg_at(LABEL_X, NEW_FILE_ROW), STRONG);
    assert!(glyph.bold_at(LABEL_X, NEW_FILE_ROW));
    assert_eq!(glyph.fg_at(HINT_X, NEW_FILE_ROW), MUTED);
    assert_eq!(glyph.fg_at(KEY_X, NEW_FILE_ROW), ACCENT);
    assert!(glyph.bold_at(KEY_X, NEW_FILE_ROW));
    assert_eq!(glyph.bg_at(ROW_X, NEW_DIR_ROW), BG);
    assert_eq!(glyph.fg_at(LABEL_X, NEW_DIR_ROW), TEXT);
    assert!(!glyph.bold_at(LABEL_X, NEW_DIR_ROW));
    assert_eq!(glyph.fg_at(HINT_X, NEW_DIR_ROW), MUTED);
    assert_eq!(glyph.fg_at(KEY_X, NEW_DIR_ROW), MUTED);

    // No card: no lit edge and no footer, and the ground below the rows.
    let screen = glyph.screen().join("\n");
    assert!(!screen.contains("ctrl+p go to file"), "{screen}");
    assert_eq!(glyph.bg_at(ROW_X, OPEN_DIR_ROW + 2), BG);
}

#[test]
fn mono_has_no_glow_and_reverses_the_selected_row() {
    let (_home, mut glyph) = start_bare_with("theme = \"mono\"\n");
    let selected = format!("  ✦ {:<18}{:<32}n ", "New file", "in ~/src");
    glyph.wait_for_reversed(NEW_FILE_ROW, &selected, WAIT);
    assert_eq!(glyph.reversed_text(NEW_DIR_ROW), "");
    // The wordmark and rule in the terminal's own colours, on its own ground.
    assert_eq!(glyph.fg_at(WORD_X, WORD_ROW + 1), Color::Default);
    assert_eq!(glyph.bg_at(WORD_X + 15, WORD_ROW + 2), Color::Default);
    assert_eq!(glyph.fg_at(50, RULE_ROW), Color::Default);

    glyph.send_keys("down");
    let selected = format!("  ✦ {:<18}{:<32}d ", "New directory", "in ~/src");
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
    glyph.click(TEXT_X + 4, WORD_ROW + 2);
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
    glyph.wait_for_text("Open directory", START);
    // Ctrl+B shows the tree the splash hides, leaving the keys on the splash.
    glyph.send_keys("ctrl+b");
    glyph.wait_for_text("README.md", WAIT);
    // Beside the 28-column tree and its blank column the editor area is 71
    // wide from column 29, so the action rows start at 36 and `✦` is at 38.
    let text_x = 38;
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
    glyph.send_keys("ctrl+b");
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

#[test]
fn the_splash_hides_the_tree_until_ctrl_b_or_ctrl_e_shows_it() {
    let (_home, mut glyph) = start_bare();
    let screen = glyph.screen().join("\n");
    assert!(!screen.contains("a.txt"), "{screen}");
    glyph.send_keys("ctrl+b");
    glyph.wait_for_text("a.txt", WAIT);
    // The keys stay on the splash beside it: the rows moved right of the tree.
    glyph.send_keys("down");
    wait_for_selected(&glyph, 38, NEW_DIR_ROW);

    let mut glyph = Glyph::spawn(&[PROJECT]);
    glyph.wait_for_text("Open directory", START);
    let screen = glyph.screen().join("\n");
    assert!(!screen.contains("README.md"), "{screen}");
    glyph.send_keys("ctrl+e");
    glyph.wait_for_text("README.md", WAIT);
}

#[test]
fn the_status_bar_names_the_project_and_the_splash_keys_and_version() {
    let (_home, mut glyph) = start_bare();
    let version = format!("v{}", env!("CARGO_PKG_VERSION"));
    wait_for_status(&glyph, &version);
    let status = glyph.screen()[usize::from(STATUS)].clone();
    // The project where `untitled` would be, and no cursor or language.
    assert_eq!(glyph.text_col(STATUS, "~/src"), Some(20), "{status:?}");
    assert!(!status.contains("untitled"), "{status:?}");
    assert!(!status.contains("Ln 1"), "{status:?}");
    assert!(
        status
            .trim_end()
            .ends_with(&format!("↑↓ select  ⏎ choose  q quit    {version}")),
        "{status:?}"
    );
    // Right-aligned to end two cells short of the edge.
    let version_x = COLS - 2 - u16::try_from(version.len()).expect("short version");
    assert_eq!(glyph.text_col(STATUS, &version), Some(version_x));
    // Keys lit; their labels and the version muted.
    let q = glyph.text_col(STATUS, "q quit").expect("q quit shown");
    assert_eq!(glyph.fg_at(q, STATUS), TEXT);
    assert_eq!(glyph.fg_at(q + 2, STATUS), MUTED);
    assert_eq!(glyph.fg_at(version_x, STATUS), MUTED);

    // Leaving the splash gives the untitled buffer's status back.
    glyph.send_keys("esc");
    wait_for_status(&glyph, "Ln 1, Col 1");
    assert!(!glyph.screen()[usize::from(STATUS)].contains("q quit"));
}

#[test]
fn q_on_the_splash_quits() {
    let (_home, mut glyph) = start_bare();
    glyph.send_keys("q");
    assert!(glyph.wait_exit(WAIT).success());
}

#[test]
fn q_typed_in_a_buffer_inserts_q() {
    let (_home, mut glyph) = start_bare();
    glyph.send_keys("ctrl+n");
    glyph.wait_for_text_gone("Open directory", WAIT);
    glyph.type_text("q");
    glyph.wait_for_text("1  q", WAIT);
    glyph.assert_running_for(Duration::from_millis(300));
}
