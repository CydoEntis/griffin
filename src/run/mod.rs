//! Runs `[[run]]` commands as child processes. Output comes back to the event loop
//! only as `AppEvent::RunOutput` and `AppEvent::RunExited` (ADR-0001).

use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;

use crate::app::AppEvent;
use crate::config::RunEntry;

mod tree;

pub use tree::ProcessTree;

/// Where `entry` runs: its `cwd` under the project root, or the root itself.
pub fn resolve_cwd(root: &Path, cwd: Option<&str>) -> PathBuf {
    match cwd {
        Some(cwd) if !cwd.trim().is_empty() => root.join(cwd),
        _ => root.to_path_buf(),
    }
}

/// `command` handed to the platform shell, so pipes, `&&` and PATH lookup work as
/// they do when typed.
pub fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let mut cmd = Command::new("cmd");
        // Passed untouched: Rust's argument quoting would escape the quotes inside
        // `command`, and cmd reads its command line raw rather than as argv.
        cmd.arg("/C").raw_arg(command);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

/// Starts `entry` from `root` and streams its stdout and stderr, line by line, as
/// `RunOutput { run, .. }`, then sends `RunExited { run, .. }` once both streams
/// have closed and the process has exited. The returned tree stops the command
/// and everything it started.
pub fn spawn(
    run: u64,
    entry: &RunEntry,
    root: &Path,
    events: UnboundedSender<AppEvent>,
) -> io::Result<ProcessTree> {
    let mut cmd = shell_command(&entry.command);
    cmd.current_dir(resolve_cwd(root, entry.cwd.as_deref()))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    tree::prepare(&mut cmd);
    let mut child = cmd.spawn()?;
    let tree = ProcessTree::adopt(&mut child)?;
    let readers = [
        child
            .stdout
            .take()
            .map(|out| tokio::spawn(forward_lines(run, out, events.clone()))),
        child
            .stderr
            .take()
            .map(|err| tokio::spawn(forward_lines(run, err, events.clone()))),
    ];
    tokio::spawn(async move {
        for reader in readers.into_iter().flatten() {
            let _ = reader.await;
        }
        let code = match child.wait().await {
            Ok(status) => status.code(),
            Err(_) => None,
        };
        let _ = events.send(AppEvent::RunExited { run, code });
    });
    Ok(tree)
}

/// Sends each line of `stream` as it arrives. Bytes go through lossily, since a
/// tool's output isn't promised to be UTF-8, and colour codes are left in for the
/// panel to draw.
async fn forward_lines(
    run: u64,
    stream: impl AsyncRead + Unpin,
    events: UnboundedSender<AppEvent>,
) {
    let mut reader = BufReader::new(stream);
    let mut bytes = Vec::new();
    loop {
        bytes.clear();
        match reader.read_until(b'\n', &mut bytes).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                while bytes.last().is_some_and(|&b| b == b'\n' || b == b'\r') {
                    bytes.pop();
                }
                let line = String::from_utf8_lossy(&bytes).into_owned();
                if events.send(AppEvent::RunOutput { run, line }).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[test]
    fn cwd_is_relative_to_the_root_and_defaults_to_it() {
        let root = Path::new("project");
        assert_eq!(resolve_cwd(root, None), root);
        assert_eq!(resolve_cwd(root, Some("")), root);
        assert_eq!(resolve_cwd(root, Some(".")), root.join("."));
        assert_eq!(resolve_cwd(root, Some("web")), root.join("web"));
    }

    #[tokio::test]
    async fn output_lines_then_the_exit_code_come_back_on_the_channel() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let entry = RunEntry {
            name: "t".into(),
            command: "echo one && echo two 1>&2 && exit 3".into(),
            cwd: Some("sub".into()),
        };
        let (tx, mut rx) = mpsc::unbounded_channel();
        let _tree = spawn(7, &entry, dir.path(), tx).unwrap();
        let mut lines = Vec::new();
        loop {
            match rx.recv().await.expect("the run reports its exit") {
                AppEvent::RunOutput { run, line } => {
                    assert_eq!(run, 7);
                    lines.push(line.trim_end().to_string());
                }
                AppEvent::RunExited { run, code } => {
                    assert_eq!(run, 7);
                    assert_eq!(code, Some(3));
                    break;
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        lines.sort();
        assert_eq!(lines, ["one", "two"]);
    }
}
