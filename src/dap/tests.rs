//! The DAP session against `fake_dap`, the scripted adapter in
//! `src/bin/fake_dap.rs`, run as a real child process.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use super::*;

/// The scripted fake adapter. Cargo builds it beside this binary's `deps` folder
/// whenever it builds the integration tests, which `cargo test` does.
fn fake_dap() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let dir = exe.parent().and_then(Path::parent).unwrap();
    let path = dir.join(format!("fake_dap{}", std::env::consts::EXE_SUFFIX));
    assert!(
        path.exists(),
        "no {}: run `cargo build --bins`",
        path.display()
    );
    path
}

/// A session with the fake, logging what it receives to `log.jsonl` and following
/// `script`.
struct Fake {
    session: Session,
    events: UnboundedReceiver<AppEvent>,
    files: tempfile::TempDir,
}

impl Fake {
    fn new(script: Value) -> Self {
        let files = tempfile::tempdir().unwrap();
        let log = files.path().join("log.jsonl");
        let path = files.path().join("script.json");
        std::fs::write(&path, script.to_string()).unwrap();
        let args = [
            "--log".to_string(),
            log.display().to_string(),
            "--script".to_string(),
            path.display().to_string(),
        ];
        Self::start(&fake_dap(), &args, files)
    }

    fn start(program: &Path, args: &[String], files: tempfile::TempDir) -> Self {
        let (tx, events) = unbounded_channel();
        let session = Session::start(7, program, args, files.path(), tx);
        Self {
            session,
            events,
            files,
        }
    }

    /// Handles adapter events, as the app loop does, until `count` pieces of news
    /// have come in, and returns them.
    async fn news(&mut self, count: usize) -> Vec<DapNews> {
        let mut news = Vec::new();
        let waited = tokio::time::timeout(Duration::from_secs(10), async {
            while news.len() < count {
                match self.events.recv().await {
                    Some(AppEvent::Dap(event)) => news.extend(self.session.handle(event)),
                    Some(_) => {}
                    None => panic!("the app channel closed"),
                }
            }
        })
        .await;
        assert!(
            waited.is_ok(),
            "timed out waiting on the fake adapter: {news:?}"
        );
        news
    }

    /// Whatever news arrives in the next moment, for checking that nothing does.
    async fn more_news(&mut self) -> Vec<DapNews> {
        let mut news = Vec::new();
        while let Ok(Some(AppEvent::Dap(event))) =
            tokio::time::timeout(Duration::from_millis(300), self.events.recv()).await
        {
            news.extend(self.session.handle(event));
        }
        news
    }

    /// Every message the fake received, in order.
    fn log(&self) -> Vec<Value> {
        let text = std::fs::read_to_string(self.files.path().join("log.jsonl")).unwrap_or_default();
        text.lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .map(|entry| entry["message"].clone())
            .collect()
    }
}

#[tokio::test]
async fn a_whole_session_from_initialize_to_disconnect() {
    let frames = json!({"stackFrames": [
        {"id": 100, "name": "main", "source": {"name": "main.rs", "path": "/p/src/main.rs"},
         "line": 4, "column": 5},
        {"id": 101, "name": "start", "line": 0, "column": 0}
    ]});
    let mut fake = Fake::new(json!({
        "responses": {
            "stackTrace": frames,
            "scopes": {"scopes": [{"name": "Locals", "variablesReference": 5}]},
            "variables": {"variables": [
                {"name": "n", "value": "3", "type": "i32", "variablesReference": 0},
                {"name": "v", "value": "Vec(2)", "type": "Vec<i32>", "variablesReference": 6}
            ]}
        },
        "events": {
            "configurationDone": [
                {"event": "output", "body": {"category": "stdout", "output": "hi\n"}},
                {"event": "stopped", "body": {"reason": "breakpoint", "threadId": 1}}
            ],
            "next": [{"event": "stopped", "body": {"reason": "step", "threadId": 1}}],
            "continue": [
                {"event": "exited", "body": {"exitCode": 2}},
                {"event": "terminated"}
            ]
        }
    }));
    let session = &mut fake.session;
    let main_rs = Path::new("/p/src/main.rs");

    session.initialize("fake").unwrap();
    let news = fake.news(2).await;
    assert_eq!(
        news,
        [
            DapNews::Capabilities(json!({"supportsConfigurationDoneRequest": true})),
            DapNews::Initialized
        ]
    );

    let session = &mut fake.session;
    session.set_breakpoints(main_rs, &[4, 9]).unwrap();
    session
        .launch(json!({"program": "/p/target/debug/p"}))
        .unwrap();
    session.configuration_done().unwrap();
    let news = fake.news(5).await;
    let verified = |line| Breakpoint {
        verified: true,
        line: Some(line),
        message: None,
    };
    assert_eq!(
        news,
        [
            DapNews::Breakpoints {
                path: main_rs.to_path_buf(),
                breakpoints: vec![verified(4), verified(9)]
            },
            DapNews::Launched,
            DapNews::ConfigurationDone,
            DapNews::Output {
                category: "stdout".to_string(),
                text: "hi\n".to_string()
            },
            DapNews::Stopped {
                reason: "breakpoint".to_string(),
                thread: Some(1),
                text: None
            },
        ]
    );

    let session = &mut fake.session;
    session.threads().unwrap();
    session.stack_trace(1).unwrap();
    session.scopes(100).unwrap();
    session.variables(5).unwrap();
    let news = fake.news(4).await;
    assert_eq!(
        news[0],
        DapNews::Threads(vec![Thread {
            id: 1,
            name: "main".to_string()
        }])
    );
    let DapNews::StackTrace { thread: 1, frames } = &news[1] else {
        panic!("{news:?}");
    };
    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].id, 100);
    assert_eq!(frames[0].line, 4);
    let source = frames[0].source.clone().unwrap();
    assert_eq!(source.path.as_deref(), Some(main_rs));
    assert_eq!(frames[1].source, None);
    assert_eq!(
        news[2],
        DapNews::Scopes {
            frame: 100,
            scopes: vec![Scope {
                name: "Locals".to_string(),
                variables_reference: 5,
                expensive: false
            }]
        }
    );
    let DapNews::Variables {
        reference: 5,
        variables,
    } = &news[3]
    else {
        panic!("{news:?}");
    };
    assert_eq!(variables[0].name, "n");
    assert_eq!(variables[0].value, "3");
    assert_eq!(variables[0].kind.as_deref(), Some("i32"));
    assert_eq!(variables[1].variables_reference, 6);

    let session = &mut fake.session;
    session.step_in(1).unwrap();
    session.step_out(1).unwrap();
    session.next(1).unwrap();
    let news = fake.news(4).await;
    assert_eq!(
        news,
        [
            DapNews::Resumed("stepIn".to_string()),
            DapNews::Resumed("stepOut".to_string()),
            DapNews::Resumed("next".to_string()),
            DapNews::Stopped {
                reason: "step".to_string(),
                thread: Some(1),
                text: None
            },
        ]
    );

    fake.session.resume(1).unwrap();
    let news = fake.news(3).await;
    assert_eq!(
        news,
        [
            DapNews::Resumed("continue".to_string()),
            DapNews::Exited(2),
            DapNews::Terminated
        ]
    );

    fake.session.disconnect().unwrap();
    assert_eq!(fake.news(1).await, [DapNews::Ended]);
    assert!(fake.session.is_over());
    assert_eq!(fake.session.threads(), None, "nothing to send to");

    let log = fake.log();
    let commands: Vec<&str> = log.iter().filter_map(|m| m["command"].as_str()).collect();
    assert_eq!(
        commands,
        [
            "initialize",
            "setBreakpoints",
            "launch",
            "configurationDone",
            "threads",
            "stackTrace",
            "scopes",
            "variables",
            "stepIn",
            "stepOut",
            "next",
            "continue",
            "disconnect"
        ]
    );
    let seqs: Vec<i64> = log.iter().filter_map(|m| m["seq"].as_i64()).collect();
    assert_eq!(
        seqs,
        (1..=13).collect::<Vec<_>>(),
        "seq counts each request"
    );
    assert!(log.iter().all(|m| m["type"] == "request"));
    assert_eq!(log[0]["arguments"]["adapterID"], "fake");
    assert_eq!(log[0]["arguments"]["linesStartAt1"], true);
    assert_eq!(
        log[1]["arguments"]["breakpoints"],
        json!([{"line": 4}, {"line": 9}])
    );
    assert_eq!(log[1]["arguments"]["source"]["path"], json!(main_rs));
    assert_eq!(log[2]["arguments"]["program"], "/p/target/debug/p");
    assert_eq!(log[5]["arguments"]["threadId"], 1);
    assert_eq!(log[6]["arguments"]["frameId"], 100);
    assert_eq!(log[7]["arguments"]["variablesReference"], 5);
    assert_eq!(log[12]["arguments"]["terminateDebuggee"], true);
}

#[tokio::test]
async fn responses_are_matched_by_request_seq_not_by_order() {
    let mut fake = Fake::new(json!({
        "hold": ["scopes"],
        "responses": {
            "scopes": {"scopes": [{"name": "Locals", "variablesReference": 5}]},
            "variables": {"variables": [{"name": "n", "value": "3"}]}
        }
    }));
    fake.session.scopes(100).unwrap();
    fake.session.variables(5).unwrap();
    let news = fake.news(2).await;
    let DapNews::Variables {
        reference: 5,
        variables,
    } = &news[0]
    else {
        panic!("variables answered first: {news:?}");
    };
    assert_eq!(variables[0].name, "n");
    let DapNews::Scopes { frame: 100, scopes } = &news[1] else {
        panic!("{news:?}");
    };
    assert_eq!(scopes[0].name, "Locals");
}

#[tokio::test]
async fn a_refused_request_says_why() {
    let mut fake = Fake::new(json!({"errors": {"launch": "no such program"}}));
    fake.session.launch(json!({"program": "nope"})).unwrap();
    fake.session.threads().unwrap();
    let news = fake.news(2).await;
    assert_eq!(
        news[0],
        DapNews::Refused {
            command: "launch".to_string(),
            message: "no such program".to_string()
        }
    );
    assert!(matches!(news[1], DapNews::Threads(_)), "{news:?}");
    assert!(!fake.session.is_over());
}

#[tokio::test]
async fn a_missing_adapter_is_one_failure() {
    let files = tempfile::tempdir().unwrap();
    let missing = files.path().join("no-such-adapter");
    let mut fake = Fake::start(&missing, &[], files);
    // Requests to an adapter that never came are dropped, not queued forever.
    assert_eq!(fake.session.initialize("fake"), None);
    let news = fake.news(1).await;
    let [DapNews::Failed(why)] = news.as_slice() else {
        panic!("{news:?}");
    };
    assert!(why.contains("no-such-adapter"), "{why}");
    assert!(fake.session.is_over());
    assert_eq!(fake.more_news().await, []);
}

#[tokio::test]
async fn a_crash_is_one_failure_however_much_was_pending() {
    let mut fake = Fake::new(json!({"exit_on": "launch"}));
    fake.session.initialize("fake").unwrap();
    fake.session.launch(json!({})).unwrap();
    fake.session.threads().unwrap();
    let news = fake.news(3).await;
    assert!(matches!(news[0], DapNews::Capabilities(_)), "{news:?}");
    assert_eq!(news[1], DapNews::Initialized);
    assert_eq!(
        news[2],
        DapNews::Failed("debug adapter exited (3)".to_string())
    );
    assert!(fake.session.is_over());
    assert_eq!(fake.more_news().await, []);
    assert_eq!(fake.session.threads(), None);
}

#[tokio::test]
async fn a_reverse_request_is_refused() {
    let mut fake = Fake::new(json!({
        "events": {"launch": [{"type": "request", "command": "runInTerminal",
                               "arguments": {"args": ["p"]}}]}
    }));
    fake.session.launch(json!({})).unwrap();
    assert_eq!(fake.news(1).await, [DapNews::Launched]);
    // The fake logs the refusal once it reads it, before this request.
    fake.session.threads().unwrap();
    fake.news(1).await;
    let log = fake.log();
    let reply = log
        .iter()
        .find(|m| m["type"] == "response")
        .expect("a reply to runInTerminal");
    assert_eq!(reply["command"], "runInTerminal");
    assert_eq!(reply["success"], false);
    assert!(reply["request_seq"].is_i64());
}

#[test]
fn events_for_another_session_are_ignored() {
    let (tx, _rx) = unbounded_channel();
    let mut session = Session {
        id: 1,
        outgoing: Some(tx),
        tasks: Vec::new(),
        next_seq: 1,
        pending: HashMap::new(),
        disconnecting: false,
        over: false,
    };
    let event = DapEvent {
        session: 2,
        event: AdapterEvent::Exited(Some(1)),
    };
    assert_eq!(session.handle(event), []);
    assert!(!session.is_over());
}
