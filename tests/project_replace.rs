mod harness;

use std::fs;
use std::path::Path;
use std::time::Duration;

use harness::{Glyph, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(5);
/// `notes.txt` has `TODO` on line 1 and `todo`, `TODO` on line 3; `src/app.rs` a
/// `// TODO` on line 2; `README.md` none. Four matches in two files.
const FIXTURE: &str = "tests/fixtures/project_replace";

/// At 100x30 the panel's query line is row 6; its right half, from column 50,
/// holds the Replace field.
const QUERY_ROW: u16 = 6;
const REPLACE_X: u16 = 50;

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            fs::create_dir(&target)?;
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// A temp copy of the fixture, so replacing never touches the real one.
fn fixture_copy() -> std::io::Result<tempfile::TempDir> {
    let dir = tempfile::tempdir()?;
    copy_dir(Path::new(FIXTURE), dir.path())?;
    Ok(dir)
}

/// Glyph in `dir` with no file open, so the root is that folder.
fn open_in(dir: &Path) -> Glyph {
    let glyph = Glyph::spawn_in(dir, &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph
}

/// Waits for the panel's query line to end with `status`, e.g. `3 hits`.
fn wait_for_status(glyph: &Glyph, status: &str) {
    glyph.wait_for_screen(&format!("the panel to say {status:?}"), WAIT, |lines| {
        lines[usize::from(QUERY_ROW)].contains(&format!("  {status} │"))
    });
}

/// Waits for the status line at the bottom to show `message`.
fn wait_for_message(glyph: &Glyph, message: &str) {
    glyph.wait_for_screen(
        &format!("the status line to say {message:?}"),
        WAIT,
        |lines| lines[usize::from(ROWS - 1)].contains(message),
    );
}

/// Opens the panel and searches for `query`.
fn search(glyph: &mut Glyph, query: &str) {
    glyph.send_keys("alt+f");
    glyph.wait_for_text("Search:", WAIT);
    glyph.type_text(query);
    glyph.wait_for_text(&format!("Search: {query}"), WAIT);
    glyph.send_keys("enter");
}

/// Searches for `todo`, Tab, `done` in the Replace field, then Alt+A.
fn replace_todo(glyph: &mut Glyph) {
    search(glyph, "todo");
    wait_for_status(glyph, "3 hits");
    // A key and quick typing after it can arrive as one paste on Windows, so wait
    // for each key to land before the next.
    glyph.send_keys("tab");
    glyph.wait_for_cursor(REPLACE_X + 9, QUERY_ROW, WAIT);
    glyph.type_text("done");
    glyph.wait_for_text("Replace: done", WAIT);
    glyph.send_keys("alt+a");
    glyph.wait_for_text("Replace 4 matches in 2 files? y / n", WAIT);
}

#[test]
fn alt_enter_asks_then_replaces_in_every_listed_file_and_searches_again() -> std::io::Result<()> {
    let dir = fixture_copy()?;
    let notes = dir.path().join("notes.txt");
    let code = dir.path().join("src").join("app.rs");
    let notes_before = fs::read_to_string(&notes)?;
    let code_before = fs::read_to_string(&code)?;
    let mut glyph = open_in(dir.path());
    search(&mut glyph, "todo");
    wait_for_status(&glyph, "3 hits");
    assert_eq!(glyph.text_col(QUERY_ROW, "Replace: "), Some(REPLACE_X));
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("Search:", WAIT);

    replace_todo(&mut glyph);
    glyph.type_text("y");
    wait_for_message(&glyph, "Replaced 4 in 2 files");
    glyph.wait_for_text_gone("Replace 4 matches", WAIT);
    // The list refreshed: nothing matches any more.
    wait_for_status(&glyph, "no hits");
    glyph.wait_for_text_gone("notes.txt:", WAIT);

    // Saved with the line endings each file had.
    let done = |text: &str| text.replace("TODO", "done").replace("todo", "done");
    assert_eq!(fs::read_to_string(&notes)?, done(&notes_before));
    assert_eq!(fs::read_to_string(&code)?, done(&code_before));
    Ok(())
}

#[test]
fn n_closes_the_prompt_and_changes_nothing() -> std::io::Result<()> {
    let dir = fixture_copy()?;
    let notes = dir.path().join("notes.txt");
    let before = fs::read(&notes)?;
    let mut glyph = open_in(dir.path());
    replace_todo(&mut glyph);
    glyph.type_text("n");
    glyph.wait_for_text_gone("Replace 4 matches", WAIT);
    wait_for_status(&glyph, "3 hits");
    assert_eq!(fs::read(&notes)?, before);
    Ok(())
}

#[test]
fn an_open_buffer_is_edited_in_place_and_left_unsaved() -> std::io::Result<()> {
    let dir = fixture_copy()?;
    let code = dir.path().join("src").join("app.rs");
    let code_before = fs::read(&code)?;
    let mut glyph = open_in(dir.path());
    // Open `src/app.rs` from its hit, the third.
    search(&mut glyph, "todo");
    wait_for_status(&glyph, "3 hits");
    glyph.send_keys("down");
    glyph.send_keys("down");
    glyph.wait_for_reversed(
        QUERY_ROW + 3,
        &format!(" {:<77}", "src/app.rs:2: // TODO tidy"),
        WAIT,
    );
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("Search:", WAIT);
    glyph.wait_for_text("2 │     // TODO tidy", WAIT);

    replace_todo(&mut glyph);
    glyph.type_text("y");
    wait_for_message(&glyph, "Replaced 4 in 2 files");
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("Search:", WAIT);
    glyph.wait_for_text("2 │     // done tidy", WAIT);
    glyph.wait_for_text("app.rs ●", WAIT);
    assert_eq!(fs::read(&code)?, code_before);

    // One undo takes it all back.
    glyph.send_keys("ctrl+z");
    glyph.wait_for_text("2 │     // TODO tidy", WAIT);
    Ok(())
}

#[test]
fn a_file_that_cannot_be_written_is_reported_and_the_others_still_go() -> std::io::Result<()> {
    let dir = fixture_copy()?;
    let notes = dir.path().join("notes.txt");
    let code = dir.path().join("src").join("app.rs");
    let notes_before = fs::read(&notes)?;
    let mut perms = fs::metadata(&notes)?.permissions();
    perms.set_readonly(true);
    fs::set_permissions(&notes, perms.clone())?;

    let mut glyph = open_in(dir.path());
    replace_todo(&mut glyph);
    glyph.type_text("y");
    wait_for_message(
        &glyph,
        "Replaced 1 in 1 file · cannot write notes.txt: read-only",
    );
    // Only the file that failed still has matches.
    wait_for_status(&glyph, "2 hits");
    let notes_after = fs::read(&notes)?;
    // tempdir can't delete a read-only file on Windows.
    #[expect(
        clippy::permissions_set_readonly_false,
        reason = "only to let the temp dir clean up"
    )]
    perms.set_readonly(false);
    fs::set_permissions(&notes, perms)?;
    assert_eq!(notes_after, notes_before);
    assert!(fs::read_to_string(&code)?.contains("// done tidy"));
    Ok(())
}
