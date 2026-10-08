//! The Debug Adapter Protocol: one session with one adapter child process.
//! Requests go out numbered by `seq`; replies and events reach the app only as
//! `AppEvent::Dap` (ADR-0001) and are turned into `DapNews` by `Session::handle`,
//! which matches each response to its request by `request_seq`. A missing or
//! crashed adapter is one `DapNews::Failed`, and nothing here ever waits on it.
//! `adapters` says which adapter debugs each language and how the program is
//! launched.

// Nothing starts a session yet: the keys and screens that do come with the
// debugger tickets that follow this one.
#![allow(dead_code)]

pub mod adapters;
mod transport;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

use crate::app::AppEvent;

/// Something one adapter did, tagged with the session it belongs to.
#[derive(Debug)]
pub struct DapEvent {
    pub session: u64,
    pub event: AdapterEvent,
}

#[derive(Debug)]
pub enum AdapterEvent {
    /// A DAP message from the adapter.
    Message(Value),
    /// The adapter's output closed and the process exited, with its code if any.
    Exited(Option<i32>),
    /// The adapter couldn't be started; the text says why.
    NotStarted(String),
}

/// A thread of the program being debugged.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Thread {
    pub id: i64,
    pub name: String,
}

/// The file a stack frame is in, when the adapter knows it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
pub struct Source {
    pub name: Option<String>,
    pub path: Option<PathBuf>,
}

/// One frame of a call stack. `line` and `column` count from 1, as Glyph asks
/// adapters to in `initialize`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StackFrame {
    pub id: i64,
    pub name: String,
    pub source: Option<Source>,
    #[serde(default)]
    pub line: usize,
    #[serde(default)]
    pub column: usize,
}

/// A group of variables in a frame, such as locals or globals.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    pub name: String,
    pub variables_reference: i64,
    #[serde(default)]
    pub expensive: bool,
}

/// One variable. A non-zero `variables_reference` means it has children, fetched
/// with `Session::variables`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    pub name: String,
    pub value: String,
    #[serde(rename = "type")]
    pub kind: Option<String>,
    #[serde(default)]
    pub variables_reference: i64,
}

/// Where the adapter put a requested breakpoint, and whether it will hit.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Breakpoint {
    #[serde(default)]
    pub verified: bool,
    pub line: Option<usize>,
    pub message: Option<String>,
}

/// What `handle` makes of the adapter's messages, for the app to act on.
#[derive(Debug, Clone, PartialEq)]
pub enum DapNews {
    /// `initialize` was answered, with the adapter's capabilities.
    Capabilities(Value),
    /// The `initialized` event: breakpoints can be sent now.
    Initialized,
    /// `launch` was answered: the program is starting.
    Launched,
    /// `setBreakpoints` for `path` was answered, one entry per line sent.
    Breakpoints {
        path: PathBuf,
        breakpoints: Vec<Breakpoint>,
    },
    /// `configurationDone` was answered.
    ConfigurationDone,
    Threads(Vec<Thread>),
    StackTrace {
        thread: i64,
        frames: Vec<StackFrame>,
    },
    Scopes {
        frame: i64,
        scopes: Vec<Scope>,
    },
    Variables {
        reference: i64,
        variables: Vec<Variable>,
    },
    /// `continue`, `next`, `stepIn` or `stepOut` (the command) was answered.
    Resumed(String),
    /// The program stopped: breakpoint, step, exception, pause.
    Stopped {
        reason: String,
        thread: Option<i64>,
        text: Option<String>,
    },
    /// The program, or the adapter about it, printed `text`.
    Output {
        category: String,
        text: String,
    },
    /// The debugging session is over.
    Terminated,
    /// The program exited with this code.
    Exited(i64),
    /// The adapter refused `command`, saying `message`.
    Refused {
        command: String,
        message: String,
    },
    /// The adapter is missing or crashed. Said once; the session is over.
    Failed(String),
    /// The adapter exited after `disconnect`, as asked.
    Ended,
}

/// A request waiting for its response, with what's needed to make sense of it.
#[derive(Debug, Clone)]
enum Pending {
    Initialize,
    Launch,
    SetBreakpoints(PathBuf),
    ConfigurationDone,
    Threads,
    StackTrace(i64),
    Scopes(i64),
    Variables(i64),
    Resume(&'static str),
    Disconnect,
}

#[derive(Debug)]
pub struct Session {
    pub id: u64,
    /// `None` once the adapter is gone, or when it never started.
    outgoing: Option<UnboundedSender<Value>>,
    /// The connection's writer and reader.
    tasks: Vec<JoinHandle<()>>,
    next_seq: i64,
    /// Requests sent and not yet answered, by `seq`.
    pending: HashMap<i64, Pending>,
    /// Glyph sent `disconnect`, so the adapter exiting is no crash.
    disconnecting: bool,
    /// `Failed` or `Ended` was reported; nothing more is.
    over: bool,
}

impl Session {
    /// Starts `program` with `args` in `cwd` as session `id`. It never fails: an
    /// adapter that can't be started comes back through `events` as one
    /// `AdapterEvent::NotStarted`, which `handle` turns into `DapNews::Failed`, so
    /// the app reports it like a crash.
    pub fn start(
        id: u64,
        program: &Path,
        args: &[String],
        cwd: &Path,
        events: UnboundedSender<AppEvent>,
    ) -> Self {
        let (outgoing, tasks) = match transport::spawn(id, program, args, cwd, events.clone()) {
            Ok(connection) => (Some(connection.outgoing), connection.tasks),
            Err(err) => {
                let why = format!("can't start {}: {err}", program.display());
                let event = AdapterEvent::NotStarted(why);
                let _ = events.send(AppEvent::Dap(DapEvent { session: id, event }));
                (None, Vec::new())
            }
        };
        Self {
            id,
            outgoing,
            tasks,
            next_seq: 1,
            pending: HashMap::new(),
            disconnecting: false,
            over: false,
        }
    }

    /// The adapter is gone, or never came.
    pub fn is_over(&self) -> bool {
        self.over
    }

    /// Sends `command` and remembers it as `pending`. Returns its `seq`, or `None`
    /// when there is no adapter to send to.
    fn request(&mut self, command: &str, arguments: Value, pending: Pending) -> Option<i64> {
        let outgoing = self.outgoing.as_ref()?;
        let seq = self.next_seq;
        let message = json!({
            "seq": seq,
            "type": "request",
            "command": command,
            "arguments": arguments,
        });
        outgoing.send(message).ok()?;
        self.next_seq += 1;
        self.pending.insert(seq, pending);
        Some(seq)
    }

    /// The first request of a session; `adapter` is the adapter's id, such as
    /// `lldb-dap`.
    pub fn initialize(&mut self, adapter: &str) -> Option<i64> {
        let arguments = json!({
            "clientID": "glyph",
            "clientName": "Glyph",
            "adapterID": adapter,
            "pathFormat": "path",
            // Counting from 1 matches what people read; callers convert from
            // the buffer's 0-based lines.
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "supportsVariableType": true,
            "supportsRunInTerminalRequest": false,
        });
        self.request("initialize", arguments, Pending::Initialize)
    }

    /// Starts the program. The arguments are the adapter's own, so they are
    /// passed through as given.
    pub fn launch(&mut self, arguments: Value) -> Option<i64> {
        self.request("launch", arguments, Pending::Launch)
    }

    /// Replaces every breakpoint in `path` with ones on `lines`, counted from 1.
    pub fn set_breakpoints(&mut self, path: &Path, lines: &[usize]) -> Option<i64> {
        let breakpoints: Vec<Value> = lines.iter().map(|line| json!({"line": line})).collect();
        let arguments = json!({
            "source": {"path": path},
            "breakpoints": breakpoints,
        });
        let pending = Pending::SetBreakpoints(path.to_path_buf());
        self.request("setBreakpoints", arguments, pending)
    }

    pub fn configuration_done(&mut self) -> Option<i64> {
        self.request("configurationDone", json!({}), Pending::ConfigurationDone)
    }

    pub fn threads(&mut self) -> Option<i64> {
        self.request("threads", json!({}), Pending::Threads)
    }

    pub fn stack_trace(&mut self, thread: i64) -> Option<i64> {
        let arguments = json!({"threadId": thread});
        self.request("stackTrace", arguments, Pending::StackTrace(thread))
    }

    pub fn scopes(&mut self, frame: i64) -> Option<i64> {
        let arguments = json!({"frameId": frame});
        self.request("scopes", arguments, Pending::Scopes(frame))
    }

    pub fn variables(&mut self, reference: i64) -> Option<i64> {
        let arguments = json!({"variablesReference": reference});
        self.request("variables", arguments, Pending::Variables(reference))
    }

    /// Runs the program on from where `thread` stopped.
    pub fn resume(&mut self, thread: i64) -> Option<i64> {
        self.step("continue", thread)
    }

    pub fn next(&mut self, thread: i64) -> Option<i64> {
        self.step("next", thread)
    }

    pub fn step_in(&mut self, thread: i64) -> Option<i64> {
        self.step("stepIn", thread)
    }

    pub fn step_out(&mut self, thread: i64) -> Option<i64> {
        self.step("stepOut", thread)
    }

    fn step(&mut self, command: &'static str, thread: i64) -> Option<i64> {
        let arguments = json!({"threadId": thread});
        self.request(command, arguments, Pending::Resume(command))
    }

    /// Ends the session and the program with it. The adapter exits on its own
    /// after answering, which `handle` reports as `DapNews::Ended`.
    pub fn disconnect(&mut self) -> Option<i64> {
        let arguments = json!({"restart": false, "terminateDebuggee": true});
        let seq = self.request("disconnect", arguments, Pending::Disconnect)?;
        self.disconnecting = true;
        Some(seq)
    }

    /// What `event` means for this session. Events for another session, or after
    /// this one is over, mean nothing.
    pub fn handle(&mut self, event: DapEvent) -> Vec<DapNews> {
        if event.session != self.id || self.over {
            return Vec::new();
        }
        match event.event {
            AdapterEvent::Message(message) => self.message(message).into_iter().collect(),
            AdapterEvent::NotStarted(why) => vec![self.end(DapNews::Failed(why))],
            AdapterEvent::Exited(code) => {
                let news = if self.disconnecting {
                    DapNews::Ended
                } else {
                    let code = code.map_or_else(|| "no code".to_string(), |c| c.to_string());
                    DapNews::Failed(format!("debug adapter exited ({code})"))
                };
                vec![self.end(news)]
            }
        }
    }

    /// Marks the session over, dropping whatever was still unanswered: no
    /// answer is coming, and reporting each would repeat the one failure.
    fn end(&mut self, news: DapNews) -> DapNews {
        self.over = true;
        self.outgoing = None;
        self.pending.clear();
        news
    }

    fn message(&mut self, message: Value) -> Option<DapNews> {
        match message["type"].as_str()? {
            "response" => self.response(&message),
            "event" => event(&message),
            "request" => {
                // Reverse requests (`runInTerminal`, say) aren't supported; a
                // refusal lets the adapter fall back instead of waiting.
                let reply = json!({
                    "seq": self.next_seq,
                    "type": "response",
                    "request_seq": message["seq"],
                    "command": message["command"],
                    "success": false,
                    "message": "not supported by Glyph",
                });
                if let Some(outgoing) = &self.outgoing
                    && outgoing.send(reply).is_ok()
                {
                    self.next_seq += 1;
                }
                None
            }
            _ => None,
        }
    }

    fn response(&mut self, message: &Value) -> Option<DapNews> {
        let pending = self.pending.remove(&message["request_seq"].as_i64()?)?;
        if message["success"].as_bool() != Some(true) {
            if matches!(pending, Pending::Disconnect) {
                // Ending anyway: the adapter goes when its stdin closes.
                return None;
            }
            let command = message["command"].as_str().unwrap_or_default().to_string();
            let message = message["message"]
                .as_str()
                .or_else(|| message["body"]["error"]["format"].as_str())
                .unwrap_or("failed")
                .to_string();
            return Some(DapNews::Refused { command, message });
        }
        let body = &message["body"];
        Some(match pending {
            Pending::Initialize => DapNews::Capabilities(body.clone()),
            Pending::Launch => DapNews::Launched,
            Pending::SetBreakpoints(path) => DapNews::Breakpoints {
                path,
                breakpoints: list(&body["breakpoints"]),
            },
            Pending::ConfigurationDone => DapNews::ConfigurationDone,
            Pending::Threads => DapNews::Threads(list(&body["threads"])),
            Pending::StackTrace(thread) => DapNews::StackTrace {
                thread,
                frames: list(&body["stackFrames"]),
            },
            Pending::Scopes(frame) => DapNews::Scopes {
                frame,
                scopes: list(&body["scopes"]),
            },
            Pending::Variables(reference) => DapNews::Variables {
                reference,
                variables: list(&body["variables"]),
            },
            Pending::Resume(command) => DapNews::Resumed(command.to_string()),
            Pending::Disconnect => return None,
        })
    }
}

/// The news in an adapter's event, for the events Glyph follows.
fn event(message: &Value) -> Option<DapNews> {
    let body = &message["body"];
    let text = |key: &str| body[key].as_str().map(str::to_string);
    Some(match message["event"].as_str()? {
        "initialized" => DapNews::Initialized,
        "stopped" => DapNews::Stopped {
            reason: text("reason").unwrap_or_default(),
            thread: body["threadId"].as_i64(),
            text: text("text").or_else(|| text("description")),
        },
        "output" => DapNews::Output {
            category: text("category").unwrap_or_else(|| "console".to_string()),
            text: text("output").unwrap_or_default(),
        },
        "terminated" => DapNews::Terminated,
        "exited" => DapNews::Exited(body["exitCode"].as_i64().unwrap_or_default()),
        _ => return None,
    })
}

/// The items of a JSON array that read as `T`; an adapter's odd entry is skipped
/// rather than losing the rest.
fn list<T: for<'de> Deserialize<'de>>(value: &Value) -> Vec<T> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| T::deserialize(item).ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
