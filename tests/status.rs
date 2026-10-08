mod harness;

use std::path::Path;
use std::time::Duration;

use harness::{COLS, Glyph, ROWS};
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
const PROJECT: &str = "tests/fixtures/project";
const AURORA: &str = "theme = \"aurora\"\n";
const STATUS_ROW: u16 = ROWS - 1;

// aurora's roles (design README §4.1).
const ACCENT: Color = Color::Rgb(0xb6, 0x9c, 0xff);
const ACCENT2: Color = Color::Rgb(0x6e, 0xe7, 0xd8);
const SURFACE: Color = Color::Rgb(0x0f, 0x0e, 0x16);
const ACC_INK: Color = Color::Rgb(0x12, 0x0a, 0x24);
const MUTED: Color = Color::Rgb(0x67, 0x62, 0x7d);
const TEXT: Color = Color::Rgb(0xa6, 0xa2, 0xbb);
const STRONG: Color = Color::Rgb(0xf4, 0xf2, 0xfb);
const WARN: Color = Color::Rgb(0xf0, 0xcf, 0x7a);

/// Glyph in the fixture project, with `docs/guide.md` opened through the
/// picker so its path has a directory part.
fn open_guide() -> Glyph {
    let mut glyph = Glyph::spawn_in_with_config(Path::new(PROJECT), AURORA, &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph.send_keys("ctrl+p");
    glyph.wait_for_text("cast · files · commands", WAIT);
    glyph.type_text("guide");
    glyph.wait_for_text("guide.md  docs", WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("cast · files · commands", WAIT);
    glyph.wait_for_screen("guide.md in the status bar", WAIT, |screen| {
        screen[usize::from(STATUS_ROW)].contains("guide.md")
    });
    glyph
}

#[test]
fn the_glyph_block_ramps_from_accent_to_accent2_then_fades_into_the_bar() {
    let glyph = Glyph::spawn_with_config(AURORA, &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    let status = &glyph.screen()[usize::from(STATUS_ROW)];
    assert!(status.starts_with(" ✦ glyph"), "{status:?}");
    // The old `glyph  ` text prefix is gone.
    assert!(!status.starts_with("glyph"), "{status:?}");

    // grad([accent, accent2], i/9) for i < 10, then mix(accent2, surface, (i − 9)/9).
    assert_eq!(glyph.bg_at(0, STATUS_ROW), ACCENT);
    assert_eq!(glyph.bg_at(4, STATUS_ROW), Color::Rgb(150, 189, 238));
    assert_eq!(glyph.bg_at(9, STATUS_ROW), ACCENT2);
    assert_eq!(glyph.bg_at(13, STATUS_ROW), Color::Rgb(68, 135, 130));
    assert_eq!(glyph.bg_at(17, STATUS_ROW), Color::Rgb(26, 38, 44));
    for col in 18..COLS {
        assert_eq!(glyph.bg_at(col, STATUS_ROW), SURFACE, "column {col}");
    }
    // `✦ glyph` at x = 1, in acc_ink.
    assert_eq!(glyph.text_col(STATUS_ROW, "✦ glyph"), Some(1));
    for col in [1, 3, 7] {
        assert_eq!(glyph.fg_at(col, STATUS_ROW), ACC_INK, "column {col}");
    }
}

#[test]
fn the_path_shows_its_directory_muted_and_its_name_strong() {
    let mut glyph = open_guide();
    let status = &glyph.screen()[usize::from(STATUS_ROW)];
    let slot: String = status.chars().skip(20).collect();
    // Either separator, as the platform writes it.
    assert!(
        slot.starts_with("docs/guide.md") || slot.starts_with("docs\\guide.md"),
        "{status:?}"
    );
    let name = glyph
        .text_col(STATUS_ROW, "guide.md")
        .expect("the file name");
    assert_eq!(name, 20 + 5);
    assert_eq!(glyph.fg_at(20, STATUS_ROW), MUTED);
    assert_eq!(glyph.fg_at(name - 1, STATUS_ROW), MUTED);
    assert_eq!(glyph.fg_at(name, STATUS_ROW), STRONG);
    assert!(!status.contains('•'), "{status:?}");

    // An edit adds ` •` in warn after the name.
    glyph.type_text("z");
    glyph.wait_for_screen("the dirty mark", WAIT, |screen| {
        screen[usize::from(STATUS_ROW)].contains("guide.md •")
    });
    let dot = glyph.text_col(STATUS_ROW, "•").expect("the dirty mark");
    assert_eq!(dot, name + 9);
    assert_eq!(glyph.fg_at(dot, STATUS_ROW), WARN);
}

#[test]
fn the_position_and_language_end_two_cells_from_the_right_edge() {
    let mut glyph = Glyph::spawn_with_config(AURORA, &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    // Leave the splash, so typing reaches the untitled buffer.
    glyph.send_keys("ctrl+n");
    // An untitled buffer has no language; it reads `Plain text`, 4 cells after
    // the position.
    let language = "Plain text";
    let lang_col = glyph.text_col(STATUS_ROW, language).expect("the language");
    assert_eq!(lang_col + language.len() as u16, COLS - 2);
    assert_eq!(glyph.fg_at(lang_col, STATUS_ROW), TEXT);
    let position = "Ln 1, Col 1";
    let col = glyph.text_col(STATUS_ROW, position).expect("the position");
    assert_eq!(col + position.len() as u16 + 4, lang_col);
    assert_eq!(glyph.fg_at(col, STATUS_ROW), TEXT);

    // A longer position grows to the left; the right end stays put.
    glyph.type_text("abcdefghij");
    let position = "Ln 1, Col 11";
    glyph.wait_for_text(position, WAIT);
    let col = glyph.text_col(STATUS_ROW, position).expect("the position");
    assert_eq!(col + position.len() as u16 + 4, lang_col);
    assert_eq!(
        glyph.text_col(STATUS_ROW, language),
        Some(lang_col),
        "the language stays put"
    );
}

#[test]
fn a_rust_file_names_its_language() {
    let glyph = Glyph::spawn_in_with_config(Path::new(PROJECT), AURORA, &["src/main.rs"]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph.wait_for_screen("Rust in the status bar", WAIT, |screen| {
        screen[usize::from(STATUS_ROW)].contains("Ln 1, Col 1    Rust")
    });
}
