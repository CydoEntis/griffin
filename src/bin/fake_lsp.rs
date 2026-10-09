//! A scripted language server for Tome's LSP tests. It speaks JSON-RPC over
//! stdio with `Content-Length` framing, like a real server, and:
//!
//! - appends every message it receives to the file named by `FAKE_LSP_LOG`, one
//!   JSON object per line: `{"pid": <its process id>, "message": <the message>}`;
//! - answers requests from the JSON file named by `FAKE_LSP_SCRIPT`, shaped like
//!   `{"responses": {"<method>": <result>}, "notify": {"<method>": [<message>]},
//!   "exit_on": "<method>"}`. Every key is optional:
//!   - `responses`: the result for each request method. `initialize` defaults to
//!     incremental sync with saves and formatting; `shutdown` and anything
//!     unscripted get `null`.
//!     `"$uri"` and `"$dir/<name>"` are filled in as for `notify`.
//!   - `notify`: messages (e.g. `textDocument/publishDiagnostics` notifications)
//!     to send right after receiving each method; `jsonrpc` is filled in, and any
//!     string `"$uri"` in them becomes the received message's document URI, and
//!     `"$dir/<name>"` the URI of file `<name>` in the same folder, so a script
//!     needn't know where the test put its files.
//!   - `exit_on`: a method that makes it exit with code 3 at once, as a crash.
//!   - `delay`: milliseconds to wait before answering each method, as a slow
//!     server would; its `notify` messages follow the late answer.
//!   - `silent`: methods whose requests are never answered, as a hung server's.
//!   - `errors`: the JSON-RPC error object to answer each method with instead
//!     of a result.
//!   - `stderr`: lines to write to stderr on receiving each method, before
//!     anything else it does (an `exit_on` crash included), as a failing server
//!     says why.
//!   - `hold_stderr`: milliseconds a child it starts at once keeps its stderr
//!     open after it, as a wrapper script's or a forking server's child would.
//!     The child is this program run with `--hold <ms>`; it touches nothing else.
//!     Unix only in effect: on Windows the child inherits the stdout pipe too.
//!
//! `--log <path>` and `--script <path>` arguments stand in for the two variables,
//! for tests that start it through a server config rather than a child of their
//! own, where they can't set its environment.
//!
//! It exits cleanly on `exit` or when stdin closes.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::process::ExitCode;

use serde_json::{Value, json};

/// The exit code `exit_on` uses, so a test can tell the crash from a clean exit.
const CRASH_CODE: u8 = 3;

fn main() -> ExitCode {
    if let Some(ms) = arg("--hold").and_then(|ms| ms.to_str()?.parse::<u64>().ok()) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        return ExitCode::SUCCESS;
    }
    let script = arg("--script")
        .or_else(|| env::var_os("FAKE_LSP_SCRIPT"))
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(ms) = script["hold_stderr"].as_u64()
        && let Ok(me) = env::current_exe()
    {
        // Only stderr is shared: a held stdout would keep the client from ever
        // seeing this server exit.
        let _ = std::process::Command::new(me)
            .args(["--hold", &ms.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .spawn();
    }
    let mut log = arg("--log")
        .or_else(|| env::var_os("FAKE_LSP_LOG"))
        .and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());
    let mut input = BufReader::new(io::stdin().lock());
    let mut output = io::stdout().lock();

    while let Ok(Some(message)) = read_message(&mut input) {
        if let Some(log) = &mut log {
            let line = json!({"pid": std::process::id(), "message": message});
            // One write per line, so two servers sharing a log never interleave.
            let _ = log.write_all(format!("{line}\n").as_bytes());
            let _ = log.flush();
        }
        let method = message["method"].as_str().unwrap_or_default();
        if let Some(lines) = script["stderr"][method].as_array() {
            let mut stderr = io::stderr().lock();
            for line in lines {
                let _ = writeln!(stderr, "{}", line.as_str().unwrap_or_default());
            }
            let _ = stderr.flush();
        }
        if !method.is_empty() && script["exit_on"].as_str() == Some(method) {
            return ExitCode::from(CRASH_CODE);
        }
        if method == "exit" {
            return ExitCode::SUCCESS;
        }
        if let Some(ms) = script["delay"][method].as_u64() {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
        let silent = script["silent"]
            .as_array()
            .is_some_and(|list| list.iter().any(|m| m.as_str() == Some(method)));
        if let Some(error) = script["errors"].get(method)
            && let Some(id) = message.get("id")
        {
            write_message(
                &mut output,
                &json!({"jsonrpc": "2.0", "id": id, "error": error}),
            );
        } else if let Some(id) = message.get("id")
            && !method.is_empty()
            && !silent
        {
            let result = match script["responses"].get(method) {
                Some(result) => {
                    let mut result = result.clone();
                    fill_uri(&mut result, &message["params"]["textDocument"]["uri"]);
                    result
                }
                None if method == "initialize" => default_initialize(),
                None => Value::Null,
            };
            write_message(
                &mut output,
                &json!({"jsonrpc": "2.0", "id": id, "result": result}),
            );
        }
        if let Some(list) = script["notify"][method].as_array() {
            let uri = message["params"]["textDocument"]["uri"].clone();
            for note in list {
                let mut note = note.clone();
                fill_uri(&mut note, &uri);
                note["jsonrpc"] = json!("2.0");
                write_message(&mut output, &note);
            }
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

/// Replaces every `"$uri"` string in `value` with `uri`, and every
/// `"$dir/<name>"` with the URI of `<name>` beside it.
fn fill_uri(value: &mut Value, uri: &Value) {
    match value {
        Value::String(text) if text == "$uri" => *value = uri.clone(),
        Value::String(text) if text.starts_with("$dir/") => {
            if let Some((dir, _)) = uri.as_str().and_then(|uri| uri.rsplit_once('/')) {
                *value = Value::String(format!("{dir}/{}", &text["$dir/".len()..]));
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| fill_uri(item, uri)),
        Value::Object(fields) => fields.values_mut().for_each(|field| fill_uri(field, uri)),
        _ => {}
    }
}

fn default_initialize() -> Value {
    json!({
        "capabilities": {
            "textDocumentSync": {
                "openClose": true,
                "change": 2,
                "save": {"includeText": false}
            },
            "documentFormattingProvider": true
        },
        "serverInfo": {"name": "fake_lsp"}
    })
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

fn write_message(output: &mut impl Write, message: &Value) {
    let body = message.to_string();
    // A closed stdout means the client is gone; the next read ends the loop.
    let _ = write!(output, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = output.flush();
}
