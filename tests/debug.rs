//! Starting and stopping a debug session against `fake_dap`, the scripted
//! adapter in `src/bin/fake_dap.rs`, driven through the real binary
//! (glyph-debugger spec D4, D5).

mod harness;

use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use harness::{Glyph, ROWS};
use serde_json::{Value, json};
use tempfile::TempDir;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(10);

/// Aurora's `accent2`, which the paused line is marked in.
const ACCENT2: vt100::Color = vt100::Color::Rgb(0x6e, 0xe7, 0xd8);

fn fake() -> &'static str {
    env!("CARGO_BIN_EXE_fake_dap")
}

/// The screen row of buffer line `line` (1-based) while the view is at the top:
/// the editor starts below the three rows of the tab header.
fn row(line: u16) -> u16 {
    line + 2
}

fn status_line(screen: &[String]) -> &str {
    &screen[usize::from(ROWS - 1)]
}

/// A Python project holding `main.py` and `other.py`, with the fake's log and
/// script outside it so the tree never shows them.
struct Project {
    dir: TempDir,
    files: TempDir,
}

impl Project {
    /// `script` is the fake's script; `glyph_toml` the project's `.glyph.toml`.
    fn new(script: Value, glyph_toml: &str) -> Self {
        let dir = tempfile::tempdir().expect("create project dir");
        fs::write(dir.path().join("main.py"), "a = 1\nb = 2\nc = 3\n").expect("write main.py");
        fs::write(dir.path().join("other.py"), "x = 0\n").expect("write other.py");
        if !glyph_toml.is_empty() {
            fs::write(dir.path().join(".glyph.toml"), glyph_toml).expect("write .glyph.toml");
        }
        let files = tempfile::tempdir().expect("create log dir");
        fs::write(files.path().join("script.json"), script.to_string()).expect("write script");
        Self { dir, files }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn log_path(&self) -> PathBuf {
        self.files.path().join("log.jsonl")
    }

    /// `config.toml` pointing Python at `adapter`, with the fake's log and
    /// script as its arguments. Literal strings, so Windows backslashes stay.
    fn config(&self, adapter: &str) -> String {
        let log = self.log_path();
        let script = self.files.path().join("script.json");
        format!(
            "theme = \"aurora\"\n[debug.python]\nadapter = '{adapter}'\nargs = ['--log', '{}', '--script', '{}']\n",
            log.display(),
            script.display()
        )
    }

    /// Glyph on `file` with the fake as Python's adapter.
    fn open(&self, file: &str) -> Glyph {
        self.open_with(fake(), file)
    }

    fn open_with(&self, adapter: &str, file: &str) -> Glyph {
        let glyph = Glyph::spawn_in_with_config(self.dir.path(), &self.config(adapter), &[file]);
        glyph.wait_for_text("Ln 1, Col 1", START);
        glyph
    }

    /// Every request the fake received, in order.
    fn requests(&self) -> Vec<Value> {
        let Ok(text) = fs::read_to_string(self.log_path()) else {
            return Vec::new();
        };
        text.lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .map(|entry| entry["message"].clone())
            .filter(|message| message["type"] == "request")
            .collect()
    }

    fn commands(&self) -> Vec<String> {
        self.requests()
            .iter()
            .filter_map(|r| r["command"].as_str().map(str::to_string))
            .collect()
    }

    /// Waits until the fake has received `count` requests named `command`.
    fn wait_for(&self, glyph: &Glyph, command: &str, count: usize) {
        glyph.wait_for_files(&format!("{count} × {command}"), WAIT, || {
            self.commands().iter().filter(|c| *c == command).count() >= count
        });
    }
}

/// Lines of `setBreakpoints` request `request`, and the file it names.
fn breakpoints(request: &Value) -> (String, Vec<u64>) {
    let path = request["arguments"]["source"]["path"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let lines = request["arguments"]["breakpoints"]
        .as_array()
        .map(|list| list.iter().filter_map(|b| b["line"].as_u64()).collect())
        .unwrap_or_default();
    (path, lines)
}

fn wait_for_status(glyph: &Glyph, text: &str) {
    glyph.wait_for_screen(&format!("{text:?} in the status line"), WAIT, |screen| {
        status_line(screen).contains(text)
    });
}

#[test]
fn alt_f5_builds_then_launches_with_every_breakpoint_in_order() {
    let project = Project::new(json!({}), "[debug]\nbuild = \"echo built\"\n");
    let mut glyph = project.open("main.py");
    glyph.send_keys("down");
    glyph.wait_for_text("Ln 2, Col 1", WAIT);
    glyph.send_keys("f9");
    // A second file, so the breakpoints go file by file.
    glyph.send_keys("ctrl+p");
    glyph.wait_for_text("cast", WAIT);
    glyph.type_text("other.py");
    glyph.wait_for_text("other.py  ", WAIT);
    glyph.send_keys("enter");
    glyph.wait_for_text("1  x = 0", WAIT);
    glyph.send_keys("f9");

    glyph.send_keys("alt+f5");
    wait_for_status(&glyph, "● debugging");
    glyph.wait_for_text("debug other.py", WAIT);
    project.wait_for(&glyph, "configurationDone", 1);
    assert_eq!(
        project.commands(),
        [
            "initialize",
            "launch",
            "setBreakpoints",
            "setBreakpoints",
            "configurationDone"
        ]
    );
    let requests = project.requests();
    assert_eq!(
        requests[1]["arguments"]["program"],
        json!(project.path("other.py"))
    );
    let sent: Vec<(String, Vec<u64>)> = requests[2..4].iter().map(breakpoints).collect();
    let file = |name: &str| project.path(name).display().to_string();
    assert_eq!(
        sent,
        [(file("main.py"), vec![2]), (file("other.py"), vec![1])]
    );
}

#[test]
fn a_failed_build_stops_there() {
    let project = Project::new(json!({}), "[debug]\nbuild = \"exit 3\"\n");
    let mut glyph = project.open("main.py");
    glyph.send_keys("alt+f5");
    wait_for_status(&glyph, "build failed");
    // The run panel keeps the build's own result.
    glyph.wait_for_text("exited 3", WAIT);
    assert!(project.commands().is_empty(), "{:?}", project.commands());
    assert!(!status_line(&glyph.screen()).contains("debugging"));
}

#[test]
fn program_output_goes_to_the_run_panel_titled_debug_program() {
    let output = json!({"event": "output", "body": {"category": "stdout", "output": "hello from the program\n"}});
    let project = Project::new(json!({"events": {"configurationDone": [output]}}), "");
    let mut glyph = project.open("main.py");
    glyph.send_keys("alt+f5");
    glyph.wait_for_text("debug main.py", WAIT);
    glyph.wait_for_text("hello from the program", WAIT);
    wait_for_status(&glyph, "● debugging");
}

#[test]
fn a_stop_opens_the_top_frames_file_and_marks_the_paused_line() {
    let project = Project::new(json!({}), "");
    // The script names the project's own main.py, so it's written once the
    // project exists.
    let script = json!({
        "events": {"configurationDone": [
            {"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1}}
        ]},
        "responses": {"stackTrace": {"stackFrames": [
            {"id": 1, "name": "<module>", "source": {"path": project.path("main.py")}, "line": 3, "column": 1}
        ]}},
    });
    fs::write(project.files.path().join("script.json"), script.to_string()).expect("write script");

    let mut glyph = project.open("other.py");
    glyph.send_keys("alt+f5");
    wait_for_status(&glyph, "‖ paused main.py:3");
    wait_for_status(&glyph, "Ln 3, Col 1");
    glyph.wait_for_text("3  c = 3", WAIT);
    glyph.wait_for_screen("▶ on line 3", WAIT, |screen| {
        screen[usize::from(row(3))].starts_with('▶')
    });
    glyph.wait_for_fg_at(0, row(3), ACCENT2, WAIT);
    // Only the paused line is marked.
    let screen = glyph.screen();
    assert!(!screen[usize::from(row(2))].starts_with('▶'));
    // The stack was asked of the thread that stopped.
    let requests = project.requests();
    let stack = requests
        .iter()
        .find(|r| r["command"] == "stackTrace")
        .expect("a stackTrace request");
    assert_eq!(stack["arguments"]["threadId"], 1);
}

#[test]
fn alt_f6_disconnects_and_ends_the_session() {
    let project = Project::new(json!({}), "");
    let mut glyph = project.open("main.py");
    glyph.send_keys("alt+f5");
    wait_for_status(&glyph, "● debugging");
    project.wait_for(&glyph, "configurationDone", 1);

    glyph.send_keys("alt+f6");
    wait_for_status(&glyph, "debugging stopped");
    project.wait_for(&glyph, "disconnect", 1);
    let requests = project.requests();
    let disconnect = requests
        .iter()
        .find(|r| r["command"] == "disconnect")
        .expect("a disconnect request");
    assert_eq!(disconnect["arguments"]["terminateDebuggee"], true);
    glyph.wait_for_text("stopped", WAIT);
    assert!(!status_line(&glyph.screen()).contains("● debugging"));

    // Nothing is left to stop.
    glyph.send_keys("alt+f6");
    wait_for_status(&glyph, "not debugging");
}

#[test]
fn the_program_exiting_ends_the_session_with_its_code() {
    let script = json!({"events": {"configurationDone": [
        {"event": "exited", "body": {"exitCode": 4}},
        {"event": "terminated"}
    ]}});
    let project = Project::new(script, "");
    let mut glyph = project.open("main.py");
    glyph.send_keys("alt+f5");
    wait_for_status(&glyph, "exited 4");
    assert!(!status_line(&glyph.screen()).contains("debugging"));
    // The session is over: the adapter was let go.
    project.wait_for(&glyph, "disconnect", 1);
}

#[test]
fn a_lone_terminated_ends_the_session_without_calling_it_a_failure() {
    // Some adapters end with `terminated` and never send an exit code.
    let script = json!({"events": {"configurationDone": [
        {"event": "terminated"}
    ]}});
    let project = Project::new(script, "");
    let mut glyph = project.open("main.py");
    glyph.send_keys("alt+f5");
    wait_for_status(&glyph, "program ended");
    glyph.wait_for_text("debug main.py  ended", WAIT);
    let screen = glyph.screen();
    assert!(!status_line(&screen).contains("debugging"));
    assert!(!screen.iter().any(|line| line.contains('✕')), "{screen:#?}");
    project.wait_for(&glyph, "disconnect", 1);
}

#[test]
fn breakpoints_toggled_during_a_session_are_sent() {
    let project = Project::new(json!({}), "");
    let mut glyph = project.open("main.py");
    glyph.send_keys("alt+f5");
    wait_for_status(&glyph, "● debugging");
    project.wait_for(&glyph, "configurationDone", 1);
    // No breakpoints at the start, so none were sent.
    assert!(!project.commands().contains(&"setBreakpoints".to_string()));

    glyph.send_keys("f9");
    project.wait_for(&glyph, "setBreakpoints", 1);
    glyph.send_keys("f9");
    project.wait_for(&glyph, "setBreakpoints", 2);
    let sent: Vec<(String, Vec<u64>)> = project
        .requests()
        .iter()
        .filter(|r| r["command"] == "setBreakpoints")
        .map(breakpoints)
        .collect();
    let main = project.path("main.py").display().to_string();
    assert_eq!(sent, [(main.clone(), vec![1]), (main, vec![])]);
}

#[test]
fn a_language_without_an_adapter_has_no_debugger() {
    let project = Project::new(json!({}), "");
    fs::write(project.path("app.ts"), "let a = 1;\n").expect("write app.ts");
    let mut glyph = project.open("app.ts");
    glyph.send_keys("alt+f5");
    wait_for_status(&glyph, "no debugger for typescript");
}

#[test]
fn a_missing_adapter_is_one_message_and_editing_carries_on() {
    let project = Project::new(json!({}), "");
    let missing = project.files.path().join("no-such-adapter");
    let mut glyph = project.open_with(&missing.display().to_string(), "main.py");
    glyph.send_keys("alt+f5");
    // The message names the adapter by its path, too long to show whole here.
    wait_for_status(&glyph, "✕ can't start");
    glyph.wait_for_screen("the session gone", WAIT, |screen| {
        !status_line(screen).contains("debugging")
    });
    glyph.type_text("z");
    glyph.wait_for_text("1  za = 1", WAIT);
}

#[test]
fn a_crashed_adapter_is_one_message_and_editing_carries_on() {
    let project = Project::new(json!({"exit_on": "launch"}), "");
    let mut glyph = project.open("main.py");
    glyph.send_keys("alt+f5");
    wait_for_status(&glyph, "debug adapter exited (3)");
    assert!(!status_line(&glyph.screen()).contains("debugging"));
    glyph.type_text("z");
    glyph.wait_for_text("1  za = 1", WAIT);
}
