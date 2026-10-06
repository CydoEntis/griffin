//! PTY harness: runs the real `griffin` binary in a 100x30 pseudo-terminal, sends
//! keys and mouse input as terminal bytes, and reads the screen back through vt100.
//!
//! Integration tests include it with `mod harness;`.

// Each test crate compiles its own copy of this module and uses only part of it.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{
    ChildKiller, CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system,
};
use tempfile::{NamedTempFile, TempDir};

pub const COLS: u16 = 100;
pub const ROWS: u16 = 30;

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Device status report "where is the cursor?".
const CURSOR_QUERY: &[u8] = b"[6n";

/// Screen state shared with the reader thread.
struct Shared {
    parser: Mutex<vt100::Parser>,
    changed: Condvar,
}

/// A running `griffin` inside a pseudo-terminal. Dropping it kills the process.
pub struct Griffin {
    shared: Arc<Shared>,
    writer: SharedWriter,
    killer: Box<dyn ChildKiller + Send + Sync>,
    exit: Receiver<std::io::Result<ExitStatus>>,
    exited: Option<ExitStatus>,
    // Kept alive so the PTY stays open for the reader and the child.
    _master: Box<dyn MasterPty + Send>,
    // Kept alive so `GRIFFIN_CONFIG` points at a real file until the end.
    _config: NamedTempFile,
    // The run's own `GRIFFIN_DATA_DIR` when the test didn't pick one, so backups
    // never land in the real data dir.
    _data: Option<TempDir>,
}

impl Griffin {
    /// Starts griffin in the current directory (the crate root under `cargo test`).
    pub fn spawn(args: &[&str]) -> Self {
        let dir = std::env::current_dir().expect("test process has a current directory");
        Self::spawn_in(&dir, args)
    }

    /// Starts griffin with `dir` as its working directory.
    pub fn spawn_in(dir: &Path, args: &[&str]) -> Self {
        Self::spawn_in_with_config(dir, "", args)
    }

    /// Starts griffin in the current directory with `toml` as its `config.toml`.
    pub fn spawn_with_config(toml: &str, args: &[&str]) -> Self {
        let dir = std::env::current_dir().expect("test process has a current directory");
        Self::spawn_in_with_config(&dir, toml, args)
    }

    /// Starts griffin with `dir` as its working directory and `toml` as its config.
    pub fn spawn_in_with_config(dir: &Path, toml: &str, args: &[&str]) -> Self {
        Self::spawn_full(dir, toml, None, args)
    }

    /// Starts griffin in `dir` with `data` as its data dir, which outlives this run
    /// so a relaunch can find what the last one left there.
    pub fn spawn_in_with_data(dir: &Path, data: &Path, args: &[&str]) -> Self {
        Self::spawn_full(dir, "", Some(data.to_path_buf()), args)
    }

    fn spawn_full(dir: &Path, toml: &str, data: Option<PathBuf>, args: &[&str]) -> Self {
        let (data, owned_data) = match data {
            Some(data) => (data, None),
            None => {
                let temp = tempfile::tempdir().expect("create temp data dir");
                (temp.path().to_path_buf(), Some(temp))
            }
        };
        let mut config = NamedTempFile::new().expect("create temp config");
        config
            .write_all(toml.as_bytes())
            .and_then(|()| config.flush())
            .expect("write temp config");

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open pseudo-terminal");

        let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_griffin"));
        cmd.args(args);
        cmd.cwd(dir);
        cmd.env("GRIFFIN_CONFIG", config.path());
        cmd.env("GRIFFIN_DATA_DIR", &data);
        cmd.env("TERM", "xterm-256color");

        let mut child = pair.slave.spawn_command(cmd).expect("spawn griffin");
        // The child holds its own handle; ours would keep the PTY open after it exits.
        drop(pair.slave);

        let killer = child.clone_killer();
        let (exit_tx, exit) = mpsc::channel();
        thread::spawn(move || {
            let _ = exit_tx.send(child.wait());
        });

        let shared = Arc::new(Shared {
            parser: Mutex::new(vt100::Parser::new(ROWS, COLS, 0)),
            changed: Condvar::new(),
        });

        let writer: SharedWriter = Arc::new(Mutex::new(
            pair.master.take_writer().expect("take PTY writer"),
        ));

        // ConPTY stalls the child when its output isn't read, so drain it always.
        let reader = pair.master.try_clone_reader().expect("clone PTY reader");
        let reader_shared = Arc::clone(&shared);
        let reply_writer = Arc::clone(&writer);
        thread::spawn(move || pump_output(reader, &reader_shared, &reply_writer));

        Self {
            shared,
            writer,
            killer,
            exit,
            exited: None,
            _master: pair.master,
            _config: config,
            _data: owned_data,
        }
    }

    /// Kills griffin without letting it clean up, as a crash would, and waits for
    /// it to be gone.
    pub fn kill(&mut self) {
        if self.exited.is_none() {
            let _ = self.killer.kill();
            self.wait_exit(Duration::from_secs(5));
        }
    }

    /// Sends one key in `[keys]` notation, e.g. `ctrl+s`, `alt+,`, `shift+f5`.
    pub fn send_keys(&mut self, notation: &str) {
        let bytes = key_bytes(notation).unwrap_or_else(|err| panic!("{err}"));
        self.write(&bytes);
    }

    /// Types literal text as-is.
    pub fn type_text(&mut self, text: &str) {
        self.write(text.as_bytes());
    }

    pub fn click(&mut self, col: u16, row: u16) {
        self.write(&click_bytes(col, row));
    }

    pub fn middle_click(&mut self, col: u16, row: u16) {
        self.write(&middle_click_bytes(col, row));
    }

    pub fn double_click(&mut self, col: u16, row: u16) {
        self.write(&double_click_bytes(col, row));
    }

    pub fn drag(&mut self, from: (u16, u16), to: (u16, u16)) {
        self.write(&drag_bytes(from, to));
    }

    pub fn scroll_down(&mut self, col: u16, row: u16) {
        self.write(&scroll_down_bytes(col, row));
    }

    pub fn scroll_up(&mut self, col: u16, row: u16) {
        self.write(&scroll_up_bytes(col, row));
    }

    /// Writes raw bytes to the terminal input.
    pub fn write(&mut self, bytes: &[u8]) {
        let mut writer = self.writer.lock().expect("PTY writer lock poisoned");
        writer.write_all(bytes).expect("write to PTY");
        writer.flush().expect("flush PTY");
    }

    /// The screen as `ROWS` lines, trailing spaces trimmed.
    pub fn screen(&self) -> Vec<String> {
        screen_lines(&self.parser())
    }

    /// The screen column (0-based) where `text` starts on `row`, counting wide
    /// characters as the two cells they fill. `None` if `text` isn't on that row.
    pub fn text_col(&self, row: u16, text: &str) -> Option<u16> {
        let parser = self.parser();
        let screen = parser.screen();
        let mut line = String::new();
        // Byte offset in `line` where each drawn cell's text starts, with its column.
        let mut starts: Vec<(usize, u16)> = Vec::new();
        for col in 0..COLS {
            let Some(cell) = screen.cell(row, col) else {
                break;
            };
            if cell.is_wide_continuation() {
                continue;
            }
            starts.push((line.len(), col));
            let contents = cell.contents();
            line.push_str(if contents.is_empty() { " " } else { contents });
        }
        let at = line.find(text)?;
        starts
            .iter()
            .rev()
            .find(|(offset, _)| *offset <= at)
            .map(|&(_, col)| col)
    }

    /// Cursor position as (col, row), both 0-based.
    pub fn cursor(&self) -> (u16, u16) {
        let (row, col) = self.parser().screen().cursor_position();
        // vt100 reports col == COLS while a wrap is pending after the last cell is
        // written; a real terminal shows the cursor on that last cell.
        (col.min(COLS - 1), row)
    }

    /// Waits until `text` appears anywhere on screen. Panics with the screen on timeout.
    pub fn wait_for_text(&self, text: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut parser = self.parser();
        loop {
            if parser.screen().contents().contains(text) {
                return;
            }
            let now = Instant::now();
            if now >= deadline {
                panic!(
                    "timed out after {timeout:?} waiting for {text:?}\n{}",
                    dump(&screen_lines(&parser))
                );
            }
            parser = self
                .shared
                .changed
                .wait_timeout(parser, deadline - now)
                .expect("screen lock poisoned")
                .0;
        }
    }

    /// Waits until `text` is nowhere on screen. Panics with the screen on timeout.
    pub fn wait_for_text_gone(&self, text: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut parser = self.parser();
        loop {
            if !parser.screen().contents().contains(text) {
                return;
            }
            let now = Instant::now();
            if now >= deadline {
                panic!(
                    "timed out after {timeout:?} waiting for {text:?} to go away\n{}",
                    dump(&screen_lines(&parser))
                );
            }
            parser = self
                .shared
                .changed
                .wait_timeout(parser, deadline - now)
                .expect("screen lock poisoned")
                .0;
        }
    }

    /// Waits until the cursor is at (col, row), 0-based. The cursor moves after the
    /// frame's text is written, so seeing new text doesn't mean the cursor is there
    /// yet. Panics with the screen and the cursor on timeout.
    pub fn wait_for_cursor(&self, col: u16, row: u16, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut parser = self.parser();
        loop {
            let (r, c) = parser.screen().cursor_position();
            if (c.min(COLS - 1), r) == (col, row) {
                return;
            }
            let now = Instant::now();
            if now >= deadline {
                panic!(
                    "timed out after {timeout:?} waiting for the cursor at ({col}, {row}); \
                     it is at ({c}, {r})\n{}",
                    dump(&screen_lines(&parser))
                );
            }
            parser = self
                .shared
                .changed
                .wait_timeout(parser, deadline - now)
                .expect("screen lock poisoned")
                .0;
        }
    }

    /// The text of the reverse-video cells on `row`, in order. A blank reversed
    /// cell counts as a space.
    pub fn reversed_text(&self, row: u16) -> String {
        reversed_cells(&self.parser(), row)
    }

    /// Waits until the reverse-video cells on `row` read exactly `text`. Panics with
    /// the screen on timeout.
    pub fn wait_for_reversed(&self, row: u16, text: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut parser = self.parser();
        loop {
            let reversed = reversed_cells(&parser, row);
            if reversed == text {
                return;
            }
            let now = Instant::now();
            if now >= deadline {
                panic!(
                    "timed out after {timeout:?} waiting for row {row} to show {text:?} \
                     reversed; it shows {reversed:?}\n{}",
                    dump(&screen_lines(&parser))
                );
            }
            parser = self
                .shared
                .changed
                .wait_timeout(parser, deadline - now)
                .expect("screen lock poisoned")
                .0;
        }
    }

    /// Waits until `check` holds for the screen lines (as `screen` returns them), so
    /// a test can wait for a whole layout rather than one string. Panics with `what`
    /// and the screen on timeout.
    pub fn wait_for_screen(
        &self,
        what: &str,
        timeout: Duration,
        check: impl Fn(&[String]) -> bool,
    ) {
        let deadline = Instant::now() + timeout;
        let mut parser = self.parser();
        loop {
            let lines = screen_lines(&parser);
            if check(&lines) {
                return;
            }
            let now = Instant::now();
            if now >= deadline {
                panic!(
                    "timed out after {timeout:?} waiting for {what}\n{}",
                    dump(&lines)
                );
            }
            parser = self
                .shared
                .changed
                .wait_timeout(parser, deadline - now)
                .expect("screen lock poisoned")
                .0;
        }
    }

    /// Checks griffin is still running after `grace`. Proving a key did nothing
    /// needs some window to wait in; this returns early if griffin exits.
    pub fn assert_running_for(&mut self, grace: Duration) {
        if self.exited.is_some() {
            panic!(
                "griffin already exited
{}",
                dump(&self.screen())
            );
        }
        match self.exit.recv_timeout(grace) {
            Err(RecvTimeoutError::Timeout) => {}
            Ok(status) => panic!(
                "griffin exited unexpectedly with {status:?}
{}",
                dump(&self.screen())
            ),
            Err(RecvTimeoutError::Disconnected) => panic!("griffin's wait thread vanished"),
        }
    }

    /// Waits for griffin to exit and returns its status. Panics with the screen on timeout.
    pub fn wait_exit(&mut self, timeout: Duration) -> ExitStatus {
        if let Some(status) = &self.exited {
            return status.clone();
        }
        match self.exit.recv_timeout(timeout) {
            Ok(Ok(status)) => {
                self.exited = Some(status.clone());
                status
            }
            Ok(Err(err)) => panic!("waiting for griffin failed: {err}"),
            Err(RecvTimeoutError::Timeout) => panic!(
                "griffin did not exit within {timeout:?}\n{}",
                dump(&self.screen())
            ),
            Err(RecvTimeoutError::Disconnected) => panic!("griffin's wait thread vanished"),
        }
    }

    /// Waits until `check` holds for the file system. Files have no change signal
    /// to wait on, so this polls. Panics with `what` and the screen on timeout.
    pub fn wait_for_files(&self, what: &str, timeout: Duration, check: impl Fn() -> bool) {
        let deadline = Instant::now() + timeout;
        while !check() {
            if Instant::now() >= deadline {
                panic!(
                    "timed out after {timeout:?} waiting for {what}
{}",
                    dump(&self.screen())
                );
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn parser(&self) -> MutexGuard<'_, vt100::Parser> {
        self.shared.parser.lock().expect("screen lock poisoned")
    }
}

impl Drop for Griffin {
    fn drop(&mut self) {
        if self.exited.is_none() && self.exit.try_recv().is_err() {
            let _ = self.killer.kill();
        }
        // The reader thread is left detached: on Windows its read only returns once
        // the pseudo-console closes, which happens when `_master` drops here.
    }
}

/// Feeds PTY output into the parser until the PTY closes. Answers cursor-position
/// queries itself: portable-pty opens ConPTY with "inherit cursor", and ConPTY
/// holds back all output until a terminal replies to its opening `ESC [ 6 n`.
fn pump_output(mut reader: Box<dyn Read + Send>, shared: &Shared, writer: &SharedWriter) {
    let mut buf = [0u8; 8192];
    // The query can straddle two reads; keep enough of the previous chunk to see it.
    let mut carry: Vec<u8> = Vec::new();
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let chunk = &buf[..n];
        let mut window = std::mem::take(&mut carry);
        let carried = window.len();
        window.extend_from_slice(chunk);

        let mut queries = 0;
        let mut i = 0;
        while let Some(pos) = find(&window[i..], CURSOR_QUERY) {
            // Matches wholly inside the carried bytes were answered last time.
            if i + pos + CURSOR_QUERY.len() > carried {
                queries += 1;
            }
            i += pos + CURSOR_QUERY.len();
        }
        let keep = window.len().min(CURSOR_QUERY.len() - 1);
        carry = window[window.len() - keep..].to_vec();

        let cursor = {
            let Ok(mut parser) = shared.parser.lock() else {
                return;
            };
            parser.process(chunk);
            parser.screen().cursor_position()
        };
        shared.changed.notify_all();

        for _ in 0..queries {
            let reply = format!("[{};{}R", cursor.0 + 1, cursor.1 + 1);
            if let Ok(mut w) = writer.lock() {
                let _ = w.write_all(reply.as_bytes());
                let _ = w.flush();
            }
        }
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn screen_lines(parser: &vt100::Parser) -> Vec<String> {
    parser
        .screen()
        .rows(0, COLS)
        .map(|row| row.trim_end().to_string())
        .collect()
}

fn reversed_cells(parser: &vt100::Parser, row: u16) -> String {
    let screen = parser.screen();
    (0..COLS)
        .filter_map(|col| screen.cell(row, col))
        .filter(|cell| cell.inverse() && !cell.is_wide_continuation())
        .map(|cell| match cell.contents() {
            "" => " ".to_string(),
            text => text.to_string(),
        })
        .collect()
}

fn dump(lines: &[String]) -> String {
    let mut out = String::from("--- screen ---\n");
    for (i, line) in lines.iter().enumerate() {
        out.push_str(&format!("{i:>2}|{line}\n"));
    }
    out.push_str("--------------");
    out
}

#[derive(Debug, Default, Clone, Copy)]
struct Mods {
    ctrl: bool,
    alt: bool,
    shift: bool,
}

impl Mods {
    /// xterm modifier parameter: 1 + shift + 2*alt + 4*ctrl.
    fn param(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }

    fn any(self) -> bool {
        self.ctrl || self.alt || self.shift
    }
}

/// Turns one key in `[keys]` notation into the bytes an xterm-compatible terminal
/// sends for it. Keys legacy encoding can't express (e.g. `shift+enter`) use CSI u.
pub fn key_bytes(notation: &str) -> Result<Vec<u8>, String> {
    let lower = notation.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return Err("empty key notation".into());
    }
    // A trailing "+" is the plus key itself (`ctrl++`), not a separator.
    let (prefix, key) = match lower.strip_suffix("++") {
        Some(rest) => (rest, "+"),
        None if lower == "+" => ("", "+"),
        None => match lower.rsplit_once('+') {
            Some((prefix, key)) => (prefix, key),
            None => ("", lower.as_str()),
        },
    };

    let mut mods = Mods::default();
    for part in prefix.split('+').filter(|p| !p.is_empty()) {
        match part {
            "ctrl" | "control" => mods.ctrl = true,
            "alt" | "meta" => mods.alt = true,
            "shift" => mods.shift = true,
            other => return Err(format!("unknown modifier {other:?} in {notation:?}")),
        }
    }

    let csi_mod = |final_byte: char| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[1;{}{final_byte}", mods.param()).into_bytes()
        } else {
            format!("\x1b[{final_byte}").into_bytes()
        }
    };
    let tilde = |code: u8| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[{code};{}~", mods.param()).into_bytes()
        } else {
            format!("\x1b[{code}~").into_bytes()
        }
    };
    let ss3 = |final_byte: char| -> Vec<u8> {
        if mods.any() {
            format!("\x1b[1;{}{final_byte}", mods.param()).into_bytes()
        } else {
            format!("\x1bO{final_byte}").into_bytes()
        }
    };
    // Plain byte, Alt as an ESC prefix; Ctrl/Shift fall back to CSI u.
    let simple = |byte: u8, codepoint: u32| -> Vec<u8> {
        if mods.ctrl || mods.shift {
            format!("\x1b[{codepoint};{}u", mods.param()).into_bytes()
        } else if mods.alt {
            vec![0x1b, byte]
        } else {
            vec![byte]
        }
    };

    let bytes = match key {
        "up" => csi_mod('A'),
        "down" => csi_mod('B'),
        "right" => csi_mod('C'),
        "left" => csi_mod('D'),
        "home" => csi_mod('H'),
        "end" => csi_mod('F'),
        "insert" | "ins" => tilde(2),
        "delete" | "del" => tilde(3),
        "pageup" | "pgup" => tilde(5),
        "pagedown" | "pgdn" => tilde(6),
        "f1" => ss3('P'),
        "f2" => ss3('Q'),
        "f3" => ss3('R'),
        "f4" => ss3('S'),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "enter" | "return" => simple(b'\r', 13),
        "esc" | "escape" => simple(0x1b, 27),
        "backspace" => simple(0x7f, 127),
        "tab" if mods.shift && !mods.ctrl && !mods.alt => b"\x1b[Z".to_vec(),
        "tab" => simple(b'\t', 9),
        "space" => char_bytes(' ', mods),
        other => {
            let mut chars = other.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => char_bytes(c, mods),
                _ => return Err(format!("unknown key {other:?} in {notation:?}")),
            }
        }
    };
    Ok(bytes)
}

fn char_bytes(c: char, mods: Mods) -> Vec<u8> {
    let mut out = Vec::new();
    if mods.alt {
        out.push(0x1b);
    }
    if mods.ctrl {
        let control = match c {
            'a'..='z' => Some(c as u8 - b'a' + 1),
            ' ' | '@' | '2' => Some(0),
            '[' | '3' => Some(0x1b),
            '\\' | '4' => Some(0x1c),
            ']' | '5' => Some(0x1d),
            '^' | '6' => Some(0x1e),
            '/' | '_' | '7' => Some(0x1f),
            _ => None,
        };
        match control {
            // Legacy control bytes can't carry Shift, so Ctrl+Shift goes to CSI u.
            Some(byte) if !mods.shift => out.push(byte),
            _ => {
                return format!("\x1b[{};{}u", c as u32, mods.param()).into_bytes();
            }
        }
        return out;
    }
    let c = if mods.shift {
        c.to_ascii_uppercase()
    } else {
        c
    };
    let mut buf = [0u8; 4];
    out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    out
}

fn sgr(button: u8, col: u16, row: u16, press: bool) -> Vec<u8> {
    let end = if press { 'M' } else { 'm' };
    format!("\x1b[<{button};{};{}{end}", col + 1, row + 1).into_bytes()
}

/// Left press and release at a 0-based cell.
pub fn click_bytes(col: u16, row: u16) -> Vec<u8> {
    let mut out = sgr(0, col, row, true);
    out.extend(sgr(0, col, row, false));
    out
}

/// Middle press and release at a 0-based cell.
pub fn middle_click_bytes(col: u16, row: u16) -> Vec<u8> {
    let mut out = sgr(1, col, row, true);
    out.extend(sgr(1, col, row, false));
    out
}

pub fn double_click_bytes(col: u16, row: u16) -> Vec<u8> {
    let mut out = click_bytes(col, row);
    out.extend(click_bytes(col, row));
    out
}

/// Left press at `from`, motion with the button held to `to`, release at `to`.
pub fn drag_bytes(from: (u16, u16), to: (u16, u16)) -> Vec<u8> {
    let mut out = sgr(0, from.0, from.1, true);
    out.extend(sgr(32, to.0, to.1, true));
    out.extend(sgr(0, to.0, to.1, false));
    out
}

pub fn scroll_up_bytes(col: u16, row: u16) -> Vec<u8> {
    sgr(64, col, row, true)
}

pub fn scroll_down_bytes(col: u16, row: u16) -> Vec<u8> {
    sgr(65, col, row, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(notation: &str) -> Vec<u8> {
        key_bytes(notation).unwrap()
    }

    #[test]
    fn ctrl_letters_are_control_bytes() {
        assert_eq!(keys("ctrl+q"), b"\x11");
        assert_eq!(keys("ctrl+s"), b"\x13");
        assert_eq!(keys("Ctrl+A"), b"\x01");
    }

    #[test]
    fn alt_prefixes_escape() {
        assert_eq!(keys("alt+."), b"\x1b.");
        assert_eq!(keys("alt+,"), b"\x1b,");
        assert_eq!(keys("alt+1"), b"\x1b1");
        assert_eq!(keys("alt+enter"), b"\x1b\r");
    }

    #[test]
    fn function_keys_carry_modifiers() {
        assert_eq!(keys("f5"), b"\x1b[15~");
        assert_eq!(keys("shift+f5"), b"\x1b[15;2~");
        assert_eq!(keys("ctrl+f5"), b"\x1b[15;5~");
        assert_eq!(keys("f1"), b"\x1bOP");
        assert_eq!(keys("shift+f8"), b"\x1b[19;2~");
        assert_eq!(keys("f12"), b"\x1b[24~");
    }

    #[test]
    fn arrows_carry_modifiers() {
        assert_eq!(keys("right"), b"\x1b[C");
        assert_eq!(keys("shift+right"), b"\x1b[1;2C");
        assert_eq!(keys("shift+left"), b"\x1b[1;2D");
        assert_eq!(keys("alt+left"), b"\x1b[1;3D");
        assert_eq!(keys("ctrl+shift+right"), b"\x1b[1;6C");
    }

    #[test]
    fn named_keys() {
        assert_eq!(keys("enter"), b"\r");
        assert_eq!(keys("esc"), b"\x1b");
        assert_eq!(keys("tab"), b"\t");
        assert_eq!(keys("shift+tab"), b"\x1b[Z");
        assert_eq!(keys("backspace"), b"\x7f");
        assert_eq!(keys("delete"), b"\x1b[3~");
        assert_eq!(keys("shift+enter"), b"\x1b[13;2u");
        assert_eq!(keys("alt+/"), b"\x1b/");
        assert_eq!(keys("ctrl++"), b"\x1b[43;5u");
    }

    #[test]
    fn bad_notation_is_an_error() {
        assert!(key_bytes("hyper+q").is_err());
        assert!(key_bytes("ctrl+nope").is_err());
        assert!(key_bytes("").is_err());
    }

    #[test]
    fn left_click_is_sgr_press_and_release() {
        assert_eq!(click_bytes(10, 5), b"\x1b[<0;11;6M\x1b[<0;11;6m");
    }

    #[test]
    fn drag_and_wheel_use_sgr_codes() {
        assert_eq!(
            drag_bytes((1, 2), (4, 2)),
            b"\x1b[<0;2;3M\x1b[<32;5;3M\x1b[<0;5;3m"
        );
        assert_eq!(scroll_up_bytes(0, 0), b"\x1b[<64;1;1M");
        assert_eq!(scroll_down_bytes(0, 0), b"\x1b[<65;1;1M");
        assert_eq!(
            double_click_bytes(0, 0),
            b"\x1b[<0;1;1M\x1b[<0;1;1m\x1b[<0;1;1M\x1b[<0;1;1m"
        );
    }
}
