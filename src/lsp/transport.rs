//! JSON-RPC over a language server's stdio: the tasks that pump framed messages
//! between the child process and the app channel.

use std::collections::VecDeque;
use std::io;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::{LspEvent, ServerEvent};
use crate::app::AppEvent;
use crate::framing::{encode, read_message};

/// A running server process, seen from the app: where to send messages, and the
/// tasks to wait on while shutting down. The reader ends once the process has
/// exited.
#[derive(Debug)]
pub struct Connection {
    pub outgoing: UnboundedSender<Value>,
    pub tasks: Vec<JoinHandle<()>>,
}

/// How many of the last lines a server wrote to stderr are kept, to say why it
/// failed.
pub const STDERR_LINES: usize = 20;

/// A kept stderr line longer than this is cut: one is a reason, not a log.
const STDERR_LINE_CHARS: usize = 500;

/// How long the exit report waits for stderr to close after the process has
/// exited. A child the server started can hold the pipe open forever, so the
/// stderr task is aborted after this. It runs on the reader task, so editing
/// never waits on it either way.
const STDERR_GRACE: Duration = Duration::from_millis(250);

/// The last lines written to a stderr, shared between the task reading it and
/// the exit report.
type Tail = Arc<Mutex<VecDeque<String>>>;

/// Starts `program` in `root` and connects its stdio. Every message it sends comes
/// back as `AppEvent::Lsp` for server `server`, and its exit as one last event once
/// its output closes, carrying the last `STDERR_LINES` lines it wrote to stderr. A
/// spawn failure (command not found, say) is returned as is.
pub fn spawn(
    server: u64,
    program: &Path,
    args: &[String],
    root: &Path,
    events: UnboundedSender<AppEvent>,
) -> io::Result<Connection> {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Read all along, never waited on: a server blocks once a full stderr
        // pipe goes unread, and its last words say why it failed.
        .stderr(Stdio::piped())
        // Glyph quitting, or dropping a server, takes the process with it.
        .kill_on_drop(true)
        .spawn()?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(io::Error::other("server stdio was not piped"));
    };
    let tail: Tail = Arc::default();
    // Dropped when the stderr task ends, so the reader can wait for that without
    // owning the task: the task's handle goes in `tasks`, where stopping or
    // forgetting the server aborts it.
    let (closed, stderr_closed) = oneshot::channel::<()>();
    let stderr = child.stderr.take().map(|stream| {
        let tail = Arc::clone(&tail);
        tokio::spawn(async move {
            keep_tail(stream, tail).await;
            drop(closed);
        })
    });
    let stderr_abort = stderr.as_ref().map(JoinHandle::abort_handle);

    let (outgoing, mut queue) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        while let Some(message) = queue.recv().await {
            let last = message.get("method").and_then(Value::as_str) == Some("exit");
            if stdin.write_all(&encode(&message)).await.is_err() || stdin.flush().await.is_err() {
                // The server is gone; its reader reports the exit.
                break;
            }
            if last {
                break;
            }
        }
    });

    let reader = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        // A garbled message ends the connection like a crash: there is no telling
        // where the next frame starts.
        while let Ok(Some(message)) = read_message(&mut reader).await {
            let event = ServerEvent::Message(message);
            if events
                .send(AppEvent::Lsp(LspEvent { server, event }))
                .is_err()
            {
                return;
            }
        }
        let code = child.wait().await.ok().and_then(|status| status.code());
        let _ = tokio::time::timeout(STDERR_GRACE, stderr_closed).await;
        // A child the server started may hold the pipe open for good; its
        // output says nothing about this server's exit, and the task would
        // outlive every retry.
        if let Some(stderr) = stderr_abort {
            stderr.abort();
        }
        let stderr = tail
            .lock()
            .map(|lines| lines.iter().cloned().collect())
            .unwrap_or_default();
        let event = ServerEvent::Exited { code, stderr };
        let _ = events.send(AppEvent::Lsp(LspEvent { server, event }));
    });

    let mut tasks = vec![writer, reader];
    tasks.extend(stderr);
    Ok(Connection { outgoing, tasks })
}

/// Reads `stream` to its end, keeping its last `STDERR_LINES` lines in `tail`.
/// Bytes go through lossily, since a server's stderr isn't promised to be UTF-8.
async fn keep_tail(stream: impl AsyncRead + Unpin, tail: Tail) {
    let mut reader = BufReader::new(stream);
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        match reader.read_until(b'\n', &mut bytes).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let line = plain_line(&String::from_utf8_lossy(&bytes));
        // A poisoned lock only loses the tail; reading goes on regardless, since
        // a server whose stderr fills up stops.
        if let Ok(mut lines) = tail.lock() {
            if lines.len() == STDERR_LINES {
                lines.pop_front();
            }
            lines.push_back(line);
        }
    }
}

/// One stderr line as a person would have seen it in a terminal: what the last
/// carriage return left (a progress line redraws itself that way), without
/// colour or cursor codes (CSI sequences), cut to `STDERR_LINE_CHARS`.
fn plain_line(raw: &str) -> String {
    let line = raw.trim_end_matches(['\r', '\n']);
    let line = line.rsplit('\r').next().unwrap_or(line);
    let mut out = String::new();
    let mut kept = 0;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            kept += 1;
            if kept == STDERR_LINE_CHARS {
                break;
            }
            continue;
        }
        if chars.next_if_eq(&'[').is_some() {
            // Parameter and intermediate bytes, up to the final letter.
            while chars
                .next_if(|c| ('\u{20}'..='\u{3f}').contains(c))
                .is_some()
            {}
            chars.next_if(|c| ('\u{40}'..='\u{7e}').contains(c));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_line_drops_colour_codes_and_what_a_carriage_return_overwrote() {
        assert_eq!(
            plain_line("\u{1b}[1;31merror\u{1b}[0m: no tsserver\r\n"),
            "error: no tsserver"
        );
        assert_eq!(
            plain_line("loading 10%\rloading 50%\r\u{1b}[2Kfailed to load\n"),
            "failed to load"
        );
        assert_eq!(plain_line("plain\n"), "plain");
        // A sequence cut off at the end of the line goes with it.
        assert_eq!(plain_line("broken \u{1b}[31"), "broken ");
        let long = "é".repeat(STDERR_LINE_CHARS + 10);
        assert_eq!(plain_line(&long).chars().count(), STDERR_LINE_CHARS);
    }
}
