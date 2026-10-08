//! The language server client against `fake_lsp`, the scripted server in
//! `src/bin/fake_lsp.rs`, driven through the real binary.

mod harness;

use std::ffi::OsString;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use harness::{Glyph, ROWS};
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

fn status_line(glyph: &Glyph) -> String {
    glyph.screen()[usize::from(ROWS - 1)].clone()
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

    fn open(&self, config: &str, file: &str) -> Glyph {
        let glyph =
            Glyph::spawn_in_with_config_and_env(self.dir.path(), config, &self.env(), &[file]);
        glyph.wait_for_text("Ln 1, Col 1", START);
        glyph
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

    fn wait_for(&self, glyph: &Glyph, method: &str, file: &str) {
        glyph.wait_for_files(&format!("{method} for {file}"), WAIT, || {
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
    let mut glyph = project.open(&rust_server(fake()), "a.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "a.rs");

    glyph.type_text("x");
    project.wait_for(&glyph, "textDocument/didChange", "a.rs");
    glyph.send_keys("ctrl+s");
    glyph.wait_for_text("saved", WAIT);
    project.wait_for(&glyph, "textDocument/didSave", "a.rs");

    // A second Rust file in the same root goes to the same server.
    glyph.send_keys("ctrl+p");
    glyph.wait_for_text("cast · files · commands", WAIT);
    glyph.type_text("b.rs");
    glyph.wait_for_text("✦ b.rs", WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("cast · files · commands", WAIT);
    project.wait_for(&glyph, "textDocument/didOpen", "b.rs");

    // Back to a.rs, which is saved, and close it.
    glyph.send_keys("alt+,");
    glyph.send_keys("ctrl+w");
    project.wait_for(&glyph, "textDocument/didClose", "a.rs");

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
    let mut glyph = project.open(&rust_server(fake()), "a.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "a.rs");
    glyph.type_text("y");
    project.wait_for(&glyph, "textDocument/didChange", "a.rs");
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
    let mut glyph = project.open(&rust_server("glyph-no-such-server"), "a.rs");
    let message = "rust: server not found (glyph-no-such-server)";
    glyph.wait_for_text(message, WAIT);

    glyph.type_text("abc");
    glyph.wait_for_text("a.rs •", WAIT);
    glyph.send_keys("ctrl+s");
    glyph.wait_for_text("saved a.rs", WAIT);
    assert_eq!(project.read("a.rs"), "abcfn a() {}\n");

    // Another Rust file doesn't try again or say it again.
    glyph.send_keys("ctrl+p");
    glyph.wait_for_text("cast · files · commands", WAIT);
    glyph.type_text("b.rs");
    glyph.wait_for_text("✦ b.rs", WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("cast · files · commands", WAIT);
    // Checked before the next key, since any key clears a message: opening
    // b.rs syncs the server in the same frame, so a second "not found" would
    // be on screen now in the path's place.
    assert_eq!(glyph.text_col(ROWS - 1, "b.rs"), Some(20));
    let status = status_line(&glyph);
    assert!(!status.contains("server not found"), "{status:?}");
    glyph.type_text("z");
    glyph.wait_for_text("b.rs •", WAIT);
}

/// npm installs servers as `.cmd` shims; Glyph has to find one through
/// `PATHEXT` and start it, as `--health` already finds it.
#[cfg(windows)]
#[test]
fn a_cmd_server_on_path_starts() {
    let project = Project::new(None);
    let bin = tempfile::tempdir().expect("create shim dir");
    fs::write(
        bin.path().join("glyph-cmd-server.cmd"),
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
    let mut glyph = Glyph::spawn_in_with_config_and_env(
        project.dir.path(),
        &rust_server("glyph-cmd-server"),
        &env,
        &["a.rs"],
    );
    glyph.wait_for_text("Ln 1, Col 1", START);
    project.wait_for(&glyph, "textDocument/didOpen", "a.rs");
    let log = project.log();
    assert!(
        log.iter().any(|(_, m)| method(m) == "initialize"),
        "{log:#?}"
    );
    let status = status_line(&glyph);
    assert!(!status.contains("server not found"), "{status:?}");
    // Quit cleanly so the fake behind the shim exits too.
    glyph.send_keys("ctrl+q");
    glyph.wait_exit(WAIT);
}

#[test]
fn crashed_server_keeps_editing() {
    let project = Project::new(Some(r#"{"exit_on": "textDocument/didChange"}"#));
    let mut glyph = project.open(&rust_server(fake()), "a.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "a.rs");

    glyph.type_text("q");
    glyph.wait_for_text("rust: server crashed (exit code 3)", WAIT);

    glyph.type_text("rs");
    glyph.wait_for_text("qrsfn a()", WAIT);
    glyph.send_keys("ctrl+s");
    glyph.wait_for_text("saved a.rs", WAIT);
    assert_eq!(project.read("a.rs"), "qrsfn a() {}\n");
    // Nothing more reached the dead server.
    let changes = project
        .log()
        .iter()
        .filter(|(_, m)| method(m) == "textDocument/didChange")
        .count();
    assert_eq!(changes, 1);
}

// aurora's roles, for the server segment's colours.
const AURORA_OK: vt100::Color = vt100::Color::Rgb(0x7f, 0xe3, 0xc9);
const AURORA_ERR: vt100::Color = vt100::Color::Rgb(0xff, 0x6f, 0x91);
const AURORA_MUTED: vt100::Color = vt100::Color::Rgb(0x67, 0x62, 0x7d);
const AURORA_TEXT: vt100::Color = vt100::Color::Rgb(0xa6, 0xa2, 0xbb);

/// `rust_server(command)` under the aurora theme. The theme key goes first, since
/// after `[lsp.rust]` it would belong to that table.
fn aurora_server(command: &str) -> String {
    format!("theme = \"aurora\"\n{}", rust_server(command))
}

/// The status line's column of `text`, which must be on it.
fn status_col(glyph: &Glyph, text: &str) -> u16 {
    glyph
        .text_col(ROWS - 1, text)
        .unwrap_or_else(|| panic!("{text:?} on the status line: {:?}", status_line(glyph)))
}

#[test]
fn a_ready_server_shows_after_the_language() {
    let project = Project::new(None);
    let glyph = project.open(&aurora_server(fake()), "a.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "a.rs");
    glyph.wait_for_text("Rust    ● fake_lsp", WAIT);

    let status = status_line(&glyph);
    assert!(
        status.ends_with("Ln 1, Col 1    Rust    ● fake_lsp"),
        "{status:?}"
    );
    // It ends two cells from the right edge, like every right-hand segment.
    let name = status_col(&glyph, "fake_lsp");
    assert_eq!(name + 8, harness::COLS - 2);
    assert_eq!(glyph.fg_at(name - 2, ROWS - 1), AURORA_OK);
    assert_eq!(glyph.fg_at(name, ROWS - 1), AURORA_TEXT);
    assert_eq!(
        glyph.fg_at(status_col(&glyph, "Rust"), ROWS - 1),
        AURORA_TEXT
    );
}

#[test]
fn a_missing_server_shows_no_server_in_muted() {
    let project = Project::new(None);
    let mut glyph = project.open(&aurora_server("glyph-no-such-server"), "a.rs");
    glyph.wait_for_text("rust: server not found (glyph-no-such-server)", WAIT);
    // Any key clears the message, and the path and full right side come back.
    glyph.send_keys("right");
    glyph.wait_for_text("Ln 1, Col 2    Rust    ○ no server", WAIT);
    let label = status_col(&glyph, "no server");
    assert_eq!(glyph.fg_at(label - 2, ROWS - 1), AURORA_MUTED);
    assert_eq!(glyph.fg_at(label, ROWS - 1), AURORA_MUTED);
    assert!(!status_line(&glyph).contains("glyph-no-such-server"));
}

#[test]
fn a_crashed_server_shows_its_name_in_err() {
    let project = Project::new(Some(r#"{"exit_on": "textDocument/didChange"}"#));
    let mut glyph = project.open(&aurora_server(fake()), "a.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "a.rs");
    glyph.wait_for_text("● fake_lsp", WAIT);

    glyph.type_text("q");
    glyph.wait_for_text("rust: server crashed (exit code 3)", WAIT);
    glyph.send_keys("left");
    glyph.wait_for_text("Rust    ✕ fake_lsp", WAIT);
    let name = status_col(&glyph, "fake_lsp");
    assert_eq!(glyph.fg_at(name - 2, ROWS - 1), AURORA_ERR);
    assert_eq!(glyph.fg_at(name, ROWS - 1), AURORA_ERR);
}

#[test]
fn a_file_without_a_language_says_plain_text_and_has_no_server() {
    let project = Project::new(None);
    fs::write(project.dir.path().join("notes.txt"), "hi\n").expect("write notes.txt");
    let glyph = project.open(&aurora_server(fake()), "notes.txt");
    let status = status_line(&glyph);
    assert!(status.ends_with("Ln 1, Col 1    Plain text"), "{status:?}");
    assert!(!status.contains("fake_lsp"), "{status:?}");
}

#[test]
fn quitting_shuts_the_server_down() {
    let project = Project::new(None);
    let mut glyph = project.open(&rust_server(fake()), "a.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "a.rs");
    glyph.send_keys("ctrl+q");
    glyph.wait_exit(WAIT);
    glyph.wait_for_files("shutdown then exit as the last messages", WAIT, || {
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

/// The theme with the severity colours pinned, so they're known.
fn diag_config() -> String {
    format!(
        "{}[theme_overrides]\nerr = \"#ff0000\"\nworking = \"#ffaa00\"\ninfo = \"#0000ff\"\n\
         err_soft = \"#880000\"\nwarn_soft = \"#885500\"\ninfo_soft = \"#000088\"\n",
        rust_server(fake())
    )
}

const ERR: vt100::Color = vt100::Color::Rgb(0xff, 0x00, 0x00);
const WORKING: vt100::Color = vt100::Color::Rgb(0xff, 0xaa, 0x00);
const INFO: vt100::Color = vt100::Color::Rgb(0x00, 0x00, 0xff);
const ERR_SOFT: vt100::Color = vt100::Color::Rgb(0x88, 0x00, 0x00);
const WARN_SOFT: vt100::Color = vt100::Color::Rgb(0x88, 0x55, 0x00);
const INFO_SOFT: vt100::Color = vt100::Color::Rgb(0x00, 0x00, 0x88);

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

fn diag_project(script: &str) -> (Project, Glyph) {
    let project = Project::new(Some(script));
    fs::write(project.dir.path().join("c.rs"), DIAG_FILE).expect("write c.rs");
    let glyph = project.open(&diag_config(), "c.rs");
    (project, glyph)
}

// Screen rows: the tab header takes rows 0-2, so buffer line n is row n + 2. The
// gutter "   1  " is 6 cells, so char column c is screen column 6 + c.
const X_ROW: u16 = 4;
const Y_ROW: u16 = 5;
const VAR_COL: u16 = 14;

#[test]
fn diagnostics_are_underlined_marked_and_counted() {
    let script = format!(r#"{{"notify": {{"textDocument/didOpen": [{}]}}}}"#, both());
    let (_project, glyph) = diag_project(&script);
    glyph.wait_for_text("✕ 1  ⚠ 1", WAIT);

    glyph.wait_for_underlined(Y_ROW, "y", WAIT);
    glyph.wait_for_underlined(X_ROW, "x", WAIT);
    // Nothing else is underlined.
    assert_eq!(glyph.underlined_text(3), "");
    assert_eq!(glyph.underlined_text(6), "");
    // The underline is curly and in the severity's colour. vt100 keeps neither,
    // so the bytes are checked. The Windows pseudo-console re-encodes the output
    // and drops both, so there only the straight underline above reaches us.
    if cfg!(not(windows)) {
        glyph.wait_for_output(b"\x1b[58:2::255:0:0m\x1b[4:3my", WAIT);
        glyph.wait_for_output(b"\x1b[58:2::255:170:0m\x1b[4:3mx", WAIT);
    }
    // The text keeps its own colour.
    assert_ne!(glyph.fg_at(VAR_COL, Y_ROW), ERR);
    assert_ne!(glyph.fg_at(VAR_COL, X_ROW), WORKING);

    // The gutter marks both lines in their colour, and only them.
    let screen = glyph.screen();
    assert!(
        screen[usize::from(X_ROW)].starts_with("◆  2  "),
        "{screen:#?}"
    );
    assert!(
        screen[usize::from(Y_ROW)].starts_with("◆  3  "),
        "{screen:#?}"
    );
    assert!(screen[3].starts_with("   1  "), "{screen:#?}");
    assert_eq!(glyph.fg_at(0, X_ROW), WORKING);
    assert_eq!(glyph.fg_at(0, Y_ROW), ERR);

    // The counts on the status line are in the same colours.
    let status = status_line(&glyph);
    assert!(status.contains("✕ 1  ⚠ 1"), "{status:?}");
    let status_row = ROWS - 1;
    let warn = glyph.text_col(status_row, "⚠ 1").expect("warning count");
    let err = glyph.text_col(status_row, "✕ 1").expect("error count");
    assert_eq!(glyph.fg_at(warn, status_row), WORKING);
    assert_eq!(glyph.fg_at(err, status_row), ERR);
}

/// `    let x = 1;` is 14 cells and the text starts at column 6, so a lens on it
/// starts 4 cells past its end.
const LENS_COL: u16 = 6 + 14 + 4;

#[test]
fn the_lens_shows_the_most_severe_message_after_the_line() {
    // Line 2 has a warning and then an error; line 1 an information.
    let script = format!(
        r#"{{"notify": {{"textDocument/didOpen": [{}]}}}}"#,
        publish(&[
            (0, 3, 7, 3, "main is fine"),
            (1, 8, 9, 2, "unused variable: x"),
            (1, 12, 13, 1, "expected bool"),
            (2, 8, 9, 2, "unused variable: y"),
        ])
    );
    let (_project, glyph) = diag_project(&script);
    glyph.wait_for_text("◈ expected bool", WAIT);

    // The error wins over the warning before it, for the lens and the mark.
    assert_eq!(glyph.text_col(X_ROW, "◈ expected bool"), Some(LENS_COL));
    assert!(!glyph.screen()[usize::from(X_ROW)].contains("unused"));
    assert_eq!(glyph.fg_at(0, X_ROW), ERR);
    assert_eq!(glyph.fg_at(LENS_COL, X_ROW), ERR);
    assert_eq!(glyph.fg_at(LENS_COL + 2, X_ROW), ERR_SOFT);
    assert!(glyph.italic_at(LENS_COL + 2, X_ROW));
    assert!(!glyph.italic_at(LENS_COL, X_ROW));

    assert_eq!(
        glyph.text_col(Y_ROW, "◈ unused variable: y"),
        Some(LENS_COL)
    );
    assert_eq!(glyph.fg_at(LENS_COL, Y_ROW), WORKING);
    assert_eq!(glyph.fg_at(LENS_COL + 2, Y_ROW), WARN_SOFT);

    // `fn main() {` is 11 cells.
    let info_col = 6 + 11 + 4;
    assert_eq!(glyph.text_col(3, "◈ main is fine"), Some(info_col));
    assert_eq!(glyph.fg_at(0, 3), INFO);
    assert_eq!(glyph.fg_at(info_col, 3), INFO);
    assert_eq!(glyph.fg_at(info_col + 2, 3), INFO_SOFT);
    assert!(glyph.italic_at(info_col + 2, 3));
}

#[test]
fn a_long_message_is_cut_and_a_long_line_has_no_lens() {
    // Line 1 ends at column 76, so its lens starts at 80: 16 cells are left for
    // the message. Line 2 ends at 84, and a lens at 88 would be too close to the
    // right edge (100 - 12).
    let near = format!("let a = \"{}\";", "a".repeat(59));
    let far = format!("let b = \"{}\";", "b".repeat(67));
    assert_eq!((near.len(), far.len()), (70, 78));
    let message = "this message is much too long to fit";
    let script = format!(
        r#"{{"notify": {{"textDocument/didOpen": [{}]}}}}"#,
        publish(&[(0, 4, 5, 1, message), (1, 4, 5, 1, message)])
    );
    let project = Project::new(Some(&script));
    fs::write(project.dir.path().join("c.rs"), format!("{near}\n{far}\n")).expect("write c.rs");
    let glyph = project.open(&diag_config(), "c.rs");
    glyph.wait_for_text("◈ this message is…", WAIT);

    assert_eq!(glyph.text_col(3, "◈ this message is…"), Some(80));
    let screen = glyph.screen();
    assert!(
        screen[3].trim_end().ends_with("◈ this message is…"),
        "{screen:#?}"
    );
    // The long line is still marked and underlined, but has no lens.
    glyph.wait_for_underlined(4, "b", WAIT);
    assert!(screen[4].starts_with("◆  2  "), "{screen:#?}");
    assert!(!screen[4].contains('◈'), "{screen:#?}");
}

#[test]
fn mono_draws_the_lens_message_in_muted() {
    let script = format!(r#"{{"notify": {{"textDocument/didOpen": [{}]}}}}"#, both());
    let project = Project::new(Some(&script));
    fs::write(project.dir.path().join("c.rs"), DIAG_FILE).expect("write c.rs");
    let config = format!("theme = \"mono\"\n{}", rust_server(fake()));
    let glyph = project.open(&config, "c.rs");
    glyph.wait_for_text("◈ mismatched types", WAIT);

    // Mono's `muted` is the terminal's dark grey; its `err` is red.
    let muted = vt100::Color::Idx(8);
    assert_eq!(glyph.fg_at(LENS_COL, Y_ROW), vt100::Color::Idx(1));
    assert_eq!(glyph.fg_at(LENS_COL + 2, Y_ROW), muted);
    assert!(glyph.italic_at(LENS_COL + 2, Y_ROW));
    assert_eq!(glyph.fg_at(LENS_COL + 2, X_ROW), muted);
}

#[test]
fn f8_and_shift_f8_cycle_through_diagnostics() {
    let script = format!(r#"{{"notify": {{"textDocument/didOpen": [{}]}}}}"#, both());
    let (_project, mut glyph) = diag_project(&script);
    glyph.wait_for_text("✕ 1  ⚠ 1", WAIT);

    glyph.send_keys("f8");
    glyph.wait_for_cursor(VAR_COL, X_ROW, WAIT);
    glyph.wait_for_text("Ln 2, Col 9", WAIT);
    glyph.send_keys("f8");
    glyph.wait_for_cursor(VAR_COL, Y_ROW, WAIT);
    // Past the last one, back to the first.
    glyph.send_keys("f8");
    glyph.wait_for_cursor(VAR_COL, X_ROW, WAIT);
    // Before the first one, round to the last.
    glyph.send_keys("shift+f8");
    glyph.wait_for_cursor(VAR_COL, Y_ROW, WAIT);
    glyph.send_keys("shift+f8");
    glyph.wait_for_cursor(VAR_COL, X_ROW, WAIT);
}

#[test]
fn the_diagnostic_under_the_cursor_shows_its_message() {
    let script = format!(r#"{{"notify": {{"textDocument/didOpen": [{}]}}}}"#, both());
    let (_project, mut glyph) = diag_project(&script);
    glyph.wait_for_text("✕ 1  ⚠ 1", WAIT);
    assert!(!status_line(&glyph).contains("unused variable"));

    glyph.send_keys("f8");
    glyph.wait_for_text("⚠ unused variable: x", WAIT);
    glyph.send_keys("f8");
    glyph.wait_for_text("✕ mismatched types", WAIT);
    // The lenses keep both messages in the text area; the status line drops one.
    glyph.wait_for_screen("the warning gone from the status line", WAIT, |lines| {
        !lines[usize::from(ROWS - 1)].contains("unused variable")
    });
    // It holds the path slot, its glyph in the severity's colour.
    let status_row = ROWS - 1;
    assert_eq!(glyph.text_col(status_row, "✕ mismatched types"), Some(20));
    assert_eq!(glyph.fg_at(20, status_row), ERR);

    // Off the diagnostic, the message goes and the path comes back.
    glyph.send_keys("right");
    glyph.wait_for_text("Ln 3, Col 10", WAIT);
    glyph.wait_for_screen("the error gone from the status line", WAIT, |lines| {
        !lines[usize::from(ROWS - 1)].contains("mismatched types")
    });
    assert_eq!(glyph.text_col(status_row, "c.rs"), Some(20));
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
    let (project, mut glyph) = diag_project(&script);
    glyph.wait_for_text("✕ 1  ⚠ 1", WAIT);
    glyph.wait_for_underlined(X_ROW, "x", WAIT);

    // Typing on line 1 sends a change; the server's new set has only the error.
    glyph.type_text("z");
    project.wait_for(&glyph, "textDocument/didChange", "c.rs");
    glyph.wait_for_text("✕ 1  ⚠ 0", WAIT);
    glyph.wait_for_underlined(X_ROW, "", WAIT);
    glyph.wait_for_underlined(Y_ROW, "y", WAIT);
    assert!(glyph.screen()[usize::from(X_ROW)].starts_with("   2  "));

    // Saving gets an empty publish: everything goes.
    glyph.send_keys("ctrl+s");
    glyph.wait_for_text("saved c.rs", WAIT);
    glyph.wait_for_text_gone("✕", WAIT);
    glyph.wait_for_underlined(Y_ROW, "", WAIT);
    assert!(glyph.screen()[usize::from(Y_ROW)].starts_with("   3  "));
    assert!(!status_line(&glyph).contains("⚠"));
}

/// The fake's answer to every `textDocument/definition`: `greet` in `util.rs`.
const DEFINITION_IN_UTIL: &str = r#"{"responses": {"textDocument/definition": {
    "uri": "$dir/util.rs",
    "range": {"start": {"line": 2, "character": 7}, "end": {"line": 2, "character": 12}}
}}}"#;

/// A project holding the `tests/fixtures/definition` files, with `main.rs` open:
/// `util::greet()` on its line 4 calls `greet`, defined on `util.rs` line 3.
fn definition_project(script: &str) -> (Project, Glyph) {
    let project = Project::new(Some(script));
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/definition");
    for name in ["main.rs", "util.rs"] {
        fs::copy(fixtures.join(name), project.dir.path().join(name)).expect("copy fixture");
    }
    let glyph = project.open(&rust_server(fake()), "main.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "main.rs");
    (project, glyph)
}

// `greet` in `    util::greet();` starts at char 10 of line 4, which is screen
// column 6 + 10 past the gutter, on row 6 below the tab header.
const GREET_COL: u16 = 16;
const GREET_ROW: u16 = 6;

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
    let (project, mut glyph) = definition_project(DEFINITION_IN_UTIL);
    // One key at a time: a burst of keys can read as a paste on Windows.
    for (key, at) in [
        ("down", "Ln 2, Col 1"),
        ("down", "Ln 3, Col 1"),
        ("down", "Ln 4, Col 1"),
        ("ctrl+right", "Ln 4, Col 9"),
        ("ctrl+right", "Ln 4, Col 11"),
    ] {
        glyph.send_keys(key);
        glyph.wait_for_text(at, WAIT);
    }

    glyph.send_keys("f12");
    project.wait_for(&glyph, "textDocument/definition", "main.rs");
    assert_eq!(definition_position(&project), (3, 10));
    // util.rs opens in a tab of its own, cursor on `greet`.
    glyph.wait_for_text("Ln 3, Col 8", WAIT);
    let tabs = glyph.screen()[1].clone();
    assert!(
        tabs.contains("main.rs") && tabs.contains("util.rs"),
        "{tabs:?}"
    );
    glyph.wait_for_cursor(6 + 7, 5, WAIT);
    assert!(status_line(&glyph).contains("util.rs"));

    glyph.send_keys("alt+left");
    glyph.wait_for_text("Ln 4, Col 11", WAIT);
    glyph.wait_for_cursor(GREET_COL, GREET_ROW, WAIT);
    assert!(status_line(&glyph).contains("main.rs"));

    // Nothing more to go back to: it stays put.
    glyph.send_keys("alt+left");
    glyph.send_keys("right");
    glyph.wait_for_text("Ln 4, Col 12", WAIT);
    assert!(status_line(&glyph).contains("main.rs"));
}

#[test]
fn ctrl_click_goes_to_the_definition() {
    let (project, mut glyph) = definition_project(DEFINITION_IN_UTIL);
    glyph.ctrl_click(GREET_COL + 2, GREET_ROW);
    project.wait_for(&glyph, "textDocument/definition", "main.rs");
    assert_eq!(definition_position(&project), (3, 12));
    glyph.wait_for_text("Ln 3, Col 8", WAIT);
    assert!(status_line(&glyph).contains("util.rs"));

    // Back to where the click put the cursor.
    glyph.send_keys("alt+left");
    glyph.wait_for_text("Ln 4, Col 13", WAIT);
    assert!(status_line(&glyph).contains("main.rs"));
}

#[test]
fn no_definition_says_so() {
    // Unscripted, the fake answers `null`.
    let (project, mut glyph) = definition_project("{}");
    glyph.send_keys("f12");
    project.wait_for(&glyph, "textDocument/definition", "main.rs");
    glyph.wait_for_text("No definition found", WAIT);
    // The message takes the path slot; the cursor stays where it was.
    let status_row = ROWS - 1;
    assert_eq!(
        glyph.text_col(status_row, "⚠ No definition found"),
        Some(20)
    );
    assert!(status_line(&glyph).contains("Ln 1, Col 1"));
    assert!(!glyph.screen()[1].contains("util.rs"));
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
    let mut glyph = project.open(&rust_server(fake()), "main.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "main.rs");

    let text = glyph.text_col(3, "abcdefghij").expect("line 1 on screen");
    // 55 different places, each left by F12: lines 2 to 26 at columns 2, 4 and 6.
    let places: Vec<(u16, u16)> = (0..55u16).map(|i| (2 + i % 25, 2 + 2 * (i / 25))).collect();
    for &(line, col) in &places {
        // Line n is on screen row n + 2, below the tab header.
        glyph.click(text + col - 1, line + 2);
        glyph.wait_for_text(&format!("Ln {line}, Col {col}"), WAIT);
        glyph.send_keys("f12");
        glyph.wait_for_text("Ln 1, Col 1", WAIT);
    }

    // Back through the newest 50, newest first.
    for &(line, col) in places.iter().rev().take(50) {
        glyph.send_keys("alt+left");
        glyph.wait_for_text(&format!("Ln {line}, Col {col}"), WAIT);
    }
    // The 50th-newest is as far as it goes: the five before it were dropped.
    let (line, col) = places[5];
    for _ in 0..5 {
        glyph.send_keys("alt+left");
    }
    glyph.send_keys("right");
    glyph.wait_for_text(&format!("Ln {line}, Col {}", col + 1), WAIT);
    assert!(status_line(&glyph).contains(&format!("Ln {line}, Col {}", col + 1)));
}

/// The theme with `card` pinned, so the popup's background is known.
const CARD: vt100::Color = vt100::Color::Rgb(0x20, 0x30, 0x40);
/// The popup's border and rule colour, pinned as `line2` where a test needs it.
const LINE2: vt100::Color = vt100::Color::Rgb(0x50, 0x60, 0x70);

/// The fake's answer to every `textDocument/hover`: markdown with a code fence
/// and a paragraph too long for one line of the popup.
const HOVER_MARKDOWN: &str = r#"{"responses": {"textDocument/hover": {"contents": {
    "kind": "markdown",
    "value": "```rust\npub fn greet()\n```\n\nGreets whoever is listening, then keeps on talking for quite a while so this line wraps."
}}}}"#;

/// A definition project (cursor still at the top) with `card` and `line2`
/// pinned, its cursor moved onto `greet` on line 4.
fn hover_project(script: &str) -> (Project, Glyph) {
    let project = Project::new(Some(script));
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/definition");
    for name in ["main.rs", "util.rs"] {
        fs::copy(fixtures.join(name), project.dir.path().join(name)).expect("copy fixture");
    }
    let config = format!(
        "{}[theme_overrides]\ncard = \"#203040\"\nline2 = \"#506070\"\n",
        rust_server(fake())
    );
    let mut glyph = project.open(&config, "main.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "main.rs");
    for (key, at) in [
        ("down", "Ln 2, Col 1"),
        ("down", "Ln 3, Col 1"),
        ("down", "Ln 4, Col 1"),
        ("ctrl+right", "Ln 4, Col 9"),
        ("ctrl+right", "Ln 4, Col 11"),
    ] {
        glyph.send_keys(key);
        glyph.wait_for_text(at, WAIT);
    }
    (project, glyph)
}

// Below the cursor on `greet` (16, 6): the border on row 7, then the text one
// cell in from the border, on rows 8 to 11.
const HOVER_TEXT_COL: u16 = 18;
const HOVER_FIRST_ROW: u16 = 8;

#[test]
fn alt_k_shows_the_hover_below_the_cursor() {
    let (project, mut glyph) = hover_project(HOVER_MARKDOWN);
    glyph.send_keys("alt+k");
    project.wait_for(&glyph, "textDocument/hover", "main.rs");
    let (_, request) = project
        .log()
        .into_iter()
        .rfind(|(_, m)| is(m, "textDocument/hover", "main.rs"))
        .expect("hover request logged");
    assert_eq!(
        request["params"]["position"],
        serde_json::json!({"line": 3, "character": 10})
    );

    glyph.wait_for_text("pub fn greet()", WAIT);
    let screen = glyph.screen();
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
            glyph.text_col(row, text),
            Some(HOVER_TEXT_COL),
            "{text:?} on row {row}: {screen:#?}"
        );
    }
    assert_eq!(glyph.bg_at(HOVER_TEXT_COL, HOVER_FIRST_ROW), CARD);
    assert_eq!(glyph.bg_at(HOVER_TEXT_COL + 40, HOVER_FIRST_ROW), CARD);
    // The cursor stays in the text, on `greet`.
    glyph.wait_for_cursor(GREET_COL, GREET_ROW, WAIT);
    let status = status_line(&glyph);
    assert!(
        status.contains("main.rs") && status.contains("Ln 4, Col 11"),
        "{status:?}"
    );
}

#[test]
fn the_hover_has_a_rounded_border_a_rule_under_the_code_and_dims_nothing() {
    let (_project, mut glyph) = hover_project(HOVER_MARKDOWN);
    // Every cell left of the card, from the tab header down past its bottom.
    let cells: Vec<(u16, u16)> = (3..14)
        .flat_map(|row| (0..16).map(move |col| (col, row)))
        .collect();
    let colours = |glyph: &Glyph| -> Vec<_> {
        cells
            .iter()
            .map(|&(col, row)| (glyph.fg_at(col, row), glyph.bg_at(col, row)))
            .collect()
    };
    let before = colours(&glyph);

    glyph.send_keys("alt+k");
    glyph.wait_for_text("pub fn greet()", WAIT);
    let screen = glyph.screen();
    // The card from (16, 7) to (79, 12): 60 columns of text plus border and
    // padding, the code, the rule, two rows of docs.
    let (left, right, top, bottom) = (16, 79, HOVER_FIRST_ROW - 1, HOVER_FIRST_ROW + 4);
    for (row, ends) in [(top, ("╭", "╮")), (bottom, ("╰", "╯"))] {
        assert_eq!(glyph.text_col(row, ends.0), Some(left), "{screen:#?}");
        assert_eq!(glyph.text_col(row, ends.1), Some(right), "{screen:#?}");
        assert_eq!(glyph.fg_at(left, row), LINE2);
        assert_eq!(glyph.fg_at(left + 30, row), LINE2);
        assert_eq!(glyph.bg_at(left + 30, row), CARD);
    }
    for row in [HOVER_FIRST_ROW, HOVER_FIRST_ROW + 2] {
        assert_eq!(glyph.text_col(row, "│"), Some(left), "{screen:#?}");
        assert_eq!(glyph.fg_at(right, row), LINE2);
    }
    // The rule takes the blank line between the code and the docs, joined to
    // the side borders.
    let rule = format!("├{}┤", "─".repeat(usize::from(right - left - 1)));
    assert_eq!(
        glyph.text_col(HOVER_FIRST_ROW + 1, &rule),
        Some(left),
        "{screen:#?}"
    );
    assert_eq!(glyph.fg_at(left, HOVER_FIRST_ROW + 1), LINE2);
    assert_eq!(glyph.fg_at(left + 30, HOVER_FIRST_ROW + 1), LINE2);
    assert_eq!(glyph.bg_at(left + 30, HOVER_FIRST_ROW + 1), CARD);
    // Not a dialog: the text beside it keeps its colours.
    assert_eq!(colours(&glyph), before);
}

#[test]
fn esc_movement_and_typing_close_the_hover() {
    let (_project, mut glyph) = hover_project(HOVER_MARKDOWN);

    glyph.send_keys("alt+k");
    glyph.wait_for_text("pub fn greet()", WAIT);
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("pub fn greet()", WAIT);
    assert!(status_line(&glyph).contains("Ln 4, Col 11"));

    glyph.send_keys("alt+k");
    glyph.wait_for_text("pub fn greet()", WAIT);
    glyph.send_keys("right");
    glyph.wait_for_text("Ln 4, Col 12", WAIT);
    glyph.wait_for_text_gone("pub fn greet()", WAIT);

    glyph.send_keys("alt+k");
    glyph.wait_for_text("pub fn greet()", WAIT);
    glyph.type_text("z");
    glyph.wait_for_text("util::gzreet();", WAIT);
    glyph.wait_for_text_gone("pub fn greet()", WAIT);
}

#[test]
fn an_empty_hover_shows_nothing() {
    // The hover reply is `null`; a publish sent right after it marks when the
    // reply has been handled.
    let script = format!(
        r#"{{"notify": {{"textDocument/hover": [{}]}}}}"#,
        publish(&[(0, 0, 2, 2, "after the hover")])
    );
    let (project, mut glyph) = hover_project(&script);
    let before = glyph.screen();
    glyph.send_keys("alt+k");
    project.wait_for(&glyph, "textDocument/hover", "main.rs");
    glyph.wait_for_text("✕ 0  ⚠ 1", WAIT);

    // No card anywhere: the text area is as it was, but for the gutter mark and
    // the lens the publish put on line 1.
    let after = glyph.screen();
    let line = after[3].replacen('◆', " ", 1);
    let lens = line.find('◈').expect("a lens on line 1");
    assert_eq!(line[..lens].trim_end(), before[3].trim_end(), "{after:#?}");
    assert_eq!(&after[..3], &before[..3]);
    assert_eq!(
        &after[4..usize::from(ROWS - 1)],
        &before[4..usize::from(ROWS - 1)]
    );
    let status = status_line(&glyph);
    assert!(
        status.contains("main.rs") && status.contains("Ln 4, Col 11"),
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
    let mut glyph = project.open(&config, "long.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "long.rs");

    glyph.send_keys("ctrl+end");
    glyph.wait_for_text("Ln 40, Col 86", WAIT);
    // The gutter "  40  " is 6 cells; the last editor row is just above the status.
    let cursor = (6 + 85, ROWS - 2);
    glyph.wait_for_cursor(cursor.0, cursor.1, WAIT);

    glyph.send_keys("alt+k");
    let doc = "Returns the last value, counting back from the very end.";
    glyph.wait_for_text(doc, WAIT);
    // 56 columns of text in a 60-wide card, pushed left to end at the right
    // edge, and flipped above: border, three text rows, border ending just over
    // the cursor's row.
    let row = cursor.1 - 2;
    let col = glyph.text_col(row, doc);
    assert_eq!(col, Some(100 - 60 + 2), "{:#?}", glyph.screen());
    assert_eq!(glyph.text_col(row - 2, "fn last() -> u32"), Some(42));
    assert_eq!(glyph.bg_at(42, row), CARD);
    // The cursor's line and the status line are untouched.
    let screen = glyph.screen();
    assert!(
        screen[usize::from(cursor.1)].starts_with("  40  fn last()"),
        "{screen:#?}"
    );
    assert!(status_line(&glyph).contains("Ln 40, Col 86"));
    glyph.wait_for_cursor(cursor.0, cursor.1, WAIT);
}

/// The theme with the popup's roles pinned, so its rows' colours are known.
const COMPLETION_THEME: &str = "[theme_overrides]\ncard = \"#203040\"\n\
    accent = \"#c080ff\"\naccent2 = \"#40e0d0\"\nfg = \"#c0c4c8\"\n\
    strong = \"#f0f2f4\"\nmuted = \"#707478\"\n";
const ACCENT: vt100::Color = vt100::Color::Rgb(0xc0, 0x80, 0xff);
const FG: vt100::Color = vt100::Color::Rgb(0xc0, 0xc4, 0xc8);
const STRONG: vt100::Color = vt100::Color::Rgb(0xf0, 0xf2, 0xf4);
const MUTED: vt100::Color = vt100::Color::Rgb(0x70, 0x74, 0x78);

/// `a` moved `t` of the way to `b`, per channel, rounded, as the editor mixes.
fn mix(a: vt100::Color, b: vt100::Color, t: f64) -> vt100::Color {
    let (vt100::Color::Rgb(r1, g1, b1), vt100::Color::Rgb(r2, g2, b2)) = (a, b) else {
        panic!("can't mix {a:?} and {b:?}");
    };
    let m = |p: u8, q: u8| (f64::from(p) + (f64::from(q) - f64::from(p)) * t).round() as u8;
    vt100::Color::Rgb(m(r1, r2), m(g1, g2), m(b1, b2))
}

/// The left end of the selected row's glow: `raised` lit 30 % towards the accent.
fn glow() -> vt100::Color {
    mix(CARD, ACCENT, 0.3)
}

/// Whether `row` of a card whose rows start `shift` cells right of the usual is
/// the selected one: the glow row, its label in `strong` from column `col`.
fn is_selected(glyph: &Glyph, row: u16, shift: u16, col: u16) -> bool {
    glyph.bg_at(KIND_COL - 1 + shift, row) == glow() && glyph.fg_at(col, row) == STRONG
}

/// Initialize advertising `.` as a completion trigger, then 12 items for every
/// `textDocument/completion`, sorted by label: `len` carries a `textEdit` from
/// the cursor after `s.` on line 2 and a detail, `push` a snippet, `push_str` plain insert
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
        r#"{"label": "len", "kind": 2, "detail": "fn(&self) -> usize", "textEdit": {"range": {
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
fn completion_project(script: &str) -> (Project, Glyph) {
    completion_project_with(
        script,
        &format!("{}{COMPLETION_THEME}", rust_server(fake())),
    )
}

/// `completion_project` under `config` rather than the pinned theme.
fn completion_project_with(script: &str, config: &str) -> (Project, Glyph) {
    let project = Project::new(Some(script));
    fs::write(project.dir.path().join("comp.rs"), "fn main() {\n    \n}\n").expect("write comp.rs");
    let mut glyph = project.open(config, "comp.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "comp.rs");
    glyph.send_keys("down");
    glyph.wait_for_text("Ln 2, Col 1", WAIT);
    glyph.send_keys("end");
    glyph.wait_for_text("Ln 2, Col 5", WAIT);
    (project, glyph)
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
fn open_completion(project: &Project, glyph: &mut Glyph, line: u16) {
    let asked = completion_requests(project).len();
    glyph.type_text("s");
    glyph.wait_for_text(&format!("Ln {line}, Col 6"), WAIT);
    glyph.type_text(".");
    glyph.wait_for_files("a completion request", WAIT, || {
        completion_requests(project).len() > asked
    });
    glyph.wait_for_text("capacity", WAIT);
}

// The gutter "   1  " is 6 cells, so after `    s.` on line 2 the cursor is at
// (12, 4). The card's border is on row 5; its rows start on row 6, the kind one
// cell in from the border at column 14 and, after the 6-wide `method` and a
// space, the label at column 21.
const ITEM_ROW: u16 = 6;
const KIND_COL: u16 = 14;
const LABEL_COL: u16 = 21;

#[test]
fn a_trigger_character_or_alt_slash_shows_up_to_ten_items_with_kinds() {
    let (project, mut glyph) = completion_project(&completion_script(""));
    open_completion(&project, &mut glyph, 2);
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

    glyph.wait_for_cursor(12, 4, WAIT);
    let screen = glyph.screen();
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
            glyph.text_col(row, &format!("{kind} {label}")),
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
    // The first row is selected, the glow row; the rest are on the card, kind
    // `muted`, label `fg`.
    assert!(is_selected(&glyph, ITEM_ROW, 0, LABEL_COL), "{screen:#?}");
    assert_eq!(glyph.fg_at(KIND_COL, ITEM_ROW), MUTED);
    assert_eq!(glyph.bg_at(LABEL_COL, ITEM_ROW + 1), CARD);
    assert_eq!(glyph.fg_at(KIND_COL, ITEM_ROW + 1), MUTED);
    assert_eq!(glyph.fg_at(LABEL_COL, ITEM_ROW + 1), FG);
    // `len`'s detail, right-aligned two cells in from the card's right border:
    // 6 + 1 + 9 + 2 + 18 cells of text make the card 40 wide, from column 12.
    let len_row = ITEM_ROW + 7;
    let detail = "fn(&self) -> usize";
    assert_eq!(
        glyph.text_col(len_row, detail),
        Some(12 + 40 - 2 - 18),
        "{screen:#?}"
    );
    assert_eq!(glyph.text_col(len_row, "│"), Some(12));
    assert_eq!(
        screen[usize::from(len_row)].chars().nth(12 + 40 - 1),
        Some('│')
    );
    // The same rounded frame as the hover's.
    assert_eq!(glyph.text_col(ITEM_ROW - 1, "╭"), Some(12));
    assert_eq!(glyph.text_col(ITEM_ROW - 1, "╮"), Some(12 + 40 - 1));
    assert_eq!(glyph.text_col(ITEM_ROW + 10, "╰"), Some(12));
    assert_eq!(glyph.fg_at(12 + 40 - 2 - 18, len_row), MUTED);
    assert!(status_line(&glyph).contains("Ln 2, Col 7"));

    // Esc closes it; Alt+/ asks again from the same place.
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("capacity", WAIT);
    glyph.send_keys("alt+/");
    glyph.wait_for_text("capacity", WAIT);
    let requests = completion_requests(&project);
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[1]["params"]["position"],
        serde_json::json!({"line": 1, "character": 6})
    );
}

#[test]
fn mono_reverses_the_selected_completion_row() {
    let config = format!("theme = \"mono\"\n{}", rust_server(fake()));
    let (project, mut glyph) = completion_project_with(&completion_script(""), &config);
    open_completion(&project, &mut glyph, 2);
    glyph.wait_for_cursor(12, 4, WAIT);
    let screen = glyph.screen();
    // The selected row is reverse video across the card, kind and label in it.
    let selected = glyph.reversed_text(ITEM_ROW);
    assert!(
        selected.contains("field  as_str"),
        "{selected:?} {screen:#?}"
    );
    assert_eq!(selected.chars().count(), 40 - 2, "{selected:?}");
    // Its label keeps the row's own colours rather than `strong`, so it reads
    // as reversed like the rest of the row.
    assert_eq!(
        glyph.fg_at(LABEL_COL, ITEM_ROW),
        glyph.fg_at(KIND_COL - 1, ITEM_ROW)
    );
    assert_ne!(
        glyph.fg_at(LABEL_COL, ITEM_ROW),
        vt100::Color::Idx(15),
        "{screen:#?}"
    );
    // The next row isn't reversed: kind in `muted` (dark grey), label in `fg`
    // (the terminal's default).
    assert_eq!(glyph.reversed_text(ITEM_ROW + 1), "", "{screen:#?}");
    assert_eq!(glyph.fg_at(KIND_COL, ITEM_ROW + 1), vt100::Color::Idx(8));
    assert_eq!(glyph.fg_at(LABEL_COL, ITEM_ROW + 1), vt100::Color::Default);
}

#[test]
fn typing_filters_and_the_arrows_move_the_selection() {
    let (project, mut glyph) = completion_project(&completion_script(""));
    open_completion(&project, &mut glyph, 2);

    // Case doesn't matter: `P` keeps `push` and `push_str`.
    glyph.type_text("P");
    glyph.wait_for_text_gone("capacity", WAIT);
    glyph.wait_for_text("s.P", WAIT);
    // The card follows the cursor, one cell right now.
    let screen = glyph.screen();
    assert_eq!(
        glyph.text_col(ITEM_ROW, "method push"),
        Some(KIND_COL + 1),
        "{screen:#?}"
    );
    assert_eq!(
        glyph.text_col(ITEM_ROW + 1, "method push_str"),
        Some(KIND_COL + 1)
    );
    // The typed `p` is `accent` bold in every label; the rest of the selected
    // label, from column `LABEL_COL + 2`, is `strong`.
    assert!(
        is_selected(&glyph, ITEM_ROW, 1, LABEL_COL + 2),
        "{screen:#?}"
    );
    for row in [ITEM_ROW, ITEM_ROW + 1] {
        assert_eq!(glyph.fg_at(LABEL_COL + 1, row), ACCENT);
        assert!(glyph.bold_at(LABEL_COL + 1, row));
        assert!(!glyph.bold_at(LABEL_COL + 2, row));
    }
    assert_eq!(glyph.fg_at(LABEL_COL + 2, ITEM_ROW + 1), FG);

    glyph.send_keys("down");
    glyph.wait_for_fg_at(LABEL_COL + 2, ITEM_ROW + 1, STRONG, WAIT);
    assert!(is_selected(&glyph, ITEM_ROW + 1, 1, LABEL_COL + 2));
    assert_eq!(glyph.bg_at(LABEL_COL + 1, ITEM_ROW), CARD);
    assert_eq!(glyph.fg_at(LABEL_COL + 2, ITEM_ROW), FG);
    // The selection stops at the last item, and the cursor never moves.
    glyph.send_keys("down");
    glyph.send_keys("up");
    glyph.wait_for_fg_at(LABEL_COL + 2, ITEM_ROW, STRONG, WAIT);
    assert!(status_line(&glyph).contains("Ln 2, Col 8"));

    // A filter matching nothing hides the popup; Backspace brings it back.
    glyph.type_text("z");
    glyph.wait_for_text_gone("push", WAIT);
    glyph.send_keys("backspace");
    glyph.wait_for_text("push_str", WAIT);
}

#[test]
fn enter_and_tab_insert_the_item_as_one_undo_step() {
    let (project, mut glyph) = completion_project(&completion_script(""));

    // `len`'s textEdit, stretched over the `le` typed since.
    open_completion(&project, &mut glyph, 2);
    glyph.type_text("le");
    glyph.wait_for_text_gone("capacity", WAIT);
    glyph.wait_for_text("method len", WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text("s.len()", WAIT);
    glyph.wait_for_text_gone("method len", WAIT);
    assert!(status_line(&glyph).contains("Ln 2, Col 12"));
    // One undo takes back just the insertion.
    glyph.send_keys("ctrl+z");
    glyph.wait_for_text_gone("s.len()", WAIT);
    glyph.wait_for_text("Ln 2, Col 9", WAIT);
    let screen = glyph.screen();
    assert!(screen[4].ends_with("s.le"), "{screen:#?}");
    glyph.send_keys("ctrl+y");
    glyph.wait_for_text("s.len()", WAIT);

    // Tab takes `push`'s snippet with its placeholder as plain text.
    glyph.send_keys("enter");
    glyph.wait_for_text("Ln 3, Col 5", WAIT);
    open_completion(&project, &mut glyph, 3);
    glyph.type_text("pu");
    glyph.wait_for_text_gone("capacity", WAIT);
    glyph.send_keys("tab");
    glyph.wait_for_text("s.push(ch)", WAIT);

    // `push_str`'s insert text, picked with Down. The cursor is on row 6, so the
    // card's rows start on row 8.
    glyph.send_keys("enter");
    glyph.wait_for_text("Ln 4, Col 5", WAIT);
    open_completion(&project, &mut glyph, 4);
    glyph.type_text("p");
    glyph.wait_for_text_gone("capacity", WAIT);
    glyph.send_keys("down");
    glyph.wait_for_fg_at(LABEL_COL + 2, 9, STRONG, WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text("s.push_str", WAIT);

    // `trim` has only its label.
    glyph.send_keys("enter");
    glyph.wait_for_text("Ln 5, Col 5", WAIT);
    open_completion(&project, &mut glyph, 5);
    glyph.type_text("tr");
    glyph.wait_for_text_gone("capacity", WAIT);
    glyph.wait_for_text("fn trim", WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text_gone("fn trim", WAIT);
    glyph.wait_for_text("s.trim", WAIT);

    glyph.send_keys("ctrl+s");
    glyph.wait_for_text("saved", WAIT);
    assert_eq!(
        project.read("comp.rs"),
        "fn main() {\n    s.len()\n    s.push(ch)\n    s.push_str\n    s.trim\n}\n"
    );
}

#[test]
fn esc_dismisses_the_popup_without_changing_the_buffer() {
    let (project, mut glyph) = completion_project(&completion_script(""));
    open_completion(&project, &mut glyph, 2);
    let before = glyph.screen()[4].clone();
    glyph.send_keys("esc");
    glyph.wait_for_text_gone("capacity", WAIT);
    let screen = glyph.screen();
    assert_eq!(screen[4], before);
    assert!(screen[4].ends_with("    s."), "{screen:#?}");
    assert!(status_line(&glyph).contains("Ln 2, Col 7"));
    // Enter is the editor's again.
    glyph.send_keys("enter");
    glyph.wait_for_text("Ln 3, Col 5", WAIT);
    glyph.send_keys("ctrl+s");
    glyph.wait_for_text("saved", WAIT);
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
    let (project, mut glyph) = completion_project(&completion_script(&extra));
    glyph.type_text("s");
    glyph.wait_for_text("Ln 2, Col 6", WAIT);
    glyph.type_text(".");
    project.wait_for(&glyph, "textDocument/completion", "comp.rs");
    glyph.send_keys("down");
    glyph.wait_for_text("Ln 3, Col 2", WAIT);

    glyph.wait_for_text("✕ 0  ⚠ 1", WAIT);
    let screen = glyph.screen();
    assert!(
        !screen.iter().any(|r| r.contains("capacity")),
        "{screen:#?}"
    );
    assert!(status_line(&glyph).contains("Ln 3, Col 2"));
    // Back on the word, the dropped answer doesn't come back either.
    glyph.send_keys("up");
    glyph.wait_for_text("Ln 2, Col 7", WAIT);
    let screen = glyph.screen();
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
    let mut glyph = project.open(&format_config(true), "a.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "a.rs");

    glyph.send_keys("ctrl+s");
    project.wait_for(&glyph, "textDocument/didSave", "a.rs");
    glyph.wait_for_text("fn a() {}", WAIT);
    glyph.wait_for_text("saved a.rs", WAIT);
    assert_eq!(project.read("a.rs"), "fn a() {}\n");
    let status = status_line(&glyph);
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
    glyph.send_keys("ctrl+z");
    glyph.wait_for_text("fn  a(){}", WAIT);
    glyph.wait_for_text("a.rs •", WAIT);
}

#[test]
fn a_server_that_never_answers_saves_unformatted_after_two_seconds() {
    let project = Project::new(Some(r#"{"silent": ["textDocument/formatting"]}"#));
    let mut glyph = project.open(&format_config(true), "a.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "a.rs");
    glyph.type_text("x");
    glyph.wait_for_text("a.rs •", WAIT);

    glyph.send_keys("ctrl+s");
    project.wait_for(&glyph, "textDocument/formatting", "a.rs");
    glyph.wait_for_text("saved a.rs unformatted (no answer in 2 s)", WAIT);
    assert_eq!(project.read("a.rs"), "xfn a() {}\n");
    glyph.wait_for_text_gone("a.rs •", WAIT);
}

#[test]
fn a_formatting_error_saves_unformatted_and_says_so() {
    let script = r#"{"errors": {"textDocument/formatting": {"code": -32603, "message": "boom"}}}"#;
    let project = Project::new(Some(script));
    let mut glyph = project.open(&format_config(true), "a.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "a.rs");
    glyph.type_text("y");
    glyph.wait_for_text("a.rs •", WAIT);

    glyph.send_keys("ctrl+s");
    glyph.wait_for_text("saved a.rs unformatted (server error: boom)", WAIT);
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
    let mut glyph = project.open(&rust_server(fake()), "a.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "a.rs");
    glyph.type_text("z");
    glyph.wait_for_text("a.rs •", WAIT);

    glyph.send_keys("ctrl+s");
    project.wait_for(&glyph, "textDocument/didSave", "a.rs");
    glyph.wait_for_text("saved a.rs", WAIT);
    assert_eq!(project.read("a.rs"), "zfn a() {}\n");
    assert!(format_requests(&project).is_empty(), "{:#?}", project.log());

    // Explicitly off behaves the same.
    let project = Project::new(Some(script));
    let mut glyph = project.open(&format_config(false), "a.rs");
    project.wait_for(&glyph, "textDocument/didOpen", "a.rs");
    glyph.send_keys("ctrl+s");
    project.wait_for(&glyph, "textDocument/didSave", "a.rs");
    assert!(format_requests(&project).is_empty(), "{:#?}", project.log());
}
