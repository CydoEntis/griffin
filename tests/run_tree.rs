mod harness;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use harness::Glyph;

const START: Duration = Duration::from_secs(10);
const WAIT: Duration = Duration::from_secs(15);
/// R27: the grandchild has to be gone this soon after a stop or a quit.
const GONE_WITHIN: Duration = Duration::from_secs(2);
/// `.glyph.toml` has one entry, `dev`, which runs the `spawn-tree` script: it
/// starts a long-lived grandchild, prints `grandchild <pid>`, then waits on it.
const PROJECT: &str = "tests/fixtures/run-tree";

/// `PATH` with the fixture folder first, so `spawn-tree` resolves to the fixture's
/// script under both `sh` and `cmd`.
fn fixture_path() -> Vec<(&'static str, OsString)> {
    let fixture = std::path::absolute(PROJECT).expect("fixture path");
    let mut paths: Vec<PathBuf> = vec![fixture];
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    let joined = std::env::join_paths(paths).expect("PATH entries join");
    vec![("PATH", joined)]
}

/// Kills the grandchild if a test fails before Glyph does, so a failure doesn't
/// leave it running for five minutes.
struct Leftover(u32);

impl Drop for Leftover {
    fn drop(&mut self) {
        if alive(self.0) {
            let pid = self.0.to_string();
            #[cfg(windows)]
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/PID", &pid])
                .output();
            #[cfg(unix)]
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid])
                .output();
        }
    }
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .expect("tasklist runs");
    String::from_utf8_lossy(&out.stdout).contains(&format!("\"{pid}\""))
}

/// A zombie counts as gone: it's dead, only waiting for its new parent to reap it.
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // The state follows the command name, which is in parentheses.
        Ok(stat) => stat
            .rsplit_once(')')
            .is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z')),
        Err(_) => false,
    }
}

/// Polls, since a process ending has no output to wait for.
fn assert_gone(pid: u32, glyph: &Glyph) {
    let deadline = Instant::now() + GONE_WITHIN;
    while alive(pid) {
        assert!(
            Instant::now() < deadline,
            "grandchild {pid} still running {GONE_WITHIN:?} later\n{}",
            glyph.screen().join("\n")
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Starts `dev` and returns the grandchild's pid once the script has printed it.
fn run_dev(glyph: &mut Glyph) -> u32 {
    glyph.send_keys("f5");
    glyph.wait_for_text("dev · running", WAIT);
    glyph.wait_for_text("grandchild ", WAIT);
    let screen = glyph.screen();
    let pid = screen
        .iter()
        .find_map(|line| {
            let (_, rest) = line.split_once("grandchild ")?;
            rest.split_whitespace().next()?.parse().ok()
        })
        .unwrap_or_else(|| panic!("no pid on screen\n{}", screen.join("\n")));
    assert!(alive(pid), "grandchild {pid} isn't running yet");
    pid
}

fn open_project() -> Glyph {
    let glyph = Glyph::spawn_in_with_env(Path::new(PROJECT), &fixture_path(), &[]);
    glyph.wait_for_text("Ln 1, Col 1", START);
    glyph
}

#[test]
fn stop_kills_grandchild() {
    let mut glyph = open_project();
    let pid = run_dev(&mut glyph);
    let _leftover = Leftover(pid);
    glyph.send_keys("shift+f5");
    glyph.wait_for_text("dev · stopped", WAIT);
    assert_gone(pid, &glyph);
    glyph.assert_running_for(Duration::from_millis(100));
}

#[test]
fn quit_kills_grandchild() {
    let mut glyph = open_project();
    let pid = run_dev(&mut glyph);
    let _leftover = Leftover(pid);
    glyph.send_keys("ctrl+q");
    glyph.wait_exit(WAIT);
    assert_gone(pid, &glyph);
}
