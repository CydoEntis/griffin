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
        let (chunk_tx, chunks) = mpsc::channel();
        thread::spawn(move || read_output(reader, &chunk_tx));
        let feeder_shared = Arc::clone(&shared);
        let reply_writer = Arc::clone(&writer);
        thread::spawn(move || {
            feed_output(&chunks, &feeder_shared, FrameGate::for_pty(), |reply| {
                if let Ok(mut w) = reply_writer.lock() {
                    let _ = w.write_all(reply);
                    let _ = w.flush();
                }
            });
        });

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
        if cfg!(windows)
            && let Some(bytes) = win32_input_bytes(notation)
        {
            return self.write(&bytes);
        }
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
        if let Err(screen) = wait_for_contents(&self.shared, text, timeout) {
            panic!("timed out after {timeout:?} waiting for {text:?}\n{screen}");
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

    /// The text of the cells on `row` whose foreground is `color`, in order.
    pub fn fg_text(&self, row: u16, color: vt100::Color) -> String {
        fg_cells(&self.parser(), row, color)
    }

    /// The background colour of the cell at (`col`, `row`).
    pub fn bg_at(&self, col: u16, row: u16) -> vt100::Color {
        self.parser()
            .screen()
            .cell(row, col)
            .map_or(vt100::Color::Default, |cell| cell.bgcolor())
    }

    /// Waits until the cell at (`col`, `row`) has background `color`. Panics with
    /// the screen on timeout.
    pub fn wait_for_bg(&self, col: u16, row: u16, color: vt100::Color, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut parser = self.parser();
        loop {
            let bg = parser
                .screen()
                .cell(row, col)
                .map_or(vt100::Color::Default, |cell| cell.bgcolor());
            if bg == color {
                return;
            }
            let now = Instant::now();
            if now >= deadline {
                panic!(
                    "timed out after {timeout:?} waiting for ({col}, {row}) to have                      background {color:?}; it has {bg:?}
{}",
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

    /// Waits until the cells on `row` drawn in `color` read exactly `text`.
    /// Panics with the screen on timeout.
    pub fn wait_for_fg(&self, row: u16, color: vt100::Color, text: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        let mut parser = self.parser();
        loop {
            let colored = fg_cells(&parser, row, color);
            if colored == text {
                return;
            }
            let now = Instant::now();
            if now >= deadline {
                panic!(
                    "timed out after {timeout:?} waiting for row {row} to show {text:?}                      in colour {color:?}; it shows {colored:?}
{}",
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

/// Reads PTY output and hands it on until the PTY closes. Kept apart from the
/// parsing so a pause in the output can be timed while a read blocks.
fn read_output(mut reader: Box<dyn Read + Send>, chunks: &mpsc::Sender<Vec<u8>>) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if chunks.send(buf[..n].to_vec()).is_err() {
                    return;
                }
            }
        }
    }
}

/// How long output must pause before held bytes are shown anyway, when the
/// synchronized-update markers are missing or can't be trusted to mark a frame.
const QUIET: Duration = Duration::from_millis(50);

/// Feeds output chunks into the screen a whole frame at a time until the sender
/// goes away. Answers cursor-position queries through `reply`: portable-pty opens
/// ConPTY with "inherit cursor", and ConPTY holds back all output until a terminal
/// replies to its opening `ESC [ 6 n`.
fn feed_output(
    chunks: &Receiver<Vec<u8>>,
    shared: &Shared,
    mut gate: FrameGate,
    mut reply: impl FnMut(&[u8]),
) {
    // The query can straddle two reads; keep enough of the previous chunk to see it.
    let mut carry: Vec<u8> = Vec::new();
    loop {
        let received = if gate.holding() {
            chunks.recv_timeout(QUIET)
        } else {
            chunks.recv().map_err(|_| RecvTimeoutError::Disconnected)
        };
        let chunk = match received {
            Ok(chunk) => chunk,
            Err(RecvTimeoutError::Timeout) => {
                let Ok(mut parser) = shared.parser.lock() else {
                    return;
                };
                gate.flush(&mut parser);
                drop(parser);
                shared.changed.notify_all();
                continue;
            }
            Err(RecvTimeoutError::Disconnected) => {
                if let Ok(mut parser) = shared.parser.lock() {
                    gate.flush(&mut parser);
                }
                shared.changed.notify_all();
                return;
            }
        };

        let mut window = std::mem::take(&mut carry);
        let carried = window.len();
        window.extend_from_slice(&chunk);
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
            gate.feed(&mut parser, &chunk);
            parser.screen().cursor_position()
        };
        shared.changed.notify_all();

        for _ in 0..queries {
            reply(format!("\x1b[{};{}R", cursor.0 + 1, cursor.1 + 1).as_bytes());
        }
    }
}

/// Waits until `text` is on screen; on timeout returns the screen dump.
fn wait_for_contents(shared: &Shared, text: &str, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    let mut parser = shared.parser.lock().expect("screen lock poisoned");
    loop {
        if parser.screen().contents().contains(text) {
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(dump(&screen_lines(&parser)));
        }
        parser = shared
            .changed
            .wait_timeout(parser, deadline - now)
            .expect("screen lock poisoned")
            .0;
    }
}

/// Begin and end synchronized update (DEC mode 2026), which griffin wraps around
/// each frame.
const SYNC_BEGIN: &[u8] = b"\x1b[?2026h";
const SYNC_END: &[u8] = b"\x1b[?2026l";

/// Holds output back until it is a whole frame, so a test never reads a
/// half-drawn screen. Output up to the end of the last whole synchronized frame
/// is shown at once; anything after it (an unfinished frame, or output with no
/// markers at all) waits for `flush`, which `feed_output` calls once output has
/// paused for `QUIET`.
#[derive(Default)]
struct FrameGate {
    pending: Vec<u8>,
    in_sync: bool,
    /// Off, every byte waits for a pause and the markers only get stripped.
    trust_markers: bool,
}

impl FrameGate {
    fn new(trust_markers: bool) -> Self {
        Self {
            trust_markers,
            ..Self::default()
        }
    }

    /// ConPTY passes the markers through as soon as it parses them but repaints
    /// the screen on its own timer, so on Windows a marked frame can arrive with
    /// only part of its cells (CI run 37509421685 showed the tab bar without the
    /// status line). There only a pause marks the end of a frame.
    fn for_pty() -> Self {
        Self::new(!cfg!(windows))
    }

    fn feed(&mut self, parser: &mut vt100::Parser, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        if !self.trust_markers {
            return;
        }
        // Rescanning from the start is safe: whatever is held never contains a
        // finished frame, and a marker split across reads is whole once its last
        // bytes arrive.
        let mut shown = 0;
        let mut at = 0;
        loop {
            let marker = if self.in_sync { SYNC_END } else { SYNC_BEGIN };
            let Some(pos) = find(&self.pending[at..], marker) else {
                break;
            };
            at += pos + marker.len();
            self.in_sync = !self.in_sync;
            if !self.in_sync {
                shown = at;
            }
        }
        self.process(parser, shown);
    }

    /// Shows everything held, treating an unfinished frame as done.
    fn flush(&mut self, parser: &mut vt100::Parser) {
        self.process(parser, self.pending.len());
        self.in_sync = false;
    }

    fn holding(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Feeds the first `len` held bytes to the parser, minus the markers.
    fn process(&mut self, parser: &mut vt100::Parser, len: usize) {
        let mut rest = &self.pending[..len];
        while !rest.is_empty() {
            let next = [SYNC_BEGIN, SYNC_END]
                .iter()
                .filter_map(|marker| find(rest, marker).map(|pos| (pos, marker.len())))
                .min();
            match next {
                Some((pos, marker_len)) => {
                    parser.process(&rest[..pos]);
                    rest = &rest[pos + marker_len..];
                }
                None => {
                    parser.process(rest);
                    rest = &[];
                }
            }
        }
        self.pending.drain(..len);
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

fn fg_cells(parser: &vt100::Parser, row: u16, color: vt100::Color) -> String {
    let screen = parser.screen();
    (0..COLS)
        .filter_map(|col| screen.cell(row, col))
        .filter(|cell| cell.fgcolor() == color && !cell.is_wide_continuation())
        .map(|cell| cell.contents().to_string())
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

/// ConPTY drops CSI u keys it doesn't know, so on Windows the keys that need CSI u
/// go in win32-input-mode instead (`ESC [ Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`, a press
/// then a release), which is what Windows Terminal itself sends ConPTY.
pub fn win32_input_bytes(notation: &str) -> Option<Vec<u8>> {
    const SHIFT_PRESSED: u16 = 0x10;
    // Virtual key, scan code and character of the key.
    let (vk, scan, ch, state) = match notation.trim().to_ascii_lowercase().as_str() {
        "shift+enter" => (0x0D, 0x1C, 13, SHIFT_PRESSED),
        _ => return None,
    };
    Some(
        format!("\x1b[{vk};{scan};{ch};1;{state};1_\x1b[{vk};{scan};{ch};0;{state};1_")
            .into_bytes(),
    )
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
    fn shift_enter_has_a_win32_input_mode_form_for_conpty() {
        assert_eq!(
            win32_input_bytes("shift+enter").as_deref(),
            Some(&b"\x1b[13;28;13;1;16;1_\x1b[13;28;13;0;16;1_"[..])
        );
        assert_eq!(win32_input_bytes("enter"), None);
    }

    fn gate_contents(gate: &mut FrameGate, parser: &mut vt100::Parser, bytes: &[u8]) -> String {
        gate.feed(parser, bytes);
        parser.screen().contents()
    }

    #[test]
    fn frames_are_shown_whole() {
        let mut parser = vt100::Parser::new(2, 10, 0);
        let mut gate = FrameGate::new(true);
        // Markers split across reads still mark the frame.
        assert_eq!(gate_contents(&mut gate, &mut parser, b"ab\x1b[?20"), "");
        assert_eq!(gate_contents(&mut gate, &mut parser, b"26hcd"), "");
        assert_eq!(gate_contents(&mut gate, &mut parser, b"ef\x1b[?2026"), "");
        assert_eq!(gate_contents(&mut gate, &mut parser, b"lgh"), "abcdef");
        // What follows the last whole frame waits for a pause.
        assert!(gate.holding());
        gate.flush(&mut parser);
        assert_eq!(parser.screen().contents(), "abcdefgh");
        assert!(!gate.holding());
    }

    #[test]
    fn output_without_markers_is_held_until_a_pause() {
        let mut parser = vt100::Parser::new(2, 10, 0);
        let mut gate = FrameGate::new(true);
        assert_eq!(gate_contents(&mut gate, &mut parser, b"half"), "");
        assert_eq!(gate_contents(&mut gate, &mut parser, b" rest"), "");
        gate.flush(&mut parser);
        assert_eq!(parser.screen().contents(), "half rest");
    }

    #[test]
    fn a_frame_missing_its_end_is_shown_after_a_pause() {
        let mut parser = vt100::Parser::new(2, 10, 0);
        let mut gate = FrameGate::new(true);
        assert_eq!(gate_contents(&mut gate, &mut parser, b"\x1b[?2026hab"), "");
        gate.flush(&mut parser);
        assert_eq!(parser.screen().contents(), "ab");
        // Later frames with both markers are shown at once again.
        assert_eq!(
            gate_contents(&mut gate, &mut parser, b"\x1b[?2026hcd\x1b[?2026l"),
            "abcd"
        );
    }

    #[test]
    fn untrusted_markers_wait_for_a_pause() {
        let mut parser = vt100::Parser::new(2, 10, 0);
        let mut gate = FrameGate::new(false);
        assert_eq!(
            gate_contents(&mut gate, &mut parser, b"[?2026hab[?2026l"),
            ""
        );
        assert_eq!(gate_contents(&mut gate, &mut parser, b"cd"), "");
        gate.flush(&mut parser);
        assert_eq!(parser.screen().contents(), "abcd");
    }

    /// Runs `feed_output` on a fresh screen and sends it `chunks` back to back.
    /// Returns the sender too, so the stream stays open: closing it would flush.
    fn stream(chunks: &[&[u8]]) -> (Arc<Shared>, mpsc::Sender<Vec<u8>>, Instant) {
        let shared = Arc::new(Shared {
            parser: Mutex::new(vt100::Parser::new(ROWS, COLS, 0)),
            changed: Condvar::new(),
        });
        let (tx, rx) = mpsc::channel();
        let feeder = Arc::clone(&shared);
        thread::spawn(move || feed_output(&rx, &feeder, FrameGate::new(true), |_| {}));
        for chunk in chunks {
            tx.send(chunk.to_vec()).unwrap();
        }
        (shared, tx, Instant::now())
    }

    #[test]
    fn wait_shows_a_marker_less_stream_once_it_goes_quiet() {
        let (shared, _tx, sent) = stream(&[b"top\r\n", b"middle\r\n", b"bottom"]);
        wait_for_contents(&shared, "top", Duration::from_secs(5)).unwrap();
        assert!(sent.elapsed() >= QUIET, "shown before the output paused");
        let contents = shared.parser.lock().unwrap().screen().contents();
        assert!(
            contents.contains("bottom"),
            "half a frame shown: {contents:?}"
        );
    }

    #[test]
    fn wait_shows_a_stream_with_split_or_missing_markers() {
        let (shared, _tx, _) = stream(&[b"\x1b[?20", b"26hx", b"y\x1b[?2026", b"lz"]);
        wait_for_contents(&shared, "xy", Duration::from_secs(5)).unwrap();
        wait_for_contents(&shared, "xyz", Duration::from_secs(5)).unwrap();

        let (shared, _tx, _) = stream(&[b"\x1b[?2026hno", b" end"]);
        wait_for_contents(&shared, "no end", Duration::from_secs(5)).unwrap();
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
