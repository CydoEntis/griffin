//! `>open directory`: the folder browser, and opening the folder it picks as the
//! project (tome-splash spec S6–S9).

mod harness;

use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{ROWS, Tome};
use tempfile::TempDir;
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(10);

/// At 100x30 the browser is the cast's card: 86 wide from column 7, dropping
/// from row 4 with its lit edge. The header is row 6, the rule row 7, the open
/// row 8 and the folders from row 9, their text four cells in; the right-aligned
/// text ends four cells in from the card's edge.
const CARD_X: u16 = 7;
const CARD_Y: u16 = 4;
const HEADER_ROW: u16 = 6;
const RULE_ROW: u16 = 7;
const OPEN_ROW: u16 = 8;
const FIRST_ROW: u16 = 9;
const TEXT_X: u16 = 11;
const RIGHT: u16 = 89;
const SCOPE: &str = "open · folders";
const FOOTER: &str = "⏎ into  ← up  ctrl+⏎ open here  esc cancel";
/// The run panel's title row at 30 rows.
const TITLE_ROW: u16 = 20;

/// The default theme, hydra: its `accent`, `raised` and the tree's glow start.
const ACCENT: Color = Color::Rgb(0xc3, 0xf5, 0x3c);
const RAISED: Color = Color::Rgb(0x0f, 0x18, 0x21);
const TREE_GLOW: Color = Color::Rgb(0x3c, 0x4e, 0x24);

/// `a` moved `t` of the way to `b`, per channel, rounded, as the editor mixes.
fn mix(a: Color, b: Color, t: f64) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (a, b) else {
        panic!("can't mix {a:?} and {b:?}");
    };
    let m = |p: u8, q: u8| (f64::from(p) + (f64::from(q) - f64::from(p)) * t).round() as u8;
    Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
}

/// A folder holding `folders` (nested with `/`) and `files` (name, text).
fn folder(folders: &[&str], files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().expect("create temp folder");
    for name in folders {
        fs::create_dir_all(dir.path().join(name)).expect("create folder");
    }
    for (name, text) in files {
        fs::write(dir.path().join(name), text).expect("write file");
    }
    dir
}

/// The last part of `dir`'s path, which the header and brand row end with.
fn name(dir: &Path) -> String {
    dir.file_name()
        .expect("temp folders have names")
        .to_string_lossy()
        .into_owned()
}

fn row(tome: &Tome, row: u16) -> String {
    tome.screen()[usize::from(row)].clone()
}

/// Tome on `dir` as the project, tree showing.
fn open_in(dir: &Path, config: &str) -> Tome {
    let mut tome = Tome::spawn_in_with_config(dir, config, &["."]);
    tome.wait_for_text("Open directory", START);
    // The splash hides the tree; Ctrl+B shows it, with its brand on row 1.
    tome.send_keys("ctrl+b");
    tome.wait_for_screen("the tree's brand", WAIT, |screen| {
        screen[1].contains("✦ tome")
    });
    tome
}

/// Ctrl+P, `>open directory`, Enter.
fn open_browser(tome: &mut Tome) {
    tome.send_keys("ctrl+p");
    tome.wait_for_text("cast · files · commands", WAIT);
    tome.type_text(">open directory");
    tome.wait_for_text("cast · commands", WAIT);
    tome.wait_for_screen("Open directory listed", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].contains("Open directory")
    });
    tome.send_keys("enter");
    tome.wait_for_text(SCOPE, WAIT);
}

#[test]
fn open_directory_in_the_cast_opens_the_folder_browser() {
    let dir = folder(&["alpha", "Beta", ".hidden"], &[("notes.txt", "")]);
    let mut tome = open_in(dir.path(), "");
    open_browser(&mut tome);
    let here = name(dir.path());

    // The lit edge across the cast's card.
    assert_eq!(
        row(&tome, CARD_Y).chars().nth(usize::from(CARD_X)),
        Some('▀')
    );
    // `✦` and the folder, `open · folders` at the right.
    assert_eq!(tome.text_col(HEADER_ROW, "✦ "), Some(TEXT_X));
    let header = row(&tome, HEADER_ROW);
    assert!(header.contains(&here), "{header:?}");
    assert_eq!(
        tome.text_col(HEADER_ROW, SCOPE),
        Some(RIGHT - u16::try_from(SCOPE.chars().count()).unwrap())
    );
    assert_eq!(row(&tome, RULE_ROW).chars().nth(9), Some('─'));
    // The open row first, selected: the glow and `⏎` at the right.
    assert_eq!(tome.text_col(OPEN_ROW, "⏎ open "), Some(TEXT_X));
    assert!(row(&tome, OPEN_ROW).contains(&here));
    assert_eq!(tome.text_col(OPEN_ROW, "⏎    "), Some(RIGHT - 1));
    assert_eq!(tome.bg_at(CARD_X, OPEN_ROW), mix(RAISED, ACCENT, 0.3));
    // Folders only, ignoring case, dot-folders last.
    assert_eq!(tome.text_col(FIRST_ROW, "▸ alpha"), Some(TEXT_X));
    assert_eq!(tome.text_col(FIRST_ROW + 1, "▸ Beta"), Some(TEXT_X));
    assert_eq!(tome.text_col(FIRST_ROW + 2, "▸ .hidden"), Some(TEXT_X));
    // A blank, then the footer.
    assert_eq!(row(&tome, FIRST_ROW + 3).trim(), "");
    assert_eq!(tome.text_col(FIRST_ROW + 4, FOOTER), Some(TEXT_X));
    assert!(
        !tome.screen()[..usize::from(ROWS - 1)]
            .iter()
            .skip(usize::from(CARD_Y))
            .any(|line| line
                .chars()
                .skip(usize::from(CARD_X))
                .collect::<String>()
                .contains("notes.txt"))
    );
}

#[test]
fn many_folders_scroll_past_the_card() {
    let names: Vec<String> = (0..14).map(|i| format!("dir{i:02}")).collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let dir = folder(&refs, &[]);
    let mut tome = open_in(dir.path(), "");
    open_browser(&mut tome);
    // Ten rows, then a blank and the footer.
    assert_eq!(tome.text_col(FIRST_ROW + 9, "▸ dir09"), Some(TEXT_X));
    assert_eq!(tome.text_col(FIRST_ROW + 11, FOOTER), Some(TEXT_X));
    for _ in 0..14 {
        tome.send_keys("down");
    }
    tome.wait_for_screen("the last folder on the last row", WAIT, |screen| {
        screen[usize::from(FIRST_ROW + 9)].contains("▸ dir13")
    });
    assert!(row(&tome, FIRST_ROW).contains("▸ dir04"));
}

#[test]
fn mono_reverses_the_selected_row() {
    let dir = folder(&["alpha"], &[]);
    let mut tome = open_in(dir.path(), "theme = \"mono\"\n");
    open_browser(&mut tome);
    let selected = tome.reversed_text(OPEN_ROW);
    assert_eq!(selected.chars().count(), 86, "{selected:?}");
    assert!(selected.starts_with("    ⏎ open "), "{selected:?}");
    tome.send_keys("down");
    // The whole card's width, `⏎` four cells in from its right edge.
    tome.wait_for_reversed(FIRST_ROW, &format!("{:<81}⏎    ", "    ▸ alpha"), WAIT);
    assert_eq!(tome.reversed_text(OPEN_ROW), "");
}

#[test]
fn the_browser_goes_in_and_up_filters_and_closes() {
    let dir = folder(&["alpha/inner", "Beta", ".hidden"], &[]);
    let here = name(dir.path());
    let mut tome = open_in(dir.path(), "");
    open_browser(&mut tome);

    // Enter on a folder goes into it.
    tome.send_keys("down");
    tome.send_keys("enter");
    tome.wait_for_screen("inside alpha", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].contains("▸ inner")
    });
    assert!(row(&tome, HEADER_ROW).contains("alpha"));
    // ← goes back up, with the folder just left selected.
    tome.send_keys("left");
    tome.wait_for_screen("back in the project", WAIT, |screen| {
        screen[usize::from(FIRST_ROW + 1)].contains("▸ Beta")
    });
    tome.wait_for_bg(CARD_X, FIRST_ROW, mix(RAISED, ACCENT, 0.3), WAIT);

    // Typing filters the folders; the open row stays.
    tome.type_text("bet");
    tome.wait_for_screen("only Beta", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].contains("▸ Beta")
            && !screen[usize::from(FIRST_ROW + 1)].contains("▸")
    });
    assert_eq!(tome.text_col(OPEN_ROW, "⏎ open "), Some(TEXT_X));
    // Backspace edits the query while there is one, then goes up.
    for _ in 0..3 {
        tome.send_keys("backspace");
    }
    tome.wait_for_screen("every folder again", WAIT, |screen| {
        screen[usize::from(FIRST_ROW + 2)].contains("▸ .hidden")
    });
    tome.send_keys("backspace");
    tome.wait_for_screen("the parent folder", WAIT, |screen| {
        !screen[usize::from(HEADER_ROW)].contains(&here)
    });

    // Esc closes it.
    tome.send_keys("esc");
    tome.wait_for_text_gone(SCOPE, WAIT);
    // So does a click outside the card, changing nothing.
    open_browser(&mut tome);
    tome.click(2, ROWS - 3);
    tome.wait_for_text_gone(SCOPE, WAIT);
    let brand: String = row(&tome, 1).chars().take(28).collect();
    assert!(
        brand.contains(&here[here.len().saturating_sub(6)..]),
        "{brand:?}"
    );
}

#[test]
fn a_folder_that_cannot_be_read_says_why_and_the_browser_stays() {
    let dir = folder(&["gone", "kept"], &[]);
    let mut tome = open_in(dir.path(), "");
    open_browser(&mut tome);
    fs::remove_dir(dir.path().join("gone")).expect("remove folder");
    tome.send_keys("down");
    tome.send_keys("enter");
    tome.wait_for_screen("the reason in the status line", WAIT, |screen| {
        screen[usize::from(ROWS - 1)].contains("cannot open")
    });
    assert!(row(&tome, ROWS - 1).contains("gone"));
    assert!(row(&tome, HEADER_ROW).contains(SCOPE));
    assert!(row(&tome, FIRST_ROW).contains("▸ gone"));
}

/// A command that runs until it's stopped.
fn long_command() -> &'static str {
    if cfg!(windows) {
        "ping -n 60 127.0.0.1 >NUL"
    } else {
        "sleep 60"
    }
}

#[test]
fn opening_a_folder_makes_it_the_project() {
    let old_toml = format!("[[run]]\nname = \"wait\"\ncommand = '{}'\n", long_command());
    let old = folder(&[], &[("old-file.txt", ""), (".tome.toml", &old_toml)]);
    let new_toml = "[[run]]\nname = \"hello\"\ncommand = \"echo switched-ok\"\n";
    let new = folder(&["sub"], &[("new-file.txt", ""), (".tome.toml", new_toml)]);
    let mut tome = open_in(old.path(), "");
    tome.wait_for_text("old-file.txt", WAIT);
    tome.send_keys("f5");
    tome.wait_for_screen("the old command running", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].contains("wait  running")
    });

    // A typed path jumps there; Enter on the open row opens it.
    open_browser(&mut tome);
    tome.type_text(&new.path().display().to_string());
    tome.send_keys("enter");
    tome.wait_for_screen("the new folder in the browser", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].contains("▸ sub")
    });
    tome.send_keys("enter");
    tome.wait_for_text_gone(SCOPE, WAIT);

    // The tree and brand show the new folder beside the no file open key
    // list, not the splash, and the tree has the keys; the run was stopped.
    tome.wait_for_text("new-file.txt", WAIT);
    assert!(
        !tome
            .screen()
            .iter()
            .any(|line| line.contains("old-file.txt"))
    );
    let brand: String = row(&tome, 1).chars().take(28).collect();
    let new_name = name(new.path());
    assert!(
        brand.contains(&new_name[new_name.len().saturating_sub(6)..]),
        "{brand:?}"
    );
    tome.wait_for_text("no file open", WAIT);
    assert!(
        !tome
            .screen()
            .iter()
            .any(|line| line.contains("Open directory")),
        "no splash after a switch"
    );
    // The selected first row glows only while the tree has focus.
    tome.wait_for_bg(0, 3, TREE_GLOW, WAIT);
    tome.wait_for_screen("the old command stopped", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].contains("wait  stopped")
    });

    // Ctrl+P lists the new folder's files.
    tome.send_keys("ctrl+p");
    tome.wait_for_text("cast · files · commands", WAIT);
    tome.wait_for_text("new-file.txt", WAIT);
    tome.type_text("file");
    tome.wait_for_screen("only the new file", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].contains("new-file.txt")
    });
    assert!(
        !tome
            .screen()
            .iter()
            .any(|line| line.contains("old-file.txt"))
    );
    tome.send_keys("esc");
    tome.wait_for_text_gone("cast · files · commands", WAIT);

    // The old run isn't restarted in the new folder: Ctrl+F5 is F5 again,
    // which runs the new folder's `.tome.toml`.
    tome.send_keys("ctrl+f5");
    tome.wait_for_screen("the new command's output", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].contains("hello")
            && screen[usize::from(TITLE_ROW + 1)].starts_with(" switched-ok")
    });
}

/// A project holding `a.txt`, `b.txt` and the folder `other` (with
/// `marker.txt`), tome open on `a.txt` with `x` typed into it.
fn edited_project() -> (TempDir, Tome) {
    let dir = folder(
        &["other"],
        &[
            (
                "a.txt", "hello
",
            ),
            (
                "b.txt", "bye
",
            ),
            ("other/marker.txt", ""),
        ],
    );
    let mut tome = Tome::spawn_in(dir.path(), &["a.txt"]);
    tome.wait_for_text("hello", START);
    tome.type_text("x");
    tome.wait_for_text("xhello", WAIT);
    (dir, tome)
}

/// Opens `b.txt` through Ctrl+P and types `y` into it.
fn edit_b(tome: &mut Tome) {
    tome.send_keys("ctrl+p");
    tome.wait_for_text("cast · files · commands", WAIT);
    tome.type_text("b.txt");
    tome.wait_for_screen("b.txt listed", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].contains("b.txt")
    });
    tome.send_keys("enter");
    tome.wait_for_text("bye", WAIT);
    tome.type_text("y");
    tome.wait_for_text("ybye", WAIT);
}

/// Picks `other` in the folder browser and opens it.
fn open_other(tome: &mut Tome) {
    open_browser(tome);
    tome.send_keys("down");
    tome.send_keys("enter");
    tome.wait_for_screen("inside other", WAIT, |screen| {
        screen[usize::from(HEADER_ROW)].contains("other")
    });
    tome.send_keys("enter");
    tome.wait_for_text_gone(SCOPE, WAIT);
}

fn read(dir: &Path, name: &str) -> String {
    fs::read_to_string(dir.join(name)).expect("read file")
}

#[test]
fn unsaved_files_ask_once_and_cancel_changes_nothing() {
    let (dir, mut tome) = edited_project();
    edit_b(&mut tome);
    open_other(&mut tome);
    // One card for both files, naming them, with its three buttons.
    tome.wait_for_text("2 files have unsaved changes", WAIT);
    tome.wait_for_text("a.txt, b.txt", WAIT);
    tome.wait_for_text(" Save all    Discard    Cancel ", WAIT);
    assert!(
        !tome
            .screen()
            .iter()
            .any(|line| line.contains("save or close unsaved files first"))
    );

    tome.type_text("c");
    tome.wait_for_text_gone("2 files have unsaved changes", WAIT);
    // Nothing changed: both edited tabs are open, nothing saved, no switch.
    assert!(tome.screen().iter().any(|line| line.contains("ybye")));
    assert!(!tome.screen().iter().any(|line| line.contains("marker.txt")));
    assert_eq!(
        read(dir.path(), "a.txt"),
        "hello
"
    );
    assert_eq!(
        read(dir.path(), "b.txt"),
        "bye
"
    );
    tome.send_keys("alt+,");
    tome.wait_for_text("xhello", WAIT);
}

#[test]
fn save_all_saves_each_file_then_switches() {
    let (dir, mut tome) = edited_project();
    edit_b(&mut tome);
    open_other(&mut tome);
    tome.wait_for_text("2 files have unsaved changes", WAIT);
    tome.send_keys("enter");
    tome.wait_for_text("marker.txt", WAIT);
    assert_eq!(
        read(dir.path(), "a.txt"),
        "xhello
"
    );
    assert_eq!(
        read(dir.path(), "b.txt"),
        "ybye
"
    );
    assert!(!tome.screen().iter().any(|line| line.contains("ybye")));
}

#[test]
fn save_all_names_an_untitled_file_through_save_as() {
    let (dir, mut tome) = edited_project();
    tome.send_keys("ctrl+n");
    tome.wait_for_text("untitled", WAIT);
    tome.type_text("fresh");
    tome.wait_for_text("fresh", WAIT);
    open_other(&mut tome);
    tome.wait_for_text("2 files have unsaved changes", WAIT);
    tome.wait_for_text("a.txt, untitled", WAIT);
    tome.type_text("s");
    tome.wait_for_text(" Save as  ", WAIT);
    tome.type_text("named.txt");
    tome.send_keys("enter");
    tome.wait_for_text("marker.txt", WAIT);
    assert_eq!(
        read(dir.path(), "a.txt"),
        "xhello
"
    );
    assert_eq!(read(dir.path(), "named.txt"), "fresh");
}

#[test]
fn cancelling_the_save_as_cancels_the_switch() {
    let (dir, mut tome) = edited_project();
    tome.send_keys("ctrl+n");
    tome.wait_for_text("untitled", WAIT);
    tome.type_text("fresh");
    tome.wait_for_text("fresh", WAIT);
    open_other(&mut tome);
    tome.wait_for_text("2 files have unsaved changes", WAIT);
    tome.type_text("s");
    tome.wait_for_text(" Save as  ", WAIT);
    tome.send_keys("esc");
    tome.wait_for_text_gone(" Save as  ", WAIT);
    // a.txt was saved before the untitled tab asked; nothing was closed and
    // the project stayed.
    assert_eq!(
        read(dir.path(), "a.txt"),
        "xhello
"
    );
    assert!(tome.screen().iter().any(|line| line.contains("fresh")));
    assert!(!tome.screen().iter().any(|line| line.contains("marker.txt")));
    tome.send_keys("alt+,");
    tome.wait_for_text("xhello", WAIT);
}

#[test]
fn discard_switches_without_saving() {
    let (dir, mut tome) = edited_project();
    edit_b(&mut tome);
    open_other(&mut tome);
    tome.wait_for_text("2 files have unsaved changes", WAIT);
    tome.type_text("d");
    tome.wait_for_text("marker.txt", WAIT);
    assert!(!tome.screen().iter().any(|line| line.contains("ybye")));
    assert_eq!(
        read(dir.path(), "a.txt"),
        "hello
"
    );
    assert_eq!(
        read(dir.path(), "b.txt"),
        "bye
"
    );
}
