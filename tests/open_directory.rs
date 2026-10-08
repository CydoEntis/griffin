//! `>open directory`: the folder browser, and opening the folder it picks as the
//! project (glyph-splash spec S6–S9).

mod harness;

use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{Glyph, ROWS};
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

fn row(glyph: &Glyph, row: u16) -> String {
    glyph.screen()[usize::from(row)].clone()
}

/// Glyph on `dir` as the project, tree showing.
fn open_in(dir: &Path, config: &str) -> Glyph {
    let glyph = Glyph::spawn_in_with_config(dir, config, &["."]);
    glyph.wait_for_text("✦ glyph", START);
    glyph
}

/// Ctrl+P, `>open directory`, Enter.
fn open_browser(glyph: &mut Glyph) {
    glyph.send_keys("ctrl+p");
    glyph.wait_for_text("cast · files · commands", WAIT);
    glyph.type_text(">open directory");
    glyph.wait_for_text("cast · commands", WAIT);
    glyph.wait_for_screen("Open directory listed", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].contains("Open directory")
    });
    glyph.send_keys("enter");
    glyph.wait_for_text(SCOPE, WAIT);
}

#[test]
fn open_directory_in_the_cast_opens_the_folder_browser() {
    let dir = folder(&["alpha", "Beta", ".hidden"], &[("notes.txt", "")]);
    let mut glyph = open_in(dir.path(), "");
    open_browser(&mut glyph);
    let here = name(dir.path());

    // The lit edge across the cast's card.
    assert_eq!(
        row(&glyph, CARD_Y).chars().nth(usize::from(CARD_X)),
        Some('▀')
    );
    // `✦` and the folder, `open · folders` at the right.
    assert_eq!(glyph.text_col(HEADER_ROW, "✦ "), Some(TEXT_X));
    let header = row(&glyph, HEADER_ROW);
    assert!(header.contains(&here), "{header:?}");
    assert_eq!(
        glyph.text_col(HEADER_ROW, SCOPE),
        Some(RIGHT - u16::try_from(SCOPE.chars().count()).unwrap())
    );
    assert_eq!(row(&glyph, RULE_ROW).chars().nth(9), Some('─'));
    // The open row first, selected: the glow and `⏎` at the right.
    assert_eq!(glyph.text_col(OPEN_ROW, "⏎ open "), Some(TEXT_X));
    assert!(row(&glyph, OPEN_ROW).contains(&here));
    assert_eq!(glyph.text_col(OPEN_ROW, "⏎    "), Some(RIGHT - 1));
    assert_eq!(glyph.bg_at(CARD_X, OPEN_ROW), mix(RAISED, ACCENT, 0.3));
    // Folders only, ignoring case, dot-folders last.
    assert_eq!(glyph.text_col(FIRST_ROW, "▸ alpha"), Some(TEXT_X));
    assert_eq!(glyph.text_col(FIRST_ROW + 1, "▸ Beta"), Some(TEXT_X));
    assert_eq!(glyph.text_col(FIRST_ROW + 2, "▸ .hidden"), Some(TEXT_X));
    // A blank, then the footer.
    assert_eq!(row(&glyph, FIRST_ROW + 3).trim(), "");
    assert_eq!(glyph.text_col(FIRST_ROW + 4, FOOTER), Some(TEXT_X));
    assert!(
        !glyph.screen()[..usize::from(ROWS - 1)]
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
    let mut glyph = open_in(dir.path(), "");
    open_browser(&mut glyph);
    // Ten rows, then a blank and the footer.
    assert_eq!(glyph.text_col(FIRST_ROW + 9, "▸ dir09"), Some(TEXT_X));
    assert_eq!(glyph.text_col(FIRST_ROW + 11, FOOTER), Some(TEXT_X));
    for _ in 0..14 {
        glyph.send_keys("down");
    }
    glyph.wait_for_screen("the last folder on the last row", WAIT, |screen| {
        screen[usize::from(FIRST_ROW + 9)].contains("▸ dir13")
    });
    assert!(row(&glyph, FIRST_ROW).contains("▸ dir04"));
}

#[test]
fn mono_reverses_the_selected_row() {
    let dir = folder(&["alpha"], &[]);
    let mut glyph = open_in(dir.path(), "theme = \"mono\"\n");
    open_browser(&mut glyph);
    let selected = glyph.reversed_text(OPEN_ROW);
    assert_eq!(selected.chars().count(), 86, "{selected:?}");
    assert!(selected.starts_with("    ⏎ open "), "{selected:?}");
    glyph.send_keys("down");
    // The whole card's width, `⏎` four cells in from its right edge.
    glyph.wait_for_reversed(FIRST_ROW, &format!("{:<81}⏎    ", "    ▸ alpha"), WAIT);
    assert_eq!(glyph.reversed_text(OPEN_ROW), "");
}

#[test]
fn the_browser_goes_in_and_up_filters_and_closes() {
    let dir = folder(&["alpha/inner", "Beta", ".hidden"], &[]);
    let here = name(dir.path());
    let mut glyph = open_in(dir.path(), "");
    open_browser(&mut glyph);

    // Enter on a folder goes into it.
    glyph.send_keys("down");
    glyph.send_keys("enter");
    glyph.wait_for_screen("inside alpha", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].contains("▸ inner")
    });
    assert!(row(&glyph, HEADER_ROW).contains("alpha"));
    // ← goes back up, with the folder just left selected.
    glyph.send_keys("left");
    glyph.wait_for_screen("back in the project", WAIT, |screen| {
        screen[usize::from(FIRST_ROW + 1)].contains("▸ Beta")
    });
    glyph.wait_for_bg(CARD_X, FIRST_ROW, mix(RAISED, ACCENT, 0.3), WAIT);

    // Typing filters the folders; the open row stays.
    glyph.type_text("bet");
    glyph.wait_for_screen("only Beta", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].contains("▸ Beta")
            && !screen[usize::from(FIRST_ROW + 1)].contains("▸")
    });
    assert_eq!(glyph.text_col(OPEN_ROW, "⏎ open "), Some(TEXT_X));
    // Backspace edits the query while there is one, then goes up.
    for _ in 0..3 {
        glyph.send_keys("backspace");
    }
    glyph.wait_for_screen("every folder again", WAIT, |screen| {
        screen[usize::from(FIRST_ROW + 2)].contains("▸ .hidden")
    });
    glyph.send_keys("backspace");
    glyph.wait_for_screen("the parent folder", WAIT, |screen| {
        !screen[usize::from(HEADER_ROW)].contains(&here)
    });

    // Esc closes it.
    glyph.send_keys("esc");
    glyph.wait_for_text_gone(SCOPE, WAIT);
    // So does a click outside the card, changing nothing.
    open_browser(&mut glyph);
    glyph.click(2, ROWS - 3);
    glyph.wait_for_text_gone(SCOPE, WAIT);
    let brand: String = row(&glyph, 1).chars().take(28).collect();
    assert!(
        brand.contains(&here[here.len().saturating_sub(6)..]),
        "{brand:?}"
    );
}

#[test]
fn a_folder_that_cannot_be_read_says_why_and_the_browser_stays() {
    let dir = folder(&["gone", "kept"], &[]);
    let mut glyph = open_in(dir.path(), "");
    open_browser(&mut glyph);
    fs::remove_dir(dir.path().join("gone")).expect("remove folder");
    glyph.send_keys("down");
    glyph.send_keys("enter");
    glyph.wait_for_screen("the reason in the status line", WAIT, |screen| {
        screen[usize::from(ROWS - 1)].contains("cannot open")
    });
    assert!(row(&glyph, ROWS - 1).contains("gone"));
    assert!(row(&glyph, HEADER_ROW).contains(SCOPE));
    assert!(row(&glyph, FIRST_ROW).contains("▸ gone"));
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
    let old = folder(&[], &[("old-file.txt", ""), (".glyph.toml", &old_toml)]);
    let new_toml = "[[run]]\nname = \"hello\"\ncommand = \"echo switched-ok\"\n";
    let new = folder(&["sub"], &[("new-file.txt", ""), (".glyph.toml", new_toml)]);
    let mut glyph = open_in(old.path(), "");
    glyph.wait_for_text("old-file.txt", WAIT);
    glyph.send_keys("f5");
    glyph.wait_for_screen("the old command running", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].contains("wait  running")
    });

    // A typed path jumps there; Enter on the open row opens it.
    open_browser(&mut glyph);
    glyph.type_text(&new.path().display().to_string());
    glyph.send_keys("enter");
    glyph.wait_for_screen("the new folder in the browser", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].contains("▸ sub")
    });
    glyph.send_keys("enter");
    glyph.wait_for_text_gone(SCOPE, WAIT);

    // The tree and brand show the new folder beside the splash, as
    // `glyph <folder>` starts; the run was stopped.
    glyph.wait_for_text("new-file.txt", WAIT);
    assert!(
        !glyph
            .screen()
            .iter()
            .any(|line| line.contains("old-file.txt"))
    );
    let brand: String = row(&glyph, 1).chars().take(28).collect();
    let new_name = name(new.path());
    assert!(
        brand.contains(&new_name[new_name.len().saturating_sub(6)..]),
        "{brand:?}"
    );
    glyph.wait_for_text("Open directory", WAIT);
    assert_ne!(
        glyph.bg_at(0, 3),
        TREE_GLOW,
        "the splash has focus, not the tree"
    );
    glyph.wait_for_screen("the old command stopped", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].contains("wait  stopped")
    });

    // Ctrl+P lists the new folder's files.
    glyph.send_keys("ctrl+p");
    glyph.wait_for_text("cast · files · commands", WAIT);
    glyph.wait_for_text("new-file.txt", WAIT);
    glyph.type_text("file");
    glyph.wait_for_screen("only the new file", WAIT, |screen| {
        screen[usize::from(FIRST_ROW)].contains("new-file.txt")
    });
    assert!(
        !glyph
            .screen()
            .iter()
            .any(|line| line.contains("old-file.txt"))
    );
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("cast · files · commands", WAIT);

    // The old run isn't restarted in the new folder: Ctrl+F5 is F5 again,
    // which runs the new folder's `.glyph.toml`.
    glyph.send_keys("ctrl+f5");
    glyph.wait_for_screen("the new command's output", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].contains("hello")
            && screen[usize::from(TITLE_ROW + 1)].starts_with(" switched-ok")
    });
}

#[test]
fn unsaved_files_stop_the_switch() {
    let dir = folder(&["other"], &[("a.txt", "hello\n")]);
    let mut glyph = Glyph::spawn_in(dir.path(), &["a.txt"]);
    glyph.wait_for_text("hello", START);
    glyph.type_text("x");
    glyph.wait_for_text("xhello", WAIT);
    open_browser(&mut glyph);
    glyph.send_keys("down");
    glyph.send_keys("enter");
    glyph.wait_for_screen("inside other", WAIT, |screen| {
        screen[usize::from(HEADER_ROW)].contains("other")
    });
    glyph.send_keys("enter");
    glyph.wait_for_text("save or close unsaved files first", WAIT);
    // Nothing changed: the edited tab is still open.
    assert!(glyph.screen().iter().any(|line| line.contains("xhello")));
    glyph.wait_for_text_gone(SCOPE, WAIT);
}
