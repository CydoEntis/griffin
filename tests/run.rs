mod harness;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use harness::{Glyph, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(10);
/// `.glyph.toml` has one entry, `dev`, which runs the `colors` script: a red
/// line, a two second pause, then a line on stderr.
const PROJECT: &str = "tests/fixtures/run-project";
/// The same `dev` entry plus `hello`.
const MULTI: &str = "tests/fixtures/run-project/multi";
/// ANSI red (SGR 31) as vt100 reports it.
const RED: vt100::Color = vt100::Color::Idx(1);

/// At 30 rows the panel is 9 tall: the editor keeps rows 0 to 19, the panel's
/// title is row 20 and its output starts on row 21, one space in.
const TITLE_ROW: u16 = 20;
const OUTPUT_ROW: u16 = 21;

/// `PATH` with the fixture folder first, so `colors` resolves to the fixture's
/// script under both `sh` and `cmd`, from any folder.
fn fixture_path() -> Vec<(&'static str, OsString)> {
    let fixture = std::path::absolute(PROJECT).expect("fixture path");
    let mut paths: Vec<PathBuf> = vec![fixture];
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    let joined = std::env::join_paths(paths).expect("PATH entries join");
    vec![("PATH", joined)]
}

fn row(glyph: &Glyph, row: u16) -> String {
    glyph.screen()[usize::from(row)].clone()
}

/// Glyph on the fixture's `main.rs`, so the project root is the fixture folder.
fn open_project() -> Glyph {
    let glyph = Glyph::spawn_in_with_env(Path::new(PROJECT), &fixture_path(), &["main.rs"]);
    glyph.wait_for_text("the editor stays visible", START);
    glyph
}

#[test]
fn f5_runs_the_only_entry_and_streams_its_output_in_colour() {
    let mut glyph = open_project();
    glyph.send_keys("f5");
    glyph.wait_for_text("red line", WAIT);
    // The red line arrived while the script is still in its pause.
    let title = row(&glyph, TITLE_ROW);
    assert!(title.starts_with(" dev · running"), "{title:?}");
    glyph.wait_for_fg(OUTPUT_ROW, RED, "red line", WAIT);
    assert_eq!(glyph.text_col(OUTPUT_ROW, "red line"), Some(1));
    // The editor above stays in view.
    assert!(row(&glyph, 2).contains("the editor stays visible"));

    // stderr comes in too, then the title shows the exit code.
    glyph.wait_for_screen("the stderr line", WAIT, |screen| {
        screen[usize::from(OUTPUT_ROW) + 1].starts_with(" second line")
    });
    glyph.wait_for_screen("the exit code in the title", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].starts_with(" dev · exited 0")
    });
}

#[test]
fn f4_toggles_the_panel_at_about_a_third_of_the_height() {
    let mut glyph = open_project();
    glyph.send_keys("f4");
    glyph.wait_for_text("F5 runs a command from .glyph.toml", WAIT);
    assert_eq!(
        glyph.text_col(OUTPUT_ROW, "F5 runs a command"),
        Some(1),
        "{:?}",
        glyph.screen()
    );
    assert!(row(&glyph, TITLE_ROW).starts_with(" run"));
    // Nothing else moved: the status line is still last.
    assert!(row(&glyph, ROWS - 1).contains("Ln 1, Col 1"));
    glyph.send_keys("f4");
    glyph.wait_for_text_gone("F5 runs a command", WAIT);

    // A run opens the panel; F4 hides its output and brings it back.
    glyph.send_keys("f5");
    glyph.wait_for_text("dev · exited 0", WAIT);
    glyph.send_keys("f4");
    glyph.wait_for_text_gone("red line", WAIT);
    glyph.wait_for_text_gone("dev · exited", WAIT);
    glyph.send_keys("f4");
    glyph.wait_for_text("red line", WAIT);
    assert!(row(&glyph, TITLE_ROW).starts_with(" dev · exited 0"));
}

#[test]
fn f5_with_several_entries_picks_one_by_name() {
    let mut glyph = Glyph::spawn_in_with_env(Path::new(MULTI), &fixture_path(), &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph.send_keys("f5");
    glyph.wait_for_text("Run:", WAIT);
    glyph.wait_for_text("hello", WAIT);
    let screen = glyph.screen().join("\n");
    assert!(screen.contains(" dev "), "{screen}");
    glyph.type_text("hel");
    glyph.wait_for_text("Run: hel", WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("Run:", WAIT);
    glyph.wait_for_text("hello from glyph", WAIT);
    glyph.wait_for_text("hello · exited 0", WAIT);
}

#[test]
fn a_malformed_project_file_says_so_in_the_status_line() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".glyph.toml"), "[[run]]\nname = = 1\n").unwrap();
    let mut glyph = Glyph::spawn_in(dir.path(), &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph.send_keys("f5");
    glyph.wait_for_screen("the error in the status line", WAIT, |screen| {
        screen[usize::from(ROWS) - 1].contains(".glyph.toml line 2")
    });
    glyph.assert_running_for(Duration::from_millis(200));
}

#[test]
fn shift_f5_stops_the_command_and_ctrl_f5_restarts_it() {
    let mut glyph = open_project();
    glyph.send_keys("f5");
    glyph.wait_for_text("red line", WAIT);
    // Stopped during the script's pause, so its stderr line never comes.
    glyph.send_keys("shift+f5");
    glyph.wait_for_screen("the stopped title", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].starts_with(" dev · stopped")
    });
    assert!(row(&glyph, OUTPUT_ROW).starts_with(" red line"));

    // Ctrl+F5 after a stop starts it again in a cleared panel.
    glyph.send_keys("ctrl+f5");
    glyph.wait_for_screen("the restarted marker", WAIT, |screen| {
        screen[usize::from(OUTPUT_ROW)].contains("restarted")
            && screen[usize::from(TITLE_ROW)].starts_with(" dev · running")
    });
    glyph.wait_for_screen("the output again under the marker", WAIT, |screen| {
        screen[usize::from(OUTPUT_ROW) + 1].starts_with(" red line")
    });
    assert!(!row(&glyph, OUTPUT_ROW + 2).contains("red line"));

    // Ctrl+F5 while it's running stops it and starts it once more.
    glyph.send_keys("ctrl+f5");
    glyph.wait_for_screen("a second restart", WAIT, |screen| {
        screen[usize::from(OUTPUT_ROW)].contains("restarted")
            && screen[usize::from(OUTPUT_ROW) + 1].starts_with(" red line")
            && screen[usize::from(TITLE_ROW)].starts_with(" dev · running")
    });
    // Only the new run finishes; the killed one sent nothing more.
    glyph.wait_for_screen("the new run's exit", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].starts_with(" dev · exited 0")
    });
    let screen = glyph.screen();
    assert_eq!(
        screen.iter().filter(|l| l.contains("second line")).count(),
        1,
        "{}",
        screen.join("\n")
    );
}

#[test]
fn f5_in_a_project_with_nothing_to_run_says_how_to_add_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut glyph = Glyph::spawn_in(dir.path(), &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph.send_keys("f5");
    glyph.wait_for_screen("the hint in the status line", WAIT, |screen| {
        screen[usize::from(ROWS) - 1].contains("add a [[run]] entry to .glyph.toml")
    });
    assert!(!glyph.screen().join("\n").contains("Run:"));
}

#[test]
fn f5_without_run_entries_offers_detected_commands() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("go.mod"), "module example.com/x\n").unwrap();
    std::fs::write(
        dir.path().join("package.json"),
        r#"{"scripts": {"dev": "vite"}}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();
    let mut glyph = Glyph::spawn_in(dir.path(), &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph.send_keys("f5");
    glyph.wait_for_text("Run:", WAIT);
    glyph.wait_for_text("pnpm run dev", WAIT);
    glyph.wait_for_text("go run .", WAIT);
    // Offered, not started: nothing runs until one is picked.
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("Run:", WAIT);
    assert!(!glyph.screen().join("\n").contains("running"));
}
