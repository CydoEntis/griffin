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
/// exited. A child the server started can hold the pipe open forever; this runs
/// on the reader task, so editing never waits on it either way.
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
    let stderr = child
        .stderr
        .take()
        .map(|stream| tokio::spawn(keep_tail(stream, Arc::clone(&tail))));

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
        if let Some(stderr) = stderr {
            let _ = tokio::time::timeout(STDERR_GRACE, stderr).await;
        }
        let stderr = tail
            .lock()
            .map(|lines| lines.iter().cloned().collect())
            .unwrap_or_default();
        let event = ServerEvent::Exited { code, stderr };
        let _ = events.send(AppEvent::Lsp(LspEvent { server, event }));
    });

    Ok(Connection {
        outgoing,
        tasks: vec![writer, reader],
    })
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
        let text = String::from_utf8_lossy(&bytes);
        let line: String = text
            .trim_end_matches(['\r', '\n'])
            .chars()
            .take(STDERR_LINE_CHARS)
            .collect();
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
