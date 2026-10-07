//! The language server client against `fake_lsp`, the scripted server in
//! `src/bin/fake_lsp.rs`, driven through the real binary.

mod harness;

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use harness::{Griffin, ROWS};
use serde_json::Value;
use tempfile::TempDir;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(10);

/// `config.toml` pointing Rust files at `command`. A literal string, so Windows
/// backslashes stay as they are.
fn rust_server(command: &str) -> String {
    format!("[lsp.rust]\ncommand = '{command}'\n")
}

fn fake() -> &'static str {
    env!("CARGO_BIN_EXE_fake_lsp")
}

fn status_line(griffin: &Griffin) -> String {
    griffin.screen()[usize::from(ROWS - 1)].clone()
}

/// A project holding `a.rs` and `b.rs`, with the fake's log (and script, if any)
/// outside it so the tree never shows them.
struct Project {
    dir: TempDir,
    files: TempDir,
}

impl Project {
    fn new(script: Option<&str>) -> Self {
        let dir = tempfile::tempdir().expect("create project dir");
        fs::write(dir.path().join("a.rs"), "fn a() {}\n").expect("write a.rs");
        fs::write(dir.path().join("b.rs"), "fn b() {}\n").expect("write b.rs");
        let files = tempfile::tempdir().expect("create log dir");
        if let Some(script) = script {
            fs::write(files.path().join("script.json"), script).expect("write script");
        }
        Self { dir, files }
    }

    fn log_path(&self) -> PathBuf {
        self.files.path().join("log.jsonl")
    }

    fn env(&self) -> Vec<(&'static str, OsString)> {
        let mut env = vec![("FAKE_LSP_LOG", self.log_path().into_os_string())];
        let script = self.files.path().join("script.json");
        if script.exists() {
            env.push(("FAKE_LSP_SCRIPT", script.into_os_string()));
        }
        env
    }

    fn open(&self, config: &str, file: &str) -> Griffin {
        let griffin =
            Griffin::spawn_in_with_config_and_env(self.dir.path(), config, &self.env(), &[file]);
        griffin.wait_for_text("Ln 1, Col 1", START);
        griffin
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.dir.path().join(name)).expect("read project file")
    }

    /// Every logged message, as `(pid, message)`.
    fn log(&self) -> Vec<(u64, Value)> {
        let Ok(text) = fs::read_to_string(self.log_path()) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .map(|entry| (entry["pid"].as_u64().unwrap_or(0), entry["message"].clone()))
            .collect()
    }

    /// Whether the fake has received `method` for a URI ending in `/<file>`.
    fn received(&self, method: &str, file: &str) -> bool {
        self.log()
            .iter()
            .any(|(_, message)| is(message, method, file))
    }

    fn wait_for(&self, griffin: &Griffin, method: &str, file: &str) {
        griffin.wait_for_files(&format!("{method} for {file}"), WAIT, || {
            self.received(method, file)
        });
    }
}

fn is(message: &Value, method: &str, file: &str) -> bool {
    message["method"] == method
        && message["params"]["textDocument"]["uri"]
            .as_str()
            .is_some_and(|uri| uri.ends_with(&format!("/{file}")))
}

fn method(message: &Value) -> &str {
    message["method"].as_str().unwrap_or("")
}

#[test]
fn syncs_document() {
    let project = Project::new(None);
    let mut griffin = project.open(&rust_server(fake()), "a.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "a.rs");

    griffin.type_text("x");
    project.wait_for(&griffin, "textDocument/didChange", "a.rs");
    griffin.send_keys("ctrl+s");
    griffin.wait_for_text("saved", WAIT);
    project.wait_for(&griffin, "textDocument/didSave", "a.rs");

    // A second Rust file in the same root goes to the same server.
    griffin.send_keys("ctrl+p");
    griffin.wait_for_text("Go to file:", WAIT);
    griffin.type_text("b.rs");
    griffin.wait_for_text("Go to file: b.rs", WAIT);
    griffin.send_keys("enter");
    griffin.wait_for_text_gone("Go to file:", WAIT);
    project.wait_for(&griffin, "textDocument/didOpen", "b.rs");

    // Back to a.rs, which is saved, and close it.
    griffin.send_keys("alt+,");
    griffin.send_keys("ctrl+w");
    project.wait_for(&griffin, "textDocument/didClose", "a.rs");

    let log = project.log();
    let pids: std::collections::BTreeSet<u64> = log.iter().map(|(pid, _)| *pid).collect();
    assert_eq!(pids.len(), 1, "one server process for both files: {log:#?}");
    let inits = log
        .iter()
        .filter(|(_, m)| method(m) == "initialize")
        .count();
    assert_eq!(inits, 1, "{log:#?}");

    // a.rs's life, in order, after the handshake.
    let mut seen: Vec<&str> = Vec::new();
    for (_, message) in &log {
        let name = method(message);
        let about_a = message["params"]["textDocument"]["uri"]
            .as_str()
            .is_some_and(|uri| uri.ends_with("/a.rs"));
        if name == "initialize" || name == "initialized" || about_a {
            seen.push(name);
        }
    }
    assert_eq!(
        seen,
        [
            "initialize",
            "initialized",
            "textDocument/didOpen",
            "textDocument/didChange",
            "textDocument/didSave",
            "textDocument/didClose",
        ],
        "{log:#?}"
    );

    // The fake advertises incremental sync, so the change is just the `x`.
    let (_, open) = log
        .iter()
        .find(|(_, m)| is(m, "textDocument/didOpen", "a.rs"))
        .expect("didOpen logged");
    assert_eq!(open["params"]["textDocument"]["text"], "fn a() {}\n");
    assert_eq!(open["params"]["textDocument"]["languageId"], "rust");
    let (_, change) = log
        .iter()
        .find(|(_, m)| is(m, "textDocument/didChange", "a.rs"))
        .expect("didChange logged");
    let edit = &change["params"]["contentChanges"][0];
    assert_eq!(edit["text"], "x");
    assert_eq!(edit["range"]["start"]["line"], 0);
    assert_eq!(edit["range"]["start"]["character"], 0);
    assert_eq!(change["params"]["textDocument"]["version"], 1);
    assert_eq!(project.read("a.rs"), "xfn a() {}\n");
}

#[test]
fn full_sync_sends_the_whole_text() {
    let script = r#"{"responses": {"initialize": {"capabilities": {"textDocumentSync": 1}}}}"#;
    let project = Project::new(Some(script));
    let mut griffin = project.open(&rust_server(fake()), "a.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "a.rs");
    griffin.type_text("y");
    project.wait_for(&griffin, "textDocument/didChange", "a.rs");
    let log = project.log();
    let (_, change) = log
        .iter()
        .find(|(_, m)| is(m, "textDocument/didChange", "a.rs"))
        .expect("didChange logged");
    let edit = &change["params"]["contentChanges"][0];
    assert_eq!(edit["text"], "yfn a() {}\n");
    assert!(edit.get("range").is_none(), "{edit}");
}

#[test]
fn missing_server_keeps_editing() {
    let project = Project::new(None);
    let mut griffin = project.open(&rust_server("griffin-no-such-server"), "a.rs");
    let message = "rust: server not found (griffin-no-such-server)";
    griffin.wait_for_text(message, WAIT);

    griffin.type_text("abc");
    griffin.wait_for_text("a.rs ●", WAIT);
    griffin.send_keys("ctrl+s");
    griffin.wait_for_text("saved a.rs", WAIT);
    assert_eq!(project.read("a.rs"), "abcfn a() {}\n");

    // Another Rust file doesn't try again or say it again.
    griffin.send_keys("ctrl+p");
    griffin.wait_for_text("Go to file:", WAIT);
    griffin.type_text("b.rs");
    griffin.wait_for_text("Go to file: b.rs", WAIT);
    griffin.send_keys("enter");
    griffin.wait_for_text_gone("Go to file:", WAIT);
    griffin.type_text("z");
    griffin.wait_for_text("b.rs ●", WAIT);
    let status = status_line(&griffin);
    assert!(!status.contains("server not found"), "{status:?}");
    assert!(status.contains("saved a.rs"), "{status:?}");
}

/// npm installs servers as `.cmd` shims; Griffin has to find one through
/// `PATHEXT` and start it, as `--health` already finds it.
#[cfg(windows)]
#[test]
fn a_cmd_server_on_path_starts() {
    let project = Project::new(None);
    let bin = tempfile::tempdir().expect("create shim dir");
    fs::write(
        bin.path().join("griffin-cmd-server.cmd"),
        format!(
            "@\"{}\" %*
",
            fake()
        ),
    )
    .expect("write shim");
    let mut path = OsString::from(bin.path());
    if let Some(rest) = std::env::var_os("PATH") {
        path.push(";");
        path.push(rest);
    }
    let mut env = project.env();
    env.push(("PATH", path));
    let mut griffin = Griffin::spawn_in_with_config_and_env(
        project.dir.path(),
        &rust_server("griffin-cmd-server"),
        &env,
        &["a.rs"],
    );
    griffin.wait_for_text("Ln 1, Col 1", START);
    project.wait_for(&griffin, "textDocument/didOpen", "a.rs");
    let log = project.log();
    assert!(
        log.iter().any(|(_, m)| method(m) == "initialize"),
        "{log:#?}"
    );
    let status = status_line(&griffin);
    assert!(!status.contains("server not found"), "{status:?}");
    // Quit cleanly so the fake behind the shim exits too.
    griffin.send_keys("ctrl+q");
    griffin.wait_exit(WAIT);
}

#[test]
fn crashed_server_keeps_editing() {
    let project = Project::new(Some(r#"{"exit_on": "textDocument/didChange"}"#));
    let mut griffin = project.open(&rust_server(fake()), "a.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "a.rs");

    griffin.type_text("q");
    griffin.wait_for_text("rust: server crashed (exit code 3)", WAIT);

    griffin.type_text("rs");
    griffin.wait_for_text("qrsfn a()", WAIT);
    griffin.send_keys("ctrl+s");
    griffin.wait_for_text("saved a.rs", WAIT);
    assert_eq!(project.read("a.rs"), "qrsfn a() {}\n");
    // Nothing more reached the dead server.
    let changes = project
        .log()
        .iter()
        .filter(|(_, m)| method(m) == "textDocument/didChange")
        .count();
    assert_eq!(changes, 1);
}

#[test]
fn quitting_shuts_the_server_down() {
    let project = Project::new(None);
    let mut griffin = project.open(&rust_server(fake()), "a.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "a.rs");
    griffin.send_keys("ctrl+q");
    griffin.wait_exit(WAIT);
    griffin.wait_for_files("shutdown then exit as the last messages", WAIT, || {
        let methods: Vec<String> = project
            .log()
            .iter()
            .map(|(_, m)| method(m).to_string())
            .collect();
        methods.ends_with(&["shutdown".to_string(), "exit".to_string()])
    });
}

/// A Rust file with something to complain about on lines 2 and 3.
const DIAG_FILE: &str = "fn main() {\n    let x = 1;\n    let y = 2;\n}\n";

/// The theme with `err` and `working` pinned, so the colours are known.
fn diag_config() -> String {
    format!(
        "{}[theme_overrides]\nerr = \"#ff0000\"\nworking = \"#ffaa00\"\n",
        rust_server(fake())
    )
}

const ERR: vt100::Color = vt100::Color::Rgb(0xff, 0x00, 0x00);
const WORKING: vt100::Color = vt100::Color::Rgb(0xff, 0xaa, 0x00);

/// A `publishDiagnostics` for the file just received, one entry per
/// `(line, start, end, severity, message)`.
fn publish(diagnostics: &[(u32, u32, u32, u8, &str)]) -> String {
    let list: Vec<String> = diagnostics
        .iter()
        .map(|(line, start, end, severity, message)| {
            format!(
                r#"{{"range": {{"start": {{"line": {line}, "character": {start}}},
                   "end": {{"line": {line}, "character": {end}}}}},
                   "severity": {severity}, "message": "{message}"}}"#
            )
        })
        .collect();
    format!(
        r#"{{"method": "textDocument/publishDiagnostics",
            "params": {{"uri": "$uri", "diagnostics": [{}]}}}}"#,
        list.join(", ")
    )
}

/// A warning on `x` (line 2) and an error on `y` (line 3).
fn both() -> String {
    publish(&[
        (1, 8, 9, 2, "unused variable: x"),
        (2, 8, 9, 1, "mismatched types"),
    ])
}

fn diag_project(script: &str) -> (Project, Griffin) {
    let project = Project::new(Some(script));
    fs::write(project.dir.path().join("c.rs"), DIAG_FILE).expect("write c.rs");
    let griffin = project.open(&diag_config(), "c.rs");
    (project, griffin)
}

// Screen rows: the tab bar is row 0, so buffer line n is row n. The gutter
// " 1 │ " is 5 cells, so char column c is screen column 5 + c.
const X_ROW: u16 = 2;
const Y_ROW: u16 = 3;
const VAR_COL: u16 = 13;

#[test]
fn diagnostics_are_underlined_marked_and_counted() {
    let script = format!(r#"{{"notify": {{"textDocument/didOpen": [{}]}}}}"#, both());
    let (_project, griffin) = diag_project(&script);
    griffin.wait_for_text("⚠ 1  ✕ 1", WAIT);

    griffin.wait_for_underlined(Y_ROW, "y", WAIT);
    assert_eq!(griffin.fg_at(VAR_COL, Y_ROW), ERR);
    griffin.wait_for_underlined(X_ROW, "x", WAIT);
    assert_eq!(griffin.fg_at(VAR_COL, X_ROW), WORKING);
    // Nothing else is underlined.
    assert_eq!(griffin.underlined_text(1), "");
    assert_eq!(griffin.underlined_text(4), "");

    // The gutter marks both lines in their colour, and only them.
    let screen = griffin.screen();
    assert!(
        screen[usize::from(X_ROW)].starts_with("●2 │ "),
        "{screen:#?}"
    );
    assert!(
        screen[usize::from(Y_ROW)].starts_with("●3 │ "),
        "{screen:#?}"
    );
    assert!(screen[1].starts_with(" 1 │ "), "{screen:#?}");
    assert_eq!(griffin.fg_at(0, X_ROW), WORKING);
    assert_eq!(griffin.fg_at(0, Y_ROW), ERR);

    // The counts on the status line are in the same colours.
    let status = status_line(&griffin);
    assert!(status.contains("⚠ 1  ✕ 1"), "{status:?}");
    let status_row = ROWS - 1;
    let warn = griffin.text_col(status_row, "⚠ 1").expect("warning count");
    let err = griffin.text_col(status_row, "✕ 1").expect("error count");
    assert_eq!(griffin.fg_at(warn, status_row), WORKING);
    assert_eq!(griffin.fg_at(err, status_row), ERR);
}

#[test]
fn f8_and_shift_f8_cycle_through_diagnostics() {
    let script = format!(r#"{{"notify": {{"textDocument/didOpen": [{}]}}}}"#, both());
    let (_project, mut griffin) = diag_project(&script);
    griffin.wait_for_text("⚠ 1  ✕ 1", WAIT);

    griffin.send_keys("f8");
    griffin.wait_for_cursor(VAR_COL, X_ROW, WAIT);
    griffin.wait_for_text("Ln 2, Col 9", WAIT);
    griffin.send_keys("f8");
    griffin.wait_for_cursor(VAR_COL, Y_ROW, WAIT);
    // Past the last one, back to the first.
    griffin.send_keys("f8");
    griffin.wait_for_cursor(VAR_COL, X_ROW, WAIT);
    // Before the first one, round to the last.
    griffin.send_keys("shift+f8");
    griffin.wait_for_cursor(VAR_COL, Y_ROW, WAIT);
    griffin.send_keys("shift+f8");
    griffin.wait_for_cursor(VAR_COL, X_ROW, WAIT);
}

#[test]
fn the_diagnostic_under_the_cursor_shows_its_message() {
    let script = format!(r#"{{"notify": {{"textDocument/didOpen": [{}]}}}}"#, both());
    let (_project, mut griffin) = diag_project(&script);
    griffin.wait_for_text("⚠ 1  ✕ 1", WAIT);
    assert!(!status_line(&griffin).contains("unused variable"));

    griffin.send_keys("f8");
    griffin.wait_for_text("griffin  unused variable: x  c.rs", WAIT);
    griffin.send_keys("f8");
    griffin.wait_for_text("griffin  mismatched types  c.rs", WAIT);
    griffin.wait_for_text_gone("unused variable", WAIT);

    // Off the diagnostic, the message goes.
    griffin.send_keys("right");
    griffin.wait_for_text("Ln 3, Col 10", WAIT);
    griffin.wait_for_text_gone("mismatched types", WAIT);
}

#[test]
fn a_new_publish_replaces_the_set_and_an_empty_one_clears_it() {
    let only_error = publish(&[(2, 8, 9, 1, "mismatched types")]);
    let script = format!(
        r#"{{"notify": {{
            "textDocument/didOpen": [{}],
            "textDocument/didChange": [{only_error}],
            "textDocument/didSave": [{}]
        }}}}"#,
        both(),
        publish(&[])
    );
    let (project, mut griffin) = diag_project(&script);
    griffin.wait_for_text("⚠ 1  ✕ 1", WAIT);
    griffin.wait_for_underlined(X_ROW, "x", WAIT);

    // Typing on line 1 sends a change; the server's new set has only the error.
    griffin.type_text("z");
    project.wait_for(&griffin, "textDocument/didChange", "c.rs");
    griffin.wait_for_text("⚠ 0  ✕ 1", WAIT);
    griffin.wait_for_underlined(X_ROW, "", WAIT);
    griffin.wait_for_underlined(Y_ROW, "y", WAIT);
    assert!(griffin.screen()[usize::from(X_ROW)].starts_with(" 2 │ "));

    // Saving gets an empty publish: everything goes.
    griffin.send_keys("ctrl+s");
    griffin.wait_for_text("saved c.rs", WAIT);
    griffin.wait_for_text_gone("✕", WAIT);
    griffin.wait_for_underlined(Y_ROW, "", WAIT);
    assert!(griffin.screen()[usize::from(Y_ROW)].starts_with(" 3 │ "));
    assert!(!status_line(&griffin).contains("⚠"));
}

/// The fake's answer to every `textDocument/definition`: `greet` in `util.rs`.
const DEFINITION_IN_UTIL: &str = r#"{"responses": {"textDocument/definition": {
    "uri": "$dir/util.rs",
    "range": {"start": {"line": 2, "character": 7}, "end": {"line": 2, "character": 12}}
}}}"#;

/// A project holding the `tests/fixtures/definition` files, with `main.rs` open:
/// `util::greet()` on its line 4 calls `greet`, defined on `util.rs` line 3.
fn definition_project(script: &str) -> (Project, Griffin) {
    let project = Project::new(Some(script));
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/definition");
    for name in ["main.rs", "util.rs"] {
        fs::copy(fixtures.join(name), project.dir.path().join(name)).expect("copy fixture");
    }
    let griffin = project.open(&rust_server(fake()), "main.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "main.rs");
    (project, griffin)
}

// `greet` in `    util::greet();` starts at char 10 of line 4, which is screen
// column 5 + 10 past the gutter, on row 4 below the tab bar.
const GREET_COL: u16 = 15;
const GREET_ROW: u16 = 4;

/// The position of the last `textDocument/definition` the fake received, as
/// (line, character).
fn definition_position(project: &Project) -> (u64, u64) {
    let log = project.log();
    let (_, request) = log
        .iter()
        .rev()
        .find(|(_, m)| is(m, "textDocument/definition", "main.rs"))
        .expect("definition request logged");
    let position = &request["params"]["position"];
    (
        position["line"].as_u64().unwrap_or(u64::MAX),
        position["character"].as_u64().unwrap_or(u64::MAX),
    )
}

#[test]
fn f12_opens_the_definition_in_another_file_and_alt_left_comes_back() {
    let (project, mut griffin) = definition_project(DEFINITION_IN_UTIL);
    // One key at a time: a burst of keys can read as a paste on Windows.
    for (key, at) in [
        ("down", "Ln 2, Col 1"),
        ("down", "Ln 3, Col 1"),
        ("down", "Ln 4, Col 1"),
        ("ctrl+right", "Ln 4, Col 9"),
        ("ctrl+right", "Ln 4, Col 11"),
    ] {
        griffin.send_keys(key);
        griffin.wait_for_text(at, WAIT);
    }

    griffin.send_keys("f12");
    project.wait_for(&griffin, "textDocument/definition", "main.rs");
    assert_eq!(definition_position(&project), (3, 10));
    // util.rs opens in a tab of its own, cursor on `greet`.
    griffin.wait_for_text("Ln 3, Col 8", WAIT);
    let tabs = griffin.screen()[0].clone();
    assert!(
        tabs.contains("main.rs") && tabs.contains("util.rs"),
        "{tabs:?}"
    );
    griffin.wait_for_cursor(5 + 7, 3, WAIT);
    assert!(status_line(&griffin).contains("util.rs"));

    griffin.send_keys("alt+left");
    griffin.wait_for_text("Ln 4, Col 11", WAIT);
    griffin.wait_for_cursor(GREET_COL, GREET_ROW, WAIT);
    assert!(status_line(&griffin).contains("main.rs"));

    // Nothing more to go back to: it stays put.
    griffin.send_keys("alt+left");
    griffin.send_keys("right");
    griffin.wait_for_text("Ln 4, Col 12", WAIT);
    assert!(status_line(&griffin).contains("main.rs"));
}

#[test]
fn ctrl_click_goes_to_the_definition() {
    let (project, mut griffin) = definition_project(DEFINITION_IN_UTIL);
    griffin.ctrl_click(GREET_COL + 2, GREET_ROW);
    project.wait_for(&griffin, "textDocument/definition", "main.rs");
    assert_eq!(definition_position(&project), (3, 12));
    griffin.wait_for_text("Ln 3, Col 8", WAIT);
    assert!(status_line(&griffin).contains("util.rs"));

    // Back to where the click put the cursor.
    griffin.send_keys("alt+left");
    griffin.wait_for_text("Ln 4, Col 13", WAIT);
    assert!(status_line(&griffin).contains("main.rs"));
}

#[test]
fn no_definition_says_so() {
    // Unscripted, the fake answers `null`.
    let (project, mut griffin) = definition_project("{}");
    griffin.send_keys("f12");
    project.wait_for(&griffin, "textDocument/definition", "main.rs");
    griffin.wait_for_text("No definition found", WAIT);
    let status = status_line(&griffin);
    assert!(
        status.contains("main.rs") && status.contains("Ln 1, Col 1"),
        "{status:?}"
    );
    assert!(!griffin.screen()[0].contains("util.rs"));
}

/// The fake's answer to every `textDocument/definition`: the start of `main.rs`.
const DEFINITION_AT_TOP: &str = r#"{"responses": {"textDocument/definition": {
    "uri": "$dir/main.rs",
    "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}}
}}}"#;

#[test]
fn the_jump_list_keeps_the_last_50_places() {
    let project = Project::new(Some(DEFINITION_AT_TOP));
    fs::write(
        project.dir.path().join("main.rs"),
        "abcdefghij\n".repeat(27),
    )
    .expect("write main.rs");
    let mut griffin = project.open(&rust_server(fake()), "main.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "main.rs");

    let text = griffin.text_col(1, "abcdefghij").expect("line 1 on screen");
    // 55 different places, each left by F12: lines 2 to 26 at columns 2, 4 and 6.
    let places: Vec<(u16, u16)> = (0..55u16).map(|i| (2 + i % 25, 2 + 2 * (i / 25))).collect();
    for &(line, col) in &places {
        // Line n is on screen row n, below the tab bar.
        griffin.click(text + col - 1, line);
        griffin.wait_for_text(&format!("Ln {line}, Col {col}"), WAIT);
        griffin.send_keys("f12");
        griffin.wait_for_text("Ln 1, Col 1", WAIT);
    }

    // Back through the newest 50, newest first.
    for &(line, col) in places.iter().rev().take(50) {
        griffin.send_keys("alt+left");
        griffin.wait_for_text(&format!("Ln {line}, Col {col}"), WAIT);
    }
    // The 50th-newest is as far as it goes: the five before it were dropped.
    let (line, col) = places[5];
    for _ in 0..5 {
        griffin.send_keys("alt+left");
    }
    griffin.send_keys("right");
    griffin.wait_for_text(&format!("Ln {line}, Col {}", col + 1), WAIT);
    assert!(status_line(&griffin).contains(&format!("Ln {line}, Col {}", col + 1)));
}

/// The theme with `card` pinned, so the popup's background is known.
const CARD: vt100::Color = vt100::Color::Rgb(0x20, 0x30, 0x40);

/// The fake's answer to every `textDocument/hover`: markdown with a code fence
/// and a paragraph too long for one line of the popup.
const HOVER_MARKDOWN: &str = r#"{"responses": {"textDocument/hover": {"contents": {
    "kind": "markdown",
    "value": "```rust\npub fn greet()\n```\n\nGreets whoever is listening, then keeps on talking for quite a while so this line wraps."
}}}}"#;

/// A definition project (cursor still at the top) with `card` pinned, its
/// cursor moved onto `greet` on line 4.
fn hover_project(script: &str) -> (Project, Griffin) {
    let project = Project::new(Some(script));
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/definition");
    for name in ["main.rs", "util.rs"] {
        fs::copy(fixtures.join(name), project.dir.path().join(name)).expect("copy fixture");
    }
    let config = format!(
        "{}[theme_overrides]\ncard = \"#203040\"\n",
        rust_server(fake())
    );
    let mut griffin = project.open(&config, "main.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "main.rs");
    for (key, at) in [
        ("down", "Ln 2, Col 1"),
        ("down", "Ln 3, Col 1"),
        ("down", "Ln 4, Col 1"),
        ("ctrl+right", "Ln 4, Col 9"),
        ("ctrl+right", "Ln 4, Col 11"),
    ] {
        griffin.send_keys(key);
        griffin.wait_for_text(at, WAIT);
    }
    (project, griffin)
}

// Below the cursor on `greet` (15, 4): the border on row 5, then the text one
// cell in from the border, on rows 6 to 9.
const HOVER_TEXT_COL: u16 = 17;
const HOVER_FIRST_ROW: u16 = 6;

#[test]
fn alt_k_shows_the_hover_below_the_cursor() {
    let (project, mut griffin) = hover_project(HOVER_MARKDOWN);
    griffin.send_keys("alt+k");
    project.wait_for(&griffin, "textDocument/hover", "main.rs");
    let (_, request) = project
        .log()
        .into_iter()
        .rfind(|(_, m)| is(m, "textDocument/hover", "main.rs"))
        .expect("hover request logged");
    assert_eq!(
        request["params"]["position"],
        serde_json::json!({"line": 3, "character": 10})
    );

    griffin.wait_for_text("pub fn greet()", WAIT);
    let screen = griffin.screen();
    assert!(!screen.iter().any(|row| row.contains("```")), "{screen:#?}");
    // Plain text, fence gone, the paragraph wrapped at the popup's 60 columns.
    for (row, text) in [
        (HOVER_FIRST_ROW, "pub fn greet()"),
        (
            HOVER_FIRST_ROW + 2,
            "Greets whoever is listening, then keeps on talking for quite",
        ),
        (HOVER_FIRST_ROW + 3, "a while so this line wraps."),
    ] {
        assert_eq!(
            griffin.text_col(row, text),
            Some(HOVER_TEXT_COL),
            "{text:?} on row {row}: {screen:#?}"
        );
    }
    assert_eq!(griffin.bg_at(HOVER_TEXT_COL, HOVER_FIRST_ROW), CARD);
    assert_eq!(griffin.bg_at(HOVER_TEXT_COL + 40, HOVER_FIRST_ROW), CARD);
    // The cursor stays in the text, on `greet`.
    griffin.wait_for_cursor(GREET_COL, GREET_ROW, WAIT);
    assert!(status_line(&griffin).contains("griffin  main.rs  Ln 4, Col 11"));
}

#[test]
fn esc_movement_and_typing_close_the_hover() {
    let (_project, mut griffin) = hover_project(HOVER_MARKDOWN);

    griffin.send_keys("alt+k");
    griffin.wait_for_text("pub fn greet()", WAIT);
    griffin.send_keys("esc");
    griffin.wait_for_text_gone("pub fn greet()", WAIT);
    assert!(status_line(&griffin).contains("Ln 4, Col 11"));

    griffin.send_keys("alt+k");
    griffin.wait_for_text("pub fn greet()", WAIT);
    griffin.send_keys("right");
    griffin.wait_for_text("Ln 4, Col 12", WAIT);
    griffin.wait_for_text_gone("pub fn greet()", WAIT);

    griffin.send_keys("alt+k");
    griffin.wait_for_text("pub fn greet()", WAIT);
    griffin.type_text("z");
    griffin.wait_for_text("util::gzreet();", WAIT);
    griffin.wait_for_text_gone("pub fn greet()", WAIT);
}

#[test]
fn an_empty_hover_shows_nothing() {
    // The hover reply is `null`; a publish sent right after it marks when the
    // reply has been handled.
    let script = format!(
        r#"{{"notify": {{"textDocument/hover": [{}]}}}}"#,
        publish(&[(0, 0, 2, 2, "after the hover")])
    );
    let (project, mut griffin) = hover_project(&script);
    let before = griffin.screen();
    griffin.send_keys("alt+k");
    project.wait_for(&griffin, "textDocument/hover", "main.rs");
    griffin.wait_for_text("⚠ 1  ✕ 0", WAIT);

    // No card anywhere: the text area is as it was, but for the gutter mark
    // the publish put on line 1.
    let after = griffin.screen();
    assert_eq!(after[1].replacen('●', " ", 1), before[1], "{after:#?}");
    assert_eq!(
        &after[2..usize::from(ROWS - 1)],
        &before[2..usize::from(ROWS - 1)]
    );
    let status = status_line(&griffin);
    assert!(
        status.contains("griffin  main.rs  Ln 4, Col 11"),
        "{status:?}"
    );
}

#[test]
fn the_hover_flips_above_and_shifts_left_at_the_screen_edges() {
    let script = r#"{"responses": {"textDocument/hover": {"contents": {
        "kind": "plaintext",
        "value": "fn last() -> u32\n\nReturns the last value, counting back from the very end."
    }}}}"#;
    let project = Project::new(Some(script));
    // 40 lines, the last 85 characters long with no line break after it.
    let mut text = "// filler\n".repeat(39);
    text.push_str(&format!("fn last() -> u32 {{ 7 }} {}", "/".repeat(85 - 23)));
    assert_eq!(text.lines().last().map(str::len), Some(85));
    fs::write(project.dir.path().join("long.rs"), &text).expect("write long.rs");
    let config = format!(
        "{}[theme_overrides]\ncard = \"#203040\"\n",
        rust_server(fake())
    );
    let mut griffin = project.open(&config, "long.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "long.rs");

    griffin.send_keys("ctrl+end");
    griffin.wait_for_text("Ln 40, Col 86", WAIT);
    // The gutter " 40 │ " is 6 cells; the last editor row is just above the status.
    let cursor = (6 + 85, ROWS - 2);
    griffin.wait_for_cursor(cursor.0, cursor.1, WAIT);

    griffin.send_keys("alt+k");
    let doc = "Returns the last value, counting back from the very end.";
    griffin.wait_for_text(doc, WAIT);
    // 56 columns of text in a 60-wide card, pushed left to end at the right
    // edge, and flipped above: border, three text rows, border ending just over
    // the cursor's row.
    let row = cursor.1 - 2;
    let col = griffin.text_col(row, doc);
    assert_eq!(col, Some(100 - 60 + 2), "{:#?}", griffin.screen());
    assert_eq!(griffin.text_col(row - 2, "fn last() -> u32"), Some(42));
    assert_eq!(griffin.bg_at(42, row), CARD);
    // The cursor's line and the status line are untouched.
    let screen = griffin.screen();
    assert!(
        screen[usize::from(cursor.1)].starts_with(" 40 │ fn last()"),
        "{screen:#?}"
    );
    assert!(status_line(&griffin).contains("Ln 40, Col 86"));
    griffin.wait_for_cursor(cursor.0, cursor.1, WAIT);
}

/// The theme with `card` and `hov` pinned, so the popup's rows are known.
const COMPLETION_THEME: &str = "[theme_overrides]\ncard = \"#203040\"\nhov = \"#405060\"\n";
const HOV: vt100::Color = vt100::Color::Rgb(0x40, 0x50, 0x60);

/// Whether `row` is the selected one, showing `label` on `hov`: drawn as reverse
/// video, so `hov` is the cells' foreground (see `Theme::highlight`).
fn on_hov(griffin: &Griffin, row: u16, label: &str) -> bool {
    griffin.fg_at(LABEL_COL, row) == HOV && griffin.reversed_text(row).contains(label)
}

/// Initialize advertising `.` as a completion trigger, then 12 items for every
/// `textDocument/completion`, sorted by label: `len` carries a `textEdit` from
/// the cursor after `s.` on line 2, `push` a snippet, `push_str` plain insert
/// text and `trim` only its label.
fn completion_script(extra: &str) -> String {
    let method = |label: &str| format!(r#"{{"label": "{label}", "kind": 2}}"#);
    let mut items: Vec<String> = [
        "capacity",
        "chars",
        "clear",
        "contains",
        "ends_with",
        "is_empty",
        "lines",
    ]
    .iter()
    .map(|l| method(l))
    .collect();
    items.extend([
        r#"{"label": "as_str", "kind": 5}"#.to_string(),
        r#"{"label": "len", "kind": 2, "textEdit": {"range": {
            "start": {"line": 1, "character": 6}, "end": {"line": 1, "character": 6}},
            "newText": "len()"}}"#
            .to_string(),
        r#"{"label": "push", "kind": 2, "insertTextFormat": 2,
            "insertText": "push(${1:ch})$0"}"#
            .to_string(),
        r#"{"label": "push_str", "kind": 2, "insertText": "push_str"}"#.to_string(),
        r#"{"label": "trim", "kind": 3}"#.to_string(),
    ]);
    format!(
        r#"{{"responses": {{
            "initialize": {{"capabilities": {{
                "textDocumentSync": {{"openClose": true, "change": 2, "save": {{"includeText": false}}}},
                "completionProvider": {{"triggerCharacters": ["."]}}}}}},
            "textDocument/completion": {{"isIncomplete": false, "items": [{}]}}
        }}{extra}}}"#,
        items.join(", ")
    )
}

/// A project with `comp.rs` open on the fake server, the cursor at the end of
/// its indented empty line 2.
fn completion_project(script: &str) -> (Project, Griffin) {
    let project = Project::new(Some(script));
    fs::write(project.dir.path().join("comp.rs"), "fn main() {\n    \n}\n").expect("write comp.rs");
    let config = format!("{}{COMPLETION_THEME}", rust_server(fake()));
    let mut griffin = project.open(&config, "comp.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "comp.rs");
    griffin.send_keys("down");
    griffin.wait_for_text("Ln 2, Col 1", WAIT);
    griffin.send_keys("end");
    griffin.wait_for_text("Ln 2, Col 5", WAIT);
    (project, griffin)
}

fn completion_requests(project: &Project) -> Vec<Value> {
    project
        .log()
        .into_iter()
        .map(|(_, m)| m)
        .filter(|m| is(m, "textDocument/completion", "comp.rs"))
        .collect()
}

/// Types `s` then `.` on the current line and waits for the popup.
fn open_completion(project: &Project, griffin: &mut Griffin, line: u16) {
    let asked = completion_requests(project).len();
    griffin.type_text("s");
    griffin.wait_for_text(&format!("Ln {line}, Col 6"), WAIT);
    griffin.type_text(".");
    griffin.wait_for_files("a completion request", WAIT, || {
        completion_requests(project).len() > asked
    });
    griffin.wait_for_text("capacity", WAIT);
}

// The gutter " 1 │ " is 5 cells, so after `    s.` on line 2 the cursor is at
// (11, 2). The card's border is on row 3; its rows start on row 4, the kind one
// cell in from the border at column 13 and, after the 6-wide `method` and a
// space, the label at column 20.
const ITEM_ROW: u16 = 4;
const KIND_COL: u16 = 13;
const LABEL_COL: u16 = 20;

#[test]
fn a_trigger_character_or_alt_slash_shows_up_to_ten_items_with_kinds() {
    let (project, mut griffin) = completion_project(&completion_script(""));
    open_completion(&project, &mut griffin, 2);
    let requests = completion_requests(&project);
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0]["params"]["position"],
        serde_json::json!({"line": 1, "character": 6})
    );
    // The server heard about the `.` before being asked about the text after it.
    let log: Vec<Value> = project.log().into_iter().map(|(_, m)| m).collect();
    let asked = log
        .iter()
        .position(|m| method(m) == "textDocument/completion")
        .expect("completion logged");
    let last_change = log[..asked]
        .iter()
        .rfind(|m| method(m) == "textDocument/didChange")
        .expect("a change before the request");
    assert_eq!(last_change["params"]["contentChanges"][0]["text"], ".");

    griffin.wait_for_cursor(11, 2, WAIT);
    let screen = griffin.screen();
    let expected = [
        ("field ", "as_str"),
        ("method", "capacity"),
        ("method", "chars"),
        ("method", "clear"),
        ("method", "contains"),
        ("method", "ends_with"),
        ("method", "is_empty"),
        ("method", "len"),
        ("method", "lines"),
        ("method", "push"),
    ];
    for (row, (kind, label)) in (ITEM_ROW..).zip(expected) {
        assert_eq!(
            griffin.text_col(row, &format!("{kind} {label}")),
            Some(KIND_COL),
            "{label} on row {row}: {screen:#?}"
        );
    }
    // Ten at most: the last two sorted items aren't shown.
    assert!(
        !screen.iter().any(|r| r.contains("push_str")),
        "{screen:#?}"
    );
    assert!(!screen.iter().any(|r| r.contains("trim")), "{screen:#?}");
    // The first row is selected, on `hov`; the rest are on the card.
    assert!(on_hov(&griffin, ITEM_ROW, "as_str"), "{screen:#?}");
    assert_eq!(griffin.bg_at(LABEL_COL, ITEM_ROW + 1), CARD);
    assert!(status_line(&griffin).contains("Ln 2, Col 7"));

    // Esc closes it; Alt+/ asks again from the same place.
    griffin.send_keys("esc");
    griffin.wait_for_text_gone("capacity", WAIT);
    griffin.send_keys("alt+/");
    griffin.wait_for_text("capacity", WAIT);
    let requests = completion_requests(&project);
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1]["params"]["position"],
        serde_json::json!({"line": 1, "character": 6})
    );
}

#[test]
fn typing_filters_and_the_arrows_move_the_selection() {
    let (project, mut griffin) = completion_project(&completion_script(""));
    open_completion(&project, &mut griffin, 2);

    // Case doesn't matter: `P` keeps `push` and `push_str`.
    griffin.type_text("P");
    griffin.wait_for_text_gone("capacity", WAIT);
    griffin.wait_for_text("s.P", WAIT);
    // The card follows the cursor, one cell right now.
    let screen = griffin.screen();
    assert_eq!(
        griffin.text_col(ITEM_ROW, "method push"),
        Some(KIND_COL + 1),
        "{screen:#?}"
    );
    assert_eq!(
        griffin.text_col(ITEM_ROW + 1, "method push_str"),
        Some(KIND_COL + 1)
    );
    assert!(on_hov(&griffin, ITEM_ROW, "push"), "{screen:#?}");

    griffin.send_keys("down");
    griffin.wait_for_fg_at(LABEL_COL + 1, ITEM_ROW + 1, HOV, WAIT);
    assert!(on_hov(&griffin, ITEM_ROW + 1, "push_str"));
    assert_eq!(griffin.bg_at(LABEL_COL + 1, ITEM_ROW), CARD);
    // The selection stops at the last item, and the cursor never moves.
    griffin.send_keys("down");
    griffin.send_keys("up");
    griffin.wait_for_fg_at(LABEL_COL + 1, ITEM_ROW, HOV, WAIT);
    assert!(status_line(&griffin).contains("Ln 2, Col 8"));

    // A filter matching nothing hides the popup; Backspace brings it back.
    griffin.type_text("z");
    griffin.wait_for_text_gone("push", WAIT);
    griffin.send_keys("backspace");
    griffin.wait_for_text("push_str", WAIT);
}

#[test]
fn enter_and_tab_insert_the_item_as_one_undo_step() {
    let (project, mut griffin) = completion_project(&completion_script(""));

    // `len`'s textEdit, stretched over the `le` typed since.
    open_completion(&project, &mut griffin, 2);
    griffin.type_text("le");
    griffin.wait_for_text_gone("capacity", WAIT);
    griffin.wait_for_text("method len", WAIT);
    griffin.send_keys("enter");
    griffin.wait_for_text("s.len()", WAIT);
    griffin.wait_for_text_gone("method len", WAIT);
    assert!(status_line(&griffin).contains("Ln 2, Col 12"));
    // One undo takes back just the insertion.
    griffin.send_keys("ctrl+z");
    griffin.wait_for_text_gone("s.len()", WAIT);
    griffin.wait_for_text("Ln 2, Col 9", WAIT);
    let screen = griffin.screen();
    assert!(screen[2].ends_with("s.le"), "{screen:#?}");
    griffin.send_keys("ctrl+y");
    griffin.wait_for_text("s.len()", WAIT);

    // Tab takes `push`'s snippet with its placeholder as plain text.
    griffin.send_keys("enter");
    griffin.wait_for_text("Ln 3, Col 5", WAIT);
    open_completion(&project, &mut griffin, 3);
    griffin.type_text("pu");
    griffin.wait_for_text_gone("capacity", WAIT);
    griffin.send_keys("tab");
    griffin.wait_for_text("s.push(ch)", WAIT);

    // `push_str`'s insert text, picked with Down. The cursor is on row 4, so the
    // card's rows start on row 6.
    griffin.send_keys("enter");
    griffin.wait_for_text("Ln 4, Col 5", WAIT);
    open_completion(&project, &mut griffin, 4);
    griffin.type_text("p");
    griffin.wait_for_text_gone("capacity", WAIT);
    griffin.send_keys("down");
    griffin.wait_for_fg_at(LABEL_COL + 1, 7, HOV, WAIT);
    griffin.send_keys("enter");
    griffin.wait_for_text("s.push_str", WAIT);

    // `trim` has only its label.
    griffin.send_keys("enter");
    griffin.wait_for_text("Ln 5, Col 5", WAIT);
    open_completion(&project, &mut griffin, 5);
    griffin.type_text("tr");
    griffin.wait_for_text_gone("capacity", WAIT);
    griffin.wait_for_text("fn trim", WAIT);
    griffin.send_keys("enter");
    griffin.wait_for_text_gone("fn trim", WAIT);
    griffin.wait_for_text("s.trim", WAIT);

    griffin.send_keys("ctrl+s");
    griffin.wait_for_text("saved", WAIT);
    assert_eq!(
        project.read("comp.rs"),
        "fn main() {\n    s.len()\n    s.push(ch)\n    s.push_str\n    s.trim\n}\n"
    );
}

#[test]
fn esc_dismisses_the_popup_without_changing_the_buffer() {
    let (project, mut griffin) = completion_project(&completion_script(""));
    open_completion(&project, &mut griffin, 2);
    let before = griffin.screen()[2].clone();
    griffin.send_keys("esc");
    griffin.wait_for_text_gone("capacity", WAIT);
    let screen = griffin.screen();
    assert_eq!(screen[2], before);
    assert!(screen[2].ends_with("    s."), "{screen:#?}");
    assert!(status_line(&griffin).contains("Ln 2, Col 7"));
    // Enter is the editor's again.
    griffin.send_keys("enter");
    griffin.wait_for_text("Ln 3, Col 5", WAIT);
    griffin.send_keys("ctrl+s");
    griffin.wait_for_text("saved", WAIT);
    assert_eq!(project.read("comp.rs"), "fn main() {\n    s.\n    \n}\n");
}

#[test]
fn a_late_answer_after_the_cursor_left_the_word_is_dropped() {
    // The answer comes 2 s late; a publish sent right after it marks when it
    // has been handled.
    let extra = format!(
        r#", "delay": {{"textDocument/completion": 2000}},
            "notify": {{"textDocument/completion": [{}]}}"#,
        publish(&[(0, 0, 2, 2, "after the completion")])
    );
    let (project, mut griffin) = completion_project(&completion_script(&extra));
    griffin.type_text("s");
    griffin.wait_for_text("Ln 2, Col 6", WAIT);
    griffin.type_text(".");
    project.wait_for(&griffin, "textDocument/completion", "comp.rs");
    griffin.send_keys("down");
    griffin.wait_for_text("Ln 3, Col 2", WAIT);

    griffin.wait_for_text("⚠ 1  ✕ 0", WAIT);
    let screen = griffin.screen();
    assert!(
        !screen.iter().any(|r| r.contains("capacity")),
        "{screen:#?}"
    );
    assert!(status_line(&griffin).contains("Ln 3, Col 2"));
    // Back on the word, the dropped answer doesn't come back either.
    griffin.send_keys("up");
    griffin.wait_for_text("Ln 2, Col 7", WAIT);
    let screen = griffin.screen();
    assert!(
        !screen.iter().any(|r| r.contains("capacity")),
        "{screen:#?}"
    );
}

/// `config.toml` with format on save for Rust, and the editor's indentation the
/// request should carry.
fn format_config(on: bool) -> String {
    format!(
        "[editor]\ntab_width = 2\ninsert_spaces = false\n{}format_on_save = {on}\n",
        rust_server(fake())
    )
}

fn format_requests(project: &Project) -> Vec<Value> {
    project
        .log()
        .into_iter()
        .map(|(_, message)| message)
        .filter(|message| method(message) == "textDocument/formatting")
        .collect()
}

#[test]
fn ctrl_s_formats_through_the_server_then_saves() {
    let range = |from: u32, to: u32| {
        format!(
            r#"{{"start": {{"line": 0, "character": {from}}}, "end": {{"line": 0, "character": {to}}}}}"#
        )
    };
    let script = format!(
        r#"{{"responses": {{"textDocument/formatting": [
            {{"range": {}, "newText": " "}},
            {{"range": {}, "newText": " "}}
        ]}}}}"#,
        range(2, 4),
        range(7, 7)
    );
    let project = Project::new(Some(&script));
    fs::write(project.dir.path().join("a.rs"), "fn  a(){}\n").expect("write a.rs");
    let mut griffin = project.open(&format_config(true), "a.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "a.rs");

    griffin.send_keys("ctrl+s");
    project.wait_for(&griffin, "textDocument/didSave", "a.rs");
    griffin.wait_for_text("fn a() {}", WAIT);
    griffin.wait_for_text("saved a.rs", WAIT);
    assert_eq!(project.read("a.rs"), "fn a() {}\n");
    let status = status_line(&griffin);
    assert!(!status.contains("unformatted"), "{status:?}");

    let requests = format_requests(&project);
    assert_eq!(requests.len(), 1, "{requests:#?}");
    assert_eq!(
        requests[0]["params"]["options"],
        serde_json::json!({"tabSize": 2, "insertSpaces": false})
    );
    // The request went out before the save the server heard of.
    let order: Vec<String> = project
        .log()
        .iter()
        .map(|(_, m)| method(m).to_string())
        .filter(|m| m == "textDocument/formatting" || m == "textDocument/didSave")
        .collect();
    assert_eq!(order, ["textDocument/formatting", "textDocument/didSave"]);

    // Both edits undo as one step.
    griffin.send_keys("ctrl+z");
    griffin.wait_for_text("fn  a(){}", WAIT);
    griffin.wait_for_text("a.rs ●", WAIT);
}

#[test]
fn a_server_that_never_answers_saves_unformatted_after_two_seconds() {
    let project = Project::new(Some(r#"{"silent": ["textDocument/formatting"]}"#));
    let mut griffin = project.open(&format_config(true), "a.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "a.rs");
    griffin.type_text("x");
    griffin.wait_for_text("a.rs ●", WAIT);

    griffin.send_keys("ctrl+s");
    project.wait_for(&griffin, "textDocument/formatting", "a.rs");
    griffin.wait_for_text("saved a.rs unformatted (no answer in 2 s)", WAIT);
    assert_eq!(project.read("a.rs"), "xfn a() {}\n");
    griffin.wait_for_text_gone("a.rs ●", WAIT);
}

#[test]
fn a_formatting_error_saves_unformatted_and_says_so() {
    let script = r#"{"errors": {"textDocument/formatting": {"code": -32603, "message": "boom"}}}"#;
    let project = Project::new(Some(script));
    let mut griffin = project.open(&format_config(true), "a.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "a.rs");
    griffin.type_text("y");
    griffin.wait_for_text("a.rs ●", WAIT);

    griffin.send_keys("ctrl+s");
    griffin.wait_for_text("saved a.rs unformatted (server error: boom)", WAIT);
    assert_eq!(project.read("a.rs"), "yfn a() {}\n");
}

#[test]
fn without_format_on_save_saving_sends_no_formatting_request() {
    // The fake would format if asked, so silence proves it wasn't.
    let script = r#"{"responses": {"textDocument/formatting": [{"range":
        {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 2}},
        "newText": "FN"}]}}"#;
    let project = Project::new(Some(script));
    // The default: no `format_on_save` key at all.
    let mut griffin = project.open(&rust_server(fake()), "a.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "a.rs");
    griffin.type_text("z");
    griffin.wait_for_text("a.rs ●", WAIT);

    griffin.send_keys("ctrl+s");
    project.wait_for(&griffin, "textDocument/didSave", "a.rs");
    griffin.wait_for_text("saved a.rs", WAIT);
    assert_eq!(project.read("a.rs"), "zfn a() {}\n");
    assert!(format_requests(&project).is_empty(), "{:#?}", project.log());

    // Explicitly off behaves the same.
    let project = Project::new(Some(script));
    let mut griffin = project.open(&format_config(false), "a.rs");
    project.wait_for(&griffin, "textDocument/didOpen", "a.rs");
    griffin.send_keys("ctrl+s");
    project.wait_for(&griffin, "textDocument/didSave", "a.rs");
    assert!(format_requests(&project).is_empty(), "{:#?}", project.log());
}
