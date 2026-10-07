//! JSON-RPC over a language server's stdio: `Content-Length` framing, and the
//! tasks that pump messages between the child process and the app channel.

use std::io;
use std::path::Path;
use std::process::Stdio;

use serde_json::Value;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::task::JoinHandle;

use super::{LspEvent, ServerEvent};
use crate::app::AppEvent;

/// `message` with its `Content-Length` header, ready for the wire.
pub fn encode(message: &Value) -> Vec<u8> {
    let body = message.to_string();
    let mut out = format!("Content-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}

/// Reads one framed message. `Ok(None)` at a clean end of stream; a header without
/// a length, or a body that isn't JSON, is an error.
pub async fn read_message<R: AsyncBufRead + Unpin>(reader: &mut R) -> io::Result<Option<Value>> {
    let mut length = None;
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line).await? == 0 {
            return if length.is_none() {
                Ok(None)
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            };
        }
        let header = line.trim_end();
        if header.is_empty() {
            if length.is_some() {
                break;
            }
            // Stray blank lines between messages carry nothing.
            continue;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = Some(value.trim().parse::<usize>().map_err(io::Error::other)?);
        }
    }
    let Some(length) = length else {
        return Err(io::Error::other("message without Content-Length"));
    };
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(io::Error::other)
}

/// A running server process, seen from the app: where to send messages, and the
/// tasks to wait on while shutting down. The reader ends once the process has
/// exited.
#[derive(Debug)]
pub struct Connection {
    pub outgoing: UnboundedSender<Value>,
    pub tasks: Vec<JoinHandle<()>>,
}

/// Starts `program` in `root` and connects its stdio. Every message it sends comes
/// back as `AppEvent::Lsp` for server `server`, and its exit as one last event once
/// its output closes. A spawn failure (command not found, say) is returned as is.
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
        .stderr(Stdio::null())
        // Glyph quitting, or dropping a server, takes the process with it.
        .kill_on_drop(true)
        .spawn()?;
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return Err(io::Error::other("server stdio was not piped"));
    };

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
        let event = ServerEvent::Exited(code);
        let _ = events.send(AppEvent::Lsp(LspEvent { server, event }));
    });

    Ok(Connection {
        outgoing,
        tasks: vec![writer, reader],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn read_all(bytes: &[u8]) -> Vec<io::Result<Option<Value>>> {
        let mut reader = BufReader::new(bytes);
        let mut out = Vec::new();
        loop {
            let message = read_message(&mut reader).await;
            let done = !matches!(message, Ok(Some(_)));
            out.push(message);
            if done {
                return out;
            }
        }
    }

    #[tokio::test]
    async fn encoded_messages_read_back_in_order() {
        let first = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"});
        let second = json!({"jsonrpc": "2.0", "method": "note", "params": {"text": "héllo"}});
        let mut bytes = encode(&first);
        bytes.extend(encode(&second));
        let read = read_all(&bytes).await;
        assert_eq!(read.len(), 3);
        assert_eq!(read[0].as_ref().unwrap().as_ref(), Some(&first));
        assert_eq!(read[1].as_ref().unwrap().as_ref(), Some(&second));
        assert!(matches!(read[2], Ok(None)));
    }

    #[test]
    fn the_length_counts_bytes_not_chars() {
        let bytes = encode(&json!("é"));
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.starts_with("Content-Length: 4\r\n\r\n"), "{text:?}");
    }

    #[tokio::test]
    async fn other_headers_are_skipped() {
        let body = r#"{"id":2}"#;
        let bytes = format!(
            "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        let read = read_all(bytes.as_bytes()).await;
        assert_eq!(read[0].as_ref().unwrap().as_ref(), Some(&json!({"id": 2})));
    }

    #[tokio::test]
    async fn a_truncated_body_is_an_error() {
        let read = read_all(b"Content-Length: 10\r\n\r\n{}").await;
        assert!(read[0].is_err());
    }
}
