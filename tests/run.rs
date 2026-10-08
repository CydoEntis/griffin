mod harness;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use harness::{Glyph, ROWS};
use vt100::Color;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(10);
/// `.glyph.toml` has one entry, `dev`, which runs the `colors` script: a red
/// line, a two second pause, then a line on stderr.
const PROJECT: &str = "tests/fixtures/run-project";
/// The same `dev` entry plus `hello`.
const MULTI: &str = "tests/fixtures/run-project/multi";
/// Runs the `palette` script, which prints the eight ANSI colours.
const COLOURS: &str = "tests/fixtures/run-project/colours";

/// The default theme's (hydra's) roles the panel maps states and ANSI colours to.
const ERR: Color = Color::Rgb(0xff, 0x6b, 0x6b);
const OK: Color = Color::Rgb(0x7f, 0xd9, 0x62);
const WARN: Color = Color::Rgb(0xff, 0xb5, 0x47);
const MUTED: Color = Color::Rgb(0x71, 0x80, 0x8f);
const STRONG: Color = Color::Rgb(0xf2, 0xf6, 0xf8);
const SURFACE: Color = Color::Rgb(0x0c, 0x13, 0x1b);
const BG: Color = Color::Rgb(0x07, 0x0b, 0x10);
/// hydra's `syn.function`, `syn.keyword` and `syn.type`.
const FUNCTION: Color = Color::Rgb(0x5a, 0xa9, 0xff);
const KEYWORD: Color = Color::Rgb(0xa5, 0x93, 0xff);
const TYPE: Color = Color::Rgb(0x3d, 0xd6, 0xc0);

/// The hints at the right of the title row while running, and once done, with
/// the blank that ends the row.
const RUNNING_HINTS: &str = "shift+F5 stop   ctrl+F5 restart   F4 hide ";
const DONE_HINTS: &str = "F5 run again   F4 hide ";

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
    assert!(title.starts_with(" ● dev  running   colors"), "{title:?}");
    // The title on `surface`: `●` and the state word in `warn`, the name in
    // `strong`, the command in `muted`, the running hints ending a cell short of
    // the right edge.
    assert_eq!(glyph.bg_at(0, TITLE_ROW), SURFACE);
    assert_eq!(glyph.fg_at(1, TITLE_ROW), WARN);
    assert_eq!(glyph.fg_at(3, TITLE_ROW), STRONG);
    assert_eq!(glyph.fg_at(8, TITLE_ROW), WARN);
    assert_eq!(glyph.fg_at(18, TITLE_ROW), MUTED);
    assert_hints(&glyph, RUNNING_HINTS);
    // Red output takes the theme's `err`, on `bg`.
    glyph.wait_for_fg(OUTPUT_ROW, ERR, "red line", WAIT);
    assert_eq!(glyph.bg_at(1, OUTPUT_ROW), BG);
    assert_eq!(glyph.text_col(OUTPUT_ROW, "red line"), Some(1));
    // The editor above stays in view.
    assert!(row(&glyph, 4).contains("the editor stays visible"));

    // stderr comes in too, then the title shows the exit code.
    glyph.wait_for_screen("the stderr line", WAIT, |screen| {
        screen[usize::from(OUTPUT_ROW) + 1].starts_with(" second line")
    });
    glyph.wait_for_screen("the exit code in the title", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].starts_with(" ✓ dev  exited 0")
    });
    assert_eq!(glyph.fg_at(1, TITLE_ROW), OK);
    assert_eq!(glyph.fg_at(8, TITLE_ROW), OK);
    assert_hints(&glyph, DONE_HINTS);
}

/// The title row ends with `hints` and a blank, in `muted`.
fn assert_hints(glyph: &Glyph, hints: &str) {
    let at = harness::COLS - u16::try_from(hints.chars().count()).expect("short hints");
    let title = row(glyph, TITLE_ROW);
    assert_eq!(
        glyph.text_col(TITLE_ROW, hints.trim_end()),
        Some(at),
        "{title:?}"
    );
    assert_eq!(glyph.fg_at(at, TITLE_ROW), MUTED);
}

/// The first output row is the restart rule, `── restarted HH:MM:SS ──…` in
/// `muted`, one cell in.
fn assert_restart_rule(glyph: &Glyph) {
    let line = row(glyph, OUTPUT_ROW);
    let rule: Vec<char> = line.chars().collect();
    let text: String = rule.iter().take(14).collect();
    assert_eq!(text, " ── restarted ", "{line:?}");
    let time: String = rule.iter().skip(14).take(8).collect();
    let digits = time.chars().enumerate().all(|(i, c)| {
        if i % 3 == 2 {
            c == ':'
        } else {
            c.is_ascii_digit()
        }
    });
    assert!(digits, "not HH:MM:SS: {line:?}");
    let tail: String = rule.iter().skip(22).take(3).collect();
    assert_eq!(tail, " ──", "{line:?}");
    assert_eq!(glyph.fg_at(5, OUTPUT_ROW), MUTED);
    assert_eq!(glyph.fg_at(14, OUTPUT_ROW), MUTED);
}

#[test]
fn ansi_colours_take_the_theme_roles() {
    let mut glyph = Glyph::spawn_in_with_env(Path::new(COLOURS), &fixture_path(), &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph.send_keys("f5");
    glyph.wait_for_text("palette  exited 0", WAIT);
    let first = OUTPUT_ROW;
    let second = OUTPUT_ROW + 1;
    assert!(row(&glyph, first).starts_with(" red green yellow blue"));
    assert!(row(&glyph, second).starts_with(" magenta cyan white black"));
    for (row, text, color) in [
        (first, "red", ERR),
        (first, "green", OK),
        (first, "yellow", WARN),
        (first, "blue", FUNCTION),
        (second, "magenta", KEYWORD),
        (second, "cyan", TYPE),
        (second, "white", STRONG),
        (second, "black", MUTED),
    ] {
        assert_eq!(
            glyph.fg_text(row, color).trim(),
            text,
            "row {row}, {color:?}"
        );
    }
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
    glyph.wait_for_text("dev  exited 0", WAIT);
    glyph.send_keys("f4");
    glyph.wait_for_text_gone("red line", WAIT);
    glyph.wait_for_text_gone("dev  exited", WAIT);
    glyph.send_keys("f4");
    glyph.wait_for_text("red line", WAIT);
    assert!(row(&glyph, TITLE_ROW).starts_with(" ✓ dev  exited 0"));
}

#[test]
fn f5_with_several_entries_picks_one_by_name() {
    let mut glyph = Glyph::spawn_in_with_env(Path::new(MULTI), &fixture_path(), &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph.send_keys("f5");
    glyph.wait_for_text("✦ Run", WAIT);
    glyph.wait_for_text("hello", WAIT);
    // The 60x8 card at (20, 11): the lit edge, the query, then a row per entry
    // with its command at x+14 and its source right-aligned, in `muted` off the
    // selected (glowing) first row.
    assert_eq!(glyph.text_col(11, "▀▀▀"), Some(20));
    assert_eq!(glyph.text_col(12, "✦ Run"), Some(22));
    assert_eq!(glyph.text_col(13, "dev"), Some(22));
    assert_eq!(glyph.text_col(13, "colors"), Some(34));
    assert_eq!(glyph.text_col(13, ".glyph.toml  ⏎"), Some(62));
    assert_eq!(glyph.text_col(14, "hello"), Some(22));
    assert_eq!(glyph.text_col(14, "echo hello from glyph"), Some(34));
    assert_eq!(glyph.fg_at(34, 14), MUTED);
    assert_eq!(glyph.fg_at(62, 14), MUTED);
    assert!(row(&glyph, 18).contains("esc close"));
    glyph.type_text("hel");
    glyph.wait_for_text("✦ Run  hel", WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("✦ Run", WAIT);
    glyph.wait_for_text("hello from glyph", WAIT);
    glyph.wait_for_text("hello  exited 0", WAIT);
}

#[test]
fn mono_dims_behind_the_run_picker_and_reverses_the_selected_row() {
    let mut glyph = Glyph::spawn_in_with_config_and_env(
        Path::new(MULTI),
        "theme = \"mono\"\n",
        &fixture_path(),
        &[],
    );
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph.send_keys("f5");
    glyph.wait_for_text("✦ Run", WAIT);
    // The whole first row of the 60-wide card at column 20.
    let selected = format!("  dev{:9}colors{:22}.glyph.toml  ⏎    ", "", "");
    glyph.wait_for_reversed(13, &selected, WAIT);
    assert!(!glyph.reversed_text(14).contains("hello"));
    let status = ROWS - 1;
    let position = glyph.text_col(status, "Ln 1").expect("the position");
    assert!(glyph.dim_at(position, status), "the status line is dimmed");
    assert!(!glyph.dim_at(22, 14), "the card isn't");
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
        screen[usize::from(TITLE_ROW)].starts_with(" ■ dev  stopped")
    });
    assert_eq!(glyph.fg_at(1, TITLE_ROW), MUTED);
    assert_hints(&glyph, DONE_HINTS);
    assert!(row(&glyph, OUTPUT_ROW).starts_with(" red line"));

    // Ctrl+F5 after a stop starts it again in a cleared panel.
    glyph.send_keys("ctrl+f5");
    glyph.wait_for_screen("the restarted marker", WAIT, |screen| {
        screen[usize::from(OUTPUT_ROW)].contains("restarted")
            && screen[usize::from(TITLE_ROW)].starts_with(" ● dev  running")
    });
    assert_restart_rule(&glyph);
    glyph.wait_for_screen("the output again under the marker", WAIT, |screen| {
        screen[usize::from(OUTPUT_ROW) + 1].starts_with(" red line")
    });
    assert!(!row(&glyph, OUTPUT_ROW + 2).contains("red line"));

    // Ctrl+F5 while it's running stops it and starts it once more.
    glyph.send_keys("ctrl+f5");
    glyph.wait_for_screen("a second restart", WAIT, |screen| {
        screen[usize::from(OUTPUT_ROW)].contains("restarted")
            && screen[usize::from(OUTPUT_ROW) + 1].starts_with(" red line")
            && screen[usize::from(TITLE_ROW)].starts_with(" ● dev  running")
    });
    // Only the new run finishes; the killed one sent nothing more.
    glyph.wait_for_screen("the new run's exit", WAIT, |screen| {
        screen[usize::from(TITLE_ROW)].starts_with(" ✓ dev  exited 0")
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
    assert!(!glyph.screen().join("\n").contains("✦ Run"));
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
    glyph.wait_for_text("✦ Run", WAIT);
    glyph.wait_for_text("pnpm run dev", WAIT);
    glyph.wait_for_text("go run .", WAIT);
    // Each says it was detected, at the right of its row.
    let screen = glyph.screen();
    let pnpm = screen
        .iter()
        .find(|line| line.contains("pnpm run dev"))
        .expect("the pnpm row");
    assert!(pnpm.contains("detected  ⏎"), "{pnpm:?}");
    // Offered, not started: nothing runs until one is picked.
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("✦ Run", WAIT);
    assert!(!glyph.screen().join("\n").contains("running"));
}
