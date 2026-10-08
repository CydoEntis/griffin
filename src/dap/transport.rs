//! A debug adapter's stdio: the tasks that pump framed DAP messages between the
//! child process and the app channel.

use std::io;
use std::path::Path;
use std::process::Stdio;

use serde_json::Value;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::task::JoinHandle;

use super::{AdapterEvent, DapEvent};
use crate::app::AppEvent;
use crate::framing::{encode, read_message};

/// A running adapter, seen from the session: where to send messages, and the
/// tasks that pump them. The reader ends once the process has exited.
#[derive(Debug)]
pub struct Connection {
    pub outgoing: UnboundedSender<Value>,
    pub tasks: Vec<JoinHandle<()>>,
}

/// Starts `program` in `cwd` and connects its stdio. Every message it sends comes
/// back as `AppEvent::Dap` for session `session`, and its exit as one last event
/// once its output closes. A spawn failure (command not found, say) is returned
/// as is.
pub fn spawn(
    session: u64,
    program: &Path,
    args: &[String],
    cwd: &Path,
    events: UnboundedSender<AppEvent>,
) -> io::Result<Connection> {
    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // Glyph quitting, or dropping a session, takes the adapter with it.
        .kill_on_drop(true)
        .spawn()?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(io::Error::other("adapter stdio was not piped"));
    };

    let (outgoing, mut queue) = mpsc::unbounded_channel::<Value>();
    let writer = tokio::spawn(async move {
        while let Some(message) = queue.recv().await {
            if stdin.write_all(&encode(&message)).await.is_err() || stdin.flush().await.is_err() {
                // The adapter is gone; its reader reports the exit.
                break;
            }
        }
    });

    let reader = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        // A garbled message ends the connection like a crash: there is no telling
        // where the next frame starts.
        while let Ok(Some(message)) = read_message(&mut reader).await {
            let event = AdapterEvent::Message(message);
            if events
                .send(AppEvent::Dap(DapEvent { session, event }))
                .is_err()
            {
                return;
            }
        }
        let code = child.wait().await.ok().and_then(|status| status.code());
        let event = AdapterEvent::Exited(code);
        let _ = events.send(AppEvent::Dap(DapEvent { session, event }));
    });

    Ok(Connection {
        outgoing,
        tasks: vec![writer, reader],
    })
}
