pub mod edit;
pub mod history;
pub mod movement;
pub mod selection;

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use ropey::Rope;

use crate::highlight::Highlighter;
use crate::save::save_atomic;
use history::History;

/// The line ending a file used on disk. The rope always holds LF; saving writes
/// this one back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LineEnding {
    #[default]
    Lf,
    Crlf,
}

impl LineEnding {
    /// Decided by the first line break; a file without one is LF.
    pub fn detect(text: &str) -> Self {
        match text.find('\n') {
            Some(i) if text[..i].ends_with('\r') => LineEnding::Crlf,
            _ => LineEnding::Lf,
        }
    }
}

/// Where a cursor and selection sit in a buffer, apart from the buffer: each split
/// showing a buffer keeps its own while another split has the buffer's live one.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Caret {
    pub cursor: usize,
    pub goal_col: Option<usize>,
    pub anchor: Option<usize>,
}

/// One open file (or an untitled scratch buffer).
#[derive(Debug, Default)]
pub struct Buffer {
    pub rope: Rope,
    pub path: Option<PathBuf>,
    pub line_ending: LineEnding,
    /// Set by every edit, cleared by saving; the status bar marks it with ` •`.
    pub dirty: bool,
    /// Char index into `rope`; at most `rope.len_chars()`.
    pub cursor: usize,
    /// Display column Up/Down aim for, so crossing a shorter line doesn't lose it.
    /// Cleared by any other motion.
    pub goal_col: Option<usize>,
    /// Where a selection started; the selection runs from here to the cursor. Set
    /// by Shift+movement and select all, cleared by plain movement and any edit.
    pub anchor: Option<usize>,
    pub history: History,
    /// Colours the text when its file type has a grammar; see `sync_highlight`.
    pub highlighter: Option<Highlighter>,
    /// The path `highlighter` was picked for, so a rename or save-as to another
    /// file type picks again.
    pub highlight_for: Option<PathBuf>,
    /// Bumped by every change to the text, so language servers can tell what
    /// they haven't heard about yet without comparing texts.
    pub revision: u64,
    /// Bumped by every successful save, for the same reason.
    pub saves: u64,
}

impl Buffer {
    /// An untitled, empty buffer.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Reads `path` into a buffer. A file that doesn't exist yet opens as an empty
    /// buffer with that path, so saving creates it. Non-UTF-8 files are refused
    /// rather than decoded lossily, since saving would then corrupt them.
    pub fn open(path: &Path) -> Result<Self> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                return Ok(Self {
                    path: Some(path.to_path_buf()),
                    ..Self::empty()
                });
            }
            Err(err) => return Err(err.into()),
        };
        let text = String::from_utf8(bytes).map_err(|_| anyhow!("not UTF-8"))?;
        Ok(Self::from_text(&text, Some(path.to_path_buf())))
    }

    fn from_text(text: &str, path: Option<PathBuf>) -> Self {
        let line_ending = LineEnding::detect(text);
        let rope = if text.contains("\r\n") {
            Rope::from_str(&text.replace("\r\n", "\n"))
        } else {
            Rope::from_str(text)
        };
        Self {
            rope,
            path,
            line_ending,
            dirty: false,
            cursor: 0,
            goal_col: None,
            anchor: None,
            history: History::default(),
            highlighter: None,
            highlight_for: None,
            revision: 0,
            saves: 0,
        }
    }

    /// Writes the buffer to its path atomically, with the line ending the file had
    /// on disk, and clears `dirty`. An untitled buffer can't be saved yet.
    pub fn save(&mut self) -> Result<()> {
        let Some(path) = &self.path else {
            bail!("no file name");
        };
        save_atomic(path, &self.disk_text())?;
        self.dirty = false;
        self.saves += 1;
        Ok(())
    }

    /// The text as it goes to disk, with the file's line ending. Backups use it
    /// too, so a backup compares byte for byte with the file it stands in for.
    pub fn disk_text(&self) -> String {
        let text = self.rope.to_string();
        match self.line_ending {
            LineEnding::Lf => text,
            LineEnding::Crlf => text.replace('\n', "\r\n"),
        }
    }

    /// Replaces the text with a recovered backup of this file. The result differs
    /// from what's on disk, so it is dirty; undo can't reach the file's text.
    pub fn recover(&mut self, text: &str) {
        *self = Self {
            dirty: true,
            // The text changed under anything following the old one.
            revision: self.revision + 1,
            saves: self.saves,
            ..Self::from_text(text, self.path.take())
        };
    }

    /// The cursor and selection, to keep while another split takes over.
    pub fn caret(&self) -> Caret {
        Caret {
            cursor: self.cursor,
            goal_col: self.goal_col,
            anchor: self.anchor,
        }
    }

    /// Puts back a kept cursor and selection. Edits made through another split may
    /// have shortened the text since, so both ends are clamped to it.
    pub fn set_caret(&mut self, caret: Caret) {
        let len = self.rope.len_chars();
        self.cursor = caret.cursor.min(len);
        self.goal_col = caret.goal_col;
        self.anchor = caret.anchor.map(|anchor| anchor.min(len));
    }

    /// A copy of the text with `caret` as its cursor, for drawing a split that
    /// doesn't hold the live cursor. Cloning a rope shares its nodes, so this is
    /// cheap; the copy has no path or history and is never edited.
    pub fn with_caret(&self, caret: Caret) -> Buffer {
        let mut copy = Buffer {
            rope: self.rope.clone(),
            highlighter: self.highlighter.clone(),
            ..Buffer::empty()
        };
        copy.set_caret(caret);
        copy
    }

    /// Picks the highlighter for the buffer's path if the path changed since the
    /// last pick, then brings its tree up to date with the text. Called before
    /// drawing, so rendering only reads the tree; a buffer that's never drawn
    /// (project search opens files too) never pays for a parse.
    pub fn sync_highlight(&mut self) {
        if self.highlight_for != self.path {
            self.highlighter = self.path.as_deref().and_then(Highlighter::for_path);
            self.highlight_for.clone_from(&self.path);
        }
        if let Some(highlighter) = &mut self.highlighter {
            highlighter.parse(&self.rope);
        }
    }

    /// What the status line calls this buffer.
    pub fn name(&self) -> String {
        match &self.path {
            Some(path) => path.display().to_string(),
            None => "untitled".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_bytes(bytes: &[u8]) -> Result<Buffer> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("file.txt");
        fs::write(&path, bytes)?;
        Buffer::open(&path)
    }

    #[test]
    fn lf_file_stays_lf() -> Result<()> {
        let buf = open_bytes(b"one\ntwo\n")?;
        assert_eq!(buf.line_ending, LineEnding::Lf);
        assert_eq!(buf.rope.to_string(), "one\ntwo\n");
        assert!(!buf.dirty);
        Ok(())
    }

    #[test]
    fn crlf_file_is_detected_and_normalised_to_lf() -> Result<()> {
        let buf = open_bytes(b"one\r\ntwo\r\nthree")?;
        assert_eq!(buf.line_ending, LineEnding::Crlf);
        assert_eq!(buf.rope.to_string(), "one\ntwo\nthree");
        assert_eq!(buf.rope.len_lines(), 3);
        Ok(())
    }

    #[test]
    fn first_line_break_decides() -> Result<()> {
        assert_eq!(open_bytes(b"a\nb\r\nc")?.line_ending, LineEnding::Lf);
        assert_eq!(open_bytes(b"a\r\nb\nc")?.line_ending, LineEnding::Crlf);
        Ok(())
    }

    #[test]
    fn no_line_break_is_lf() -> Result<()> {
        assert_eq!(open_bytes(b"single line")?.line_ending, LineEnding::Lf);
        assert_eq!(open_bytes(b"")?.line_ending, LineEnding::Lf);
        Ok(())
    }

    #[test]
    fn missing_file_is_an_empty_buffer_with_that_path() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("missing.txt");
        let buf = Buffer::open(&path)?;
        assert_eq!(buf.rope.len_chars(), 0);
        assert_eq!(buf.path.as_deref(), Some(path.as_path()));
        Ok(())
    }

    #[test]
    fn non_utf8_file_is_refused() {
        let err = open_bytes(b"ok \xff\xfe bad").expect_err("invalid UTF-8 must not open");
        assert_eq!(err.to_string(), "not UTF-8");
    }

    /// Opens `bytes` from a file, types `typed` at the start, saves, and returns
    /// what landed on disk.
    fn round_trip(bytes: &[u8], typed: &str) -> Result<Vec<u8>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("file.txt");
        fs::write(&path, bytes)?;
        let mut buf = Buffer::open(&path)?;
        buf.insert(typed);
        assert_eq!(buf.dirty, !typed.is_empty());
        buf.save()?;
        assert!(!buf.dirty);
        Ok(fs::read(&path)?)
    }

    #[test]
    fn saving_a_crlf_file_writes_crlf() -> Result<()> {
        assert_eq!(round_trip(b"one\r\ntwo\r\n", "")?, b"one\r\ntwo\r\n");
        // Line breaks typed into a CRLF file are written as CRLF too.
        assert_eq!(round_trip(b"a\r\nb\r\n", "x\n")?, b"x\r\na\r\nb\r\n");
        Ok(())
    }

    #[test]
    fn saving_an_lf_file_writes_lf() -> Result<()> {
        assert_eq!(round_trip(b"one\ntwo\n", "x\n")?, b"x\none\ntwo\n");
        Ok(())
    }

    #[test]
    fn a_missing_final_newline_stays_missing() -> Result<()> {
        assert_eq!(round_trip(b"one\ntwo", "")?, b"one\ntwo");
        assert_eq!(round_trip(b"one\r\ntwo", "")?, b"one\r\ntwo");
        assert_eq!(round_trip(b"single", "x")?, b"xsingle");
        Ok(())
    }

    #[test]
    fn saving_a_missing_file_creates_it() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("new.txt");
        let mut buf = Buffer::open(&path)?;
        buf.insert("hi\n");
        buf.save()?;
        assert_eq!(fs::read(&path)?, b"hi\n");
        Ok(())
    }

    #[test]
    fn untitled_buffer_cannot_be_saved() {
        let mut buf = Buffer::empty();
        buf.insert("x");
        let err = buf.save().expect_err("no path to save to");
        assert_eq!(err.to_string(), "no file name");
        assert!(buf.dirty);
    }

    #[test]
    fn recover_loads_the_text_as_dirty_and_keeps_the_path() -> Result<()> {
        let mut buf = open_bytes(b"one\r\n")?;
        let path = buf.path.clone();
        buf.recover("two\r\nthree\r\n");
        assert_eq!(buf.rope.to_string(), "two\nthree\n");
        assert_eq!(buf.line_ending, LineEnding::Crlf);
        assert_eq!(buf.disk_text(), "two\r\nthree\r\n");
        assert!(buf.dirty);
        assert_eq!(buf.path, path);
        Ok(())
    }

    #[test]
    fn a_kept_caret_comes_back_clamped_to_the_text() {
        let mut buf = Buffer {
            rope: Rope::from_str("hello"),
            ..Buffer::empty()
        };
        buf.set_caret(Caret {
            cursor: 4,
            goal_col: Some(4),
            anchor: Some(1),
        });
        let kept = buf.caret();
        assert_eq!(buf.selection(), Some(1..4));
        buf.place_cursor(0);
        buf.rope = Rope::from_str("hi");
        buf.set_caret(kept);
        assert_eq!((buf.cursor, buf.anchor), (2, Some(1)));
        let copy = Buffer {
            rope: Rope::from_str("hello"),
            ..Buffer::empty()
        }
        .with_caret(kept);
        assert_eq!(copy.selection(), Some(1..4));
        assert_eq!(copy.cursor_line_col(), (0, 4));
    }

    #[test]
    fn untitled_buffer_is_named_untitled() {
        let buf = Buffer::empty();
        assert_eq!(buf.name(), "untitled");
        assert!(buf.path.is_none());
    }
}
