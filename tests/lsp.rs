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
