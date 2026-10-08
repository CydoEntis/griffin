//! A scripted debug adapter for Glyph's DAP tests. It speaks the Debug Adapter
//! Protocol over stdio with `Content-Length` framing, like a real adapter, and:
//!
//! - appends every message it receives to the file named by `FAKE_DAP_LOG`, one
//!   JSON object per line: `{"pid": <its process id>, "message": <the message>}`;
//! - answers requests from the JSON file named by `FAKE_DAP_SCRIPT`, shaped like
//!   `{"responses": {"<command>": <body>}, "events": {"<command>": [<message>]},
//!   "errors": {"<command>": "<text>"}, "exit_on": "<command>", "hold":
//!   ["<command>"], "scopes": {"<frameId>": [<scope>]}, "variables":
//!   {"<variablesReference>": [<variable>]}}`. Every key is optional:
//!   - `responses`: the body answering each command. Unscripted, `initialize`
//!     says it supports `configurationDone`, `setBreakpoints` verifies every line
//!     it was given, `threads` lists one thread `1` named `main`, `continue` says
//!     all threads continued, and anything else gets `{}`.
//!   - `events`: messages to send right after answering each command. `type`
//!     defaults to `event` and `seq` is filled in, so a script can also send a
//!     reverse request with `"type": "request"`. Unscripted, `initialize` is
//!     followed by the `initialized` event.
//!   - `errors`: commands answered with `success: false` and that message.
//!   - `exit_on`: a command that makes it exit with code 3 at once, as a crash.
//!   - `hold`: commands whose answers wait until the next request is answered,
//!     so responses arrive out of order.
//!   - `scopes` and `variables`: when `responses` doesn't script the command,
//!     the scopes of each frame and the variables of each reference, so a
//!     test can tell one frame or one expanded value from another. Unlisted
//!     ids get none.
//!
//! `--log <path>` and `--script <path>` arguments stand in for the two variables.
//!
//! It exits cleanly after answering `disconnect`, or when stdin closes.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::process::ExitCode;

use serde_json::{Value, json};

/// The exit code `exit_on` uses, so a test can tell the crash from a clean exit.
const CRASH_CODE: u8 = 3;

fn main() -> ExitCode {
    let script = arg("--script")
        .or_else(|| env::var_os("FAKE_DAP_SCRIPT"))
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or_else(|| json!({}));
    let mut log = arg("--log")
        .or_else(|| env::var_os("FAKE_DAP_LOG"))
        .and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());
    let mut input = BufReader::new(io::stdin().lock());
    let mut output = Output {
        out: io::stdout().lock(),
        seq: 0,
    };
    let mut held: Vec<Value> = Vec::new();

    while let Ok(Some(message)) = read_message(&mut input) {
        if let Some(log) = &mut log {
            let line = json!({"pid": std::process::id(), "message": message});
            // One write per line, so the log never holds half a message.
            let _ = log.write_all(format!("{line}\n").as_bytes());
            let _ = log.flush();
        }
        if message["type"].as_str() != Some("request") {
            // Answers to reverse requests need no reply.
            continue;
        }
        let command = message["command"].as_str().unwrap_or_default();
        if script["exit_on"].as_str() == Some(command) {
            return ExitCode::from(CRASH_CODE);
        }
        let mut response = json!({
            "type": "response",
            "request_seq": message["seq"],
            "command": command,
            "success": true,
        });
        if let Some(text) = script["errors"].get(command) {
            response["success"] = json!(false);
            response["message"] = text.clone();
        } else {
            response["body"] = match script["responses"].get(command) {
                Some(body) => body.clone(),
                None => default_body(command, &message["arguments"], &script),
            };
        }
        let holds = script["hold"]
            .as_array()
            .is_some_and(|list| list.iter().any(|c| c.as_str() == Some(command)));
        if holds {
            held.push(response);
        } else {
            output.send(response);
            for late in held.drain(..) {
                output.send(late);
            }
        }
        let events = match script["events"].get(command) {
            Some(list) => list.as_array().cloned().unwrap_or_default(),
            None if command == "initialize" => vec![json!({"event": "initialized"})],
            None => Vec::new(),
        };
        for mut event in events {
            if event.get("type").is_none() {
                event["type"] = json!("event");
            }
            output.send(event);
        }
        if command == "disconnect" {
            return ExitCode::SUCCESS;
        }
    }
    ExitCode::SUCCESS
}

/// The value following `flag` on the command line.
fn arg(flag: &str) -> Option<std::ffi::OsString> {
    let mut args = env::args_os();
    args.by_ref().find(|a| a == flag)?;
    args.next()
}

fn default_body(command: &str, arguments: &Value, script: &Value) -> Value {
    // Ids are JSON numbers, but object keys are strings.
    let by_id = |table: &str, id: &Value| -> Value {
        script[table]
            .get(id.to_string())
            .cloned()
            .unwrap_or_else(|| json!([]))
    };
    match command {
        "scopes" => json!({"scopes": by_id("scopes", &arguments["frameId"])}),
        "variables" => {
            json!({"variables": by_id("variables", &arguments["variablesReference"])})
        }
        "initialize" => json!({"supportsConfigurationDoneRequest": true}),
        "setBreakpoints" => {
            let breakpoints: Vec<Value> = arguments["breakpoints"]
                .as_array()
                .map(|list| {
                    list.iter()
                        .map(|b| json!({"verified": true, "line": b["line"]}))
                        .collect()
                })
                .unwrap_or_default();
            json!({"breakpoints": breakpoints})
        }
        "threads" => json!({"threads": [{"id": 1, "name": "main"}]}),
        "continue" => json!({"allThreadsContinued": true}),
        _ => json!({}),
    }
}

/// Stdout, numbering every message it sends as an adapter must.
struct Output<W: Write> {
    out: W,
    seq: i64,
}

impl<W: Write> Output<W> {
    fn send(&mut self, mut message: Value) {
        self.seq += 1;
        message["seq"] = json!(self.seq);
        let body = message.to_string();
        // A closed stdout means the client is gone; the next read ends the loop.
        let _ = write!(self.out, "Content-Length: {}\r\n\r\n{body}", body.len());
        let _ = self.out.flush();
    }
}

fn read_message(input: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let header = line.trim_end();
        if header.is_empty() {
            if length.is_some() {
                break;
            }
            continue;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length.unwrap_or(0)];
    input.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body).ok())
}
