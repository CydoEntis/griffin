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
