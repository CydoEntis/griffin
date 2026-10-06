mod harness;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use harness::{Griffin, ROWS};

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(10);
/// `.griffin.toml` has one entry, `dev`, which runs the `colors` script: a red
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

fn row(griffin: &Griffin, row: u16) -> String {
    griffin.screen()[usize::from(row)].clone()
}

/// Griffin on the fixture's `main.rs`, so the project root is the fixture folder.
fn open_project() -> Griffin {
    let griffin = Griffin::spawn_in_with_env(Path::new(PROJECT), &fixture_path(), &["main.rs"]);
    griffin.wait_for_text("the editor stays visible", START);
    griffin
}

#[test]
fn f5_runs_the_only_entry_and_streams_its_output_in_colour() {
    let mut griffin = open_project();
    griffin.send_keys("f5");
    griffin.wait_for_text("red line", WAIT);
    // The red line arrived while the script is still in its pause.
    let title = row(&griffin, TITLE_ROW);
    assert!(title.starts_with(" dev · running"), "{title:?}");
    griffin.wait_for_fg(OUTPUT_ROW, RED, "red line", WAIT);
    assert_eq!(griffin.text_col(OUTPUT_ROW, "red line"), Some(1));
    // The editor above stays in view.
    assert!(row(&griffin, 2).contains("the editor stays visible"));

    // stderr comes in too, then the title shows the exit code.
    griffin.wait_for_screen("the stderr line", WAIT, |screen| {
        screen[usize::from(OUTPUT_ROW) + 1].starts_with(" second line")
    });
    griffin.wait_for_screen("the exit code in the title", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].starts_with(" dev · exited 0")
    });
}

#[test]
fn f4_toggles_the_panel_at_about_a_third_of_the_height() {
    let mut griffin = open_project();
    griffin.send_keys("f4");
    griffin.wait_for_text("F5 runs a command from .griffin.toml", WAIT);
    assert_eq!(
        griffin.text_col(OUTPUT_ROW, "F5 runs a command"),
        Some(1),
        "{:?}",
        griffin.screen()
    );
    assert!(row(&griffin, TITLE_ROW).starts_with(" run"));
    // Nothing else moved: the status line is still last.
    assert!(row(&griffin, ROWS - 1).contains("Ln 1, Col 1"));
    griffin.send_keys("f4");
    griffin.wait_for_text_gone("F5 runs a command", WAIT);

    // A run opens the panel; F4 hides its output and brings it back.
    griffin.send_keys("f5");
    griffin.wait_for_text("dev · exited 0", WAIT);
    griffin.send_keys("f4");
    griffin.wait_for_text_gone("red line", WAIT);
    griffin.wait_for_text_gone("dev · exited", WAIT);
    griffin.send_keys("f4");
    griffin.wait_for_text("red line", WAIT);
    assert!(row(&griffin, TITLE_ROW).starts_with(" dev · exited 0"));
}

#[test]
fn f5_with_several_entries_picks_one_by_name() {
    let mut griffin = Griffin::spawn_in_with_env(Path::new(MULTI), &fixture_path(), &[]);
    griffin.wait_for_text("Ln 1, Col 1", START);
    griffin.send_keys("f5");
    griffin.wait_for_text("Run:", WAIT);
    griffin.wait_for_text("hello", WAIT);
    let screen = griffin.screen().join("\n");
    assert!(screen.contains(" dev "), "{screen}");
    griffin.type_text("hel");
    griffin.wait_for_text("Run: hel", WAIT);
    griffin.send_keys("enter");
    griffin.wait_for_text_gone("Run:", WAIT);
    griffin.wait_for_text("hello from griffin", WAIT);
    griffin.wait_for_text("hello · exited 0", WAIT);
}

#[test]
fn a_malformed_project_file_says_so_in_the_status_line() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".griffin.toml"), "[[run]]\nname = = 1\n").unwrap();
    let mut griffin = Griffin::spawn_in(dir.path(), &[]);
    griffin.wait_for_text("Ln 1, Col 1", START);
    griffin.send_keys("f5");
    griffin.wait_for_screen("the error in the status line", WAIT, |screen| {
        screen[usize::from(ROWS) - 1].contains(".griffin.toml line 2")
    });
    griffin.assert_running_for(Duration::from_millis(200));
}

#[test]
fn shift_f5_stops_the_command_and_ctrl_f5_restarts_it() {
    let mut griffin = open_project();
    griffin.send_keys("f5");
    griffin.wait_for_text("red line", WAIT);
    // Stopped during the script's pause, so its stderr line never comes.
    griffin.send_keys("shift+f5");
    griffin.wait_for_screen("the stopped title", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].starts_with(" dev · stopped")
    });
    assert!(row(&griffin, OUTPUT_ROW).starts_with(" red line"));

    // Ctrl+F5 after a stop starts it again in a cleared panel.
    griffin.send_keys("ctrl+f5");
    griffin.wait_for_screen("the restarted marker", WAIT, |screen| {
        screen[usize::from(OUTPUT_ROW)].contains("restarted")
            && screen[usize::from(TITLE_ROW)].starts_with(" dev · running")
    });
    griffin.wait_for_screen("the output again under the marker", WAIT, |screen| {
        screen[usize::from(OUTPUT_ROW) + 1].starts_with(" red line")
    });
    assert!(!row(&griffin, OUTPUT_ROW + 2).contains("red line"));

    // Ctrl+F5 while it's running stops it and starts it once more.
    griffin.send_keys("ctrl+f5");
    griffin.wait_for_screen("a second restart", WAIT, |screen| {
        screen[usize::from(OUTPUT_ROW)].contains("restarted")
            && screen[usize::from(OUTPUT_ROW) + 1].starts_with(" red line")
            && screen[usize::from(TITLE_ROW)].starts_with(" dev · running")
    });
    // Only the new run finishes; the killed one sent nothing more.
    griffin.wait_for_screen("the new run's exit", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].starts_with(" dev · exited 0")
    });
    let screen = griffin.screen();
    assert_eq!(
        screen.iter().filter(|l| l.contains("second line")).count(),
        1,
        "{}",
        screen.join("\n")
    );
}
