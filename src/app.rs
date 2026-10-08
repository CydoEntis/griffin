use std::borrow::Cow;
use std::collections::HashSet;
use std::io;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::Result;
use crossterm::event::{
    Event, EventStream, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use futures_util::StreamExt;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::Style;
use tokio::sync::mpsc;
#[cfg(windows)]
use tokio::time::timeout;

use crate::Tui;
use crate::backup::{self, Backups};
use crate::buffer::movement::Motion;
use crate::buffer::{Buffer, Caret};
use crate::clipboard::Clipboard;
use crate::config::{self, EditorConfig, RunEntry};
use crate::highlight::languages;
#[cfg(windows)]
use crate::keymap::burst_as_paste;
use crate::keymap::{Action, Input, Keymap, Scope};
use crate::lsp::{
    CompletionItem, Diagnostic, FormatRequest, HoverText, Location, Lsp, LspEvent, LspNews,
    Severity, char_index, diagnostic_at, diagnostic_jump,
};
use crate::search::{self, Hit, Query};
use crate::theme::Theme;
use crate::ui::completion::{self, Completion};
use crate::ui::confirm::{Answer, Choice, Confirm, Reply};
use crate::ui::dirpicker::{Browsed, DirPicker};
use crate::ui::find::{FindBar, Step};
use crate::ui::hover::render_hover;
use crate::ui::picker::{self, Picked, Picker};
use crate::ui::prompt::{Outcome, PromptBar};
use crate::ui::run::{RunStatus, RunView, render_run_panel};
use crate::ui::search::{ProjectSearch, Searched};
use crate::ui::status::{Status, Tone, render_status};
use crate::ui::tabs::{HEADER_HEIGHT, TabLabel, render_tabs, tab_at};
use crate::ui::tree::{
    TREE_WIDTH, TreeMarks, nodes_area, project_label, render_divider, render_tree,
};
use crate::view::{Marks, View, cursor_cell, render_buffer};
use crate::workspace::ops::{self, Trash};
use crate::workspace::walk::{list_files, relative_name};
use crate::workspace::{Launch, Tree};

/// Everything the event loop reacts to. Background work (LSP, run output, timers)
/// adds variants here instead of touching `App` (ADR-0001).
#[derive(Debug)]
pub enum AppEvent {
    Input(Event),
    /// Edits stopped long enough ago that a crash backup is due.
    BackupDue,
    /// How a backup write went.
    BackupWritten(io::Result<()>),
    /// File walk number `walk` for the go-to-file picker finished: every file in
    /// the project, relative to the root it walked.
    FilesListed {
        walk: u64,
        files: Vec<String>,
    },
    /// A batch of hits from project search number `search`.
    SearchHits {
        search: u64,
        hits: Vec<Hit>,
    },
    /// Project search number `search` has looked at every file.
    SearchDone {
        search: u64,
    },
    /// One line of stdout or stderr from run number `run`, colour codes and all.
    RunOutput {
        run: u64,
        line: String,
    },
    /// Run number `run` exited, with its code when it had one.
    RunExited {
        run: u64,
        code: Option<i32>,
    },
    /// A language server sent a message or exited.
    Lsp(LspEvent),
    /// Format on save number `format` has waited `FORMAT_TIMEOUT` for its server.
    FormatTimedOut {
        format: u64,
    },
}

/// A question that takes over the keyboard until it's answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prompt {
    /// Ctrl+Q with unsaved changes, asked once per dirty tab, about the active one.
    UnsavedQuit,
    /// Closing the active tab with unsaved changes.
    UnsavedClose,
    /// The file opened with a newer crash backup beside it.
    Recover,
    /// Moving the tree's selected entry to the trash.
    Trash,
    /// Replacing project search's hits in every listed file.
    ProjectReplace,
}

/// A project replace waiting for its prompt's answer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingReplace {
    query: Query,
    with: String,
    /// The files with matches, relative to the root as the list shows them.
    files: Vec<String>,
    /// How many matches those files hold, for the prompt.
    matches: usize,
}

/// What happens once a save as succeeds, for the saves a question started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AfterSave {
    Nothing,
    /// The close prompt's save: close the tab.
    Close,
    /// The quit prompt's save: go on to the next dirty tab, or quit.
    Quit,
}

/// What the prompt bar's answer will be used for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum BarOp {
    /// A new file in this folder.
    NewFile(PathBuf),
    /// A new folder in this folder.
    NewFolder(PathBuf),
    /// A new name for this file or folder.
    Rename(PathBuf),
    /// A path, relative to the project root, to save the active buffer to.
    SaveAs(AfterSave),
}

impl BarOp {
    /// What Enter and Esc do, shown at the right of the bar (SPEC_V1_LAYOUT §8).
    fn hint(&self) -> &'static str {
        match self {
            BarOp::NewFile(_) | BarOp::NewFolder(_) => "⏎ create   esc cancel",
            BarOp::Rename(_) => "⏎ rename   esc cancel",
            BarOp::SaveAs(_) => "⏎ save   esc cancel",
        }
    }
}

/// The prompt bar while it asks for a name.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NamePrompt {
    op: BarOp,
    bar: PromptBar,
}

/// What the picker's choice will be used for.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
enum PickerFor {
    /// Go to file: the choice is a path to open.
    #[default]
    File,
    /// F5 with several `[[run]]` entries: the choice names the one to run.
    Run(Vec<RunEntry>),
}

/// Which pane keys go to.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Focus {
    #[default]
    Editor,
    Tree,
}

const UNSAVED: &[Choice] = &[
    Choice {
        key: 's',
        label: "Save",
    },
    Choice {
        key: 'd',
        label: "Discard",
    },
    Choice {
        key: 'c',
        label: "Cancel",
    },
];

const RECOVER: &[Choice] = &[
    Choice {
        key: 'r',
        label: "Recover",
    },
    Choice {
        key: 'd',
        label: "Discard",
    },
];

const REPLACE: &[Choice] = &[
    Choice {
        key: 'r',
        label: "Replace",
    },
    Choice {
        key: 'c',
        label: "Cancel",
    },
];

const TRASH: &[Choice] = &[
    Choice {
        key: 'y',
        label: "Move to trash",
    },
    Choice {
        key: 'n',
        label: "Cancel",
    },
];

/// A second left click on the same cell within this long selects a word.
const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Lines one wheel notch scrolls.
const WHEEL_LINES: isize = 3;

/// Where and when the last left button press landed, for double-click detection.
#[derive(Debug, Clone, Copy)]
struct Click {
    at: Instant,
    col: u16,
    row: u16,
}

/// One open buffer, shared by every split showing it.
#[derive(Debug)]
struct Doc {
    /// Unique for the run; names an untitled buffer's crash backup.
    id: u64,
    buffer: Buffer,
    /// Edited since its last backup was started.
    backup_due: bool,
    /// What its language server last published for it, sorted by start.
    diagnostics: Vec<Diagnostic>,
}

impl Doc {
    fn label(&self) -> TabLabel {
        TabLabel {
            name: self
                .buffer
                .path
                .as_deref()
                .map_or_else(|| "untitled".to_string(), file_name),
            dirty: self.buffer.dirty,
        }
    }

    /// An untitled buffer nobody has typed in, which opening a file may replace.
    fn is_pristine(&self) -> bool {
        self.buffer.path.is_none() && !self.buffer.dirty && self.buffer.rope.len_chars() == 0
    }
}

/// A tab in one split: the buffer it shows and that split's own view of it.
#[derive(Debug, Clone, Copy)]
struct Tab {
    doc: u64,
    view: View,
    /// This tab's cursor and selection. The buffer holds the live copy while this
    /// is the focused split's active tab; otherwise this one counts, so two splits
    /// on one buffer each keep their place.
    caret: Caret,
}

/// One editor split: its tabs and which one shows.
#[derive(Debug, Default)]
struct Split {
    /// Never empty once `Tabs` is built; a buffer appears at most once.
    tabs: Vec<Tab>,
    /// Index into `tabs`; always in bounds.
    active: usize,
}

impl Split {
    fn active(&self) -> &Tab {
        &self.tabs[self.active]
    }

    fn position(&self, doc: u64) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.doc == doc)
    }

    /// Removes tab `index`; the one to its right (or, at the end, its left) takes
    /// its place as active if it was. May leave the split empty.
    fn remove(&mut self, index: usize) {
        self.tabs.remove(index);
        if index < self.active || (self.active >= self.tabs.len() && self.active > 0) {
            self.active -= 1;
        }
    }
}

/// The open buffers and the one or two splits showing them. Every buffer has a
/// tab in at least one split; closing its last tab closes it. No split is empty:
/// closing a split's last tab leaves an untitled one.
#[derive(Debug)]
struct Tabs {
    /// In the order they were opened, which is the order quitting asks in.
    docs: Vec<Doc>,
    /// The left split, then the right one while split; never empty.
    splits: Vec<Split>,
    /// The split keys and tab actions go to; index into `splits`.
    focused: usize,
    next_id: u64,
}

impl Default for Tabs {
    fn default() -> Self {
        Self::new(Buffer::empty())
    }
}

impl Tabs {
    fn new(buffer: Buffer) -> Self {
        let mut tabs = Tabs {
            docs: Vec::new(),
            splits: vec![Split::default()],
            focused: 0,
            next_id: 0,
        };
        tabs.push(buffer);
        tabs
    }

    fn add_doc(&mut self, buffer: Buffer) -> u64 {
        let id = self.next_id;
        self.docs.push(Doc {
            id,
            buffer,
            backup_due: false,
            diagnostics: Vec::new(),
        });
        self.next_id += 1;
        id
    }

    /// Opens `buffer` in a new tab at the end of the focused split and makes it
    /// active.
    fn push(&mut self, buffer: Buffer) {
        let id = self.add_doc(buffer);
        self.show(id);
    }

    /// Makes `doc` the focused split's active tab, giving it a tab there first if
    /// that split doesn't show it yet.
    fn show(&mut self, doc: u64) {
        let index = match self.split().position(doc) {
            Some(index) => index,
            None => {
                self.stash();
                let caret = self.doc(doc).buffer.caret();
                let split = &mut self.splits[self.focused];
                split.tabs.push(Tab {
                    doc,
                    view: View::default(),
                    caret,
                });
                split.tabs.len() - 1
            }
        };
        self.activate(self.focused, index);
    }

    /// Focuses tab `index` of split `split`, handing the live cursor over.
    fn activate(&mut self, split: usize, index: usize) {
        self.stash();
        self.focused = split;
        self.splits[split].active = index;
        self.restore();
    }

    /// Copies the live cursor into the focused tab, before focus leaves it.
    fn stash(&mut self) {
        let split = &mut self.splits[self.focused];
        if let Some(tab) = split.tabs.get_mut(split.active)
            && let Some(doc) = self.docs.iter().find(|doc| doc.id == tab.doc)
        {
            tab.caret = doc.buffer.caret();
        }
    }

    /// Hands the focused tab's cursor to its buffer, as focus arrives.
    fn restore(&mut self) {
        let split = &self.splits[self.focused];
        if let Some(tab) = split.tabs.get(split.active)
            && let Some(doc) = self.docs.iter_mut().find(|doc| doc.id == tab.doc)
        {
            doc.buffer.set_caret(tab.caret);
        }
    }

    fn doc_mut(&mut self, id: u64) -> &mut Doc {
        // As in `doc`: callers only pass ids of open buffers.
        self.docs
            .iter_mut()
            .find(|doc| doc.id == id)
            .expect("a tab's buffer is open")
    }

    fn doc(&self, id: u64) -> &Doc {
        // Every tab's buffer stays open until its last tab closes, and callers
        // only pass ids taken from a tab.
        self.docs
            .iter()
            .find(|doc| doc.id == id)
            .expect("a tab's buffer is open")
    }

    fn split(&self) -> &Split {
        &self.splits[self.focused]
    }

    fn active_tab(&self) -> &Tab {
        self.split().active()
    }

    fn active_tab_mut(&mut self) -> &mut Tab {
        let split = &mut self.splits[self.focused];
        &mut split.tabs[split.active]
    }

    /// The buffer keys edit: the focused split's active tab's.
    fn active(&self) -> &Doc {
        self.doc(self.active_tab().doc)
    }

    fn active_mut(&mut self) -> &mut Doc {
        let id = self.active_tab().doc;
        // As in `doc`: the active tab's buffer is open.
        self.docs
            .iter_mut()
            .find(|doc| doc.id == id)
            .expect("the active tab's buffer is open")
    }

    /// Whether a split other than the focused one shows `doc`.
    fn shown_elsewhere(&self, doc: u64) -> bool {
        self.splits
            .iter()
            .enumerate()
            .any(|(index, split)| index != self.focused && split.position(doc).is_some())
    }

    /// The open buffer holding `path`, however the path was spelled.
    fn find(&self, path: &Path) -> Option<u64> {
        let wanted = absolute(path);
        self.docs
            .iter()
            .find(|doc| doc.buffer.path.as_deref().map(absolute).as_ref() == Some(&wanted))
            .map(|doc| doc.id)
    }

    /// Focuses a tab showing `doc`, preferring the focused split.
    fn reveal(&mut self, doc: u64) {
        let found = std::iter::once(self.focused)
            .chain(0..self.splits.len())
            .find_map(|split| Some((split, self.splits[split].position(doc)?)));
        if let Some((split, index)) = found {
            self.activate(split, index);
        }
    }

    /// Closes the focused split's active tab, and its buffer when no other split
    /// shows it, returning the buffer then.
    fn close_active(&mut self) -> Option<Doc> {
        let split = &mut self.splits[self.focused];
        let doc = split.active().doc;
        split.remove(split.active);
        let closed = self.close_unshown(doc);
        self.fill_empty();
        self.restore();
        closed
    }

    /// Closes every buffer `close` picks, and all their tabs.
    fn close_where(&mut self, close: impl Fn(&Doc) -> bool) -> Vec<Doc> {
        self.stash();
        let (closed, kept): (Vec<Doc>, Vec<Doc>) =
            std::mem::take(&mut self.docs).into_iter().partition(close);
        self.docs = kept;
        for split in &mut self.splits {
            let mut index = 0;
            while index < split.tabs.len() {
                if closed.iter().any(|doc| doc.id == split.tabs[index].doc) {
                    split.remove(index);
                } else {
                    index += 1;
                }
            }
        }
        self.fill_empty();
        self.restore();
        closed
    }

    /// Takes `doc` out of `docs` if no tab shows it any more.
    fn close_unshown(&mut self, doc: u64) -> Option<Doc> {
        if self
            .splits
            .iter()
            .any(|split| split.position(doc).is_some())
        {
            return None;
        }
        let index = self.docs.iter().position(|open| open.id == doc)?;
        Some(self.docs.remove(index))
    }

    /// Gives every split left without tabs a fresh untitled one.
    fn fill_empty(&mut self) {
        for index in 0..self.splits.len() {
            if self.splits[index].tabs.is_empty() {
                let doc = self.add_doc(Buffer::empty());
                self.splits[index] = Split {
                    tabs: vec![Tab {
                        doc,
                        view: View::default(),
                        caret: Caret::default(),
                    }],
                    active: 0,
                };
            }
        }
    }

    /// Opens a right split showing the focused tab, and focuses it; or closes the
    /// right split. Tabs only the right split had move to the left one, so closing
    /// it never closes a buffer.
    fn toggle_split(&mut self) {
        self.stash();
        if self.splits.len() > 1 {
            let right = self.splits.remove(1);
            let left = &mut self.splits[0];
            for tab in right.tabs {
                if left.position(tab.doc).is_none() {
                    left.tabs.push(tab);
                }
            }
            self.focused = 0;
        } else {
            let tab = *self.active_tab();
            self.splits.push(Split {
                tabs: vec![tab],
                active: 0,
            });
            self.focused = 1;
        }
        self.restore();
    }

    /// What split `split`'s pills show.
    fn labels(&self, split: usize) -> Vec<TabLabel> {
        self.splits[split]
            .tabs
            .iter()
            .map(|tab| self.doc(tab.doc).label())
            .collect()
    }

    /// The buffer split `split`'s active tab shows, with that tab's cursor: the
    /// buffer itself in the focused split, a copy with the kept cursor elsewhere.
    fn shown(&self, split: usize) -> Shown<'_> {
        let tab = self.splits[split].active();
        let buffer = &self.doc(tab.doc).buffer;
        if split == self.focused {
            Shown::Live(buffer)
        } else {
            Shown::Copy(Box::new(buffer.with_caret(tab.caret)))
        }
    }

    /// Scrolls each split's active view to its cursor; `areas` are the splits'
    /// editor panes, in order.
    fn follow(&mut self, areas: &[Rect], tab_width: usize) {
        for (index, &area) in areas.iter().enumerate().take(self.splits.len()) {
            let shown = self.shown(index);
            let view = shown.follow_from(self.splits[index].active().view, area, tab_width);
            let split = &mut self.splits[index];
            split.tabs[split.active].view = view;
        }
    }

    /// Moves the kept cursors of `doc`'s other tabs that sit past `at` by `delta`
    /// chars, after text changed there through the live one, so a split on the
    /// same buffer stays on the text it was on.
    fn shift_others(&mut self, doc: u64, at: usize, delta: isize) {
        let shift = |pos: usize| {
            if pos > at {
                pos.saturating_add_signed(delta).max(at)
            } else {
                pos
            }
        };
        for (index, split) in self.splits.iter_mut().enumerate() {
            for (tab_index, tab) in split.tabs.iter_mut().enumerate() {
                let live = index == self.focused && tab_index == split.active;
                if tab.doc == doc && !live {
                    tab.caret.cursor = shift(tab.caret.cursor);
                    tab.caret.anchor = tab.caret.anchor.map(shift);
                }
            }
        }
    }

    /// Scrolls split `split`'s active view by `lines` without moving any cursor.
    fn scroll(&mut self, split: usize, area: Rect, lines: isize) {
        let split = &mut self.splits[split];
        let tab = &mut split.tabs[split.active];
        if let Some(doc) = self.docs.iter().find(|doc| doc.id == tab.doc) {
            tab.view.scroll_by(&doc.buffer, area, lines);
        }
    }
}

/// A split's buffer as `Tabs::shown` hands it out for drawing and scrolling.
enum Shown<'a> {
    Live(&'a Buffer),
    // Boxed: a buffer is far bigger than the reference beside it.
    Copy(Box<Buffer>),
}

impl Shown<'_> {
    fn buffer(&self) -> &Buffer {
        match self {
            Shown::Live(buffer) => buffer,
            Shown::Copy(buffer) => buffer,
        }
    }

    /// `view` scrolled to this buffer's cursor in `area`.
    fn follow_from(&self, mut view: View, area: Rect, tab_width: usize) -> View {
        view.follow(self.buffer(), area, tab_width);
        view
    }
}

/// How many places Alt+Left can go back through; older ones are forgotten.
const JUMP_LIST_LEN: usize = 50;

/// How long a save waits for its language server's formatting before saving the
/// text as it is.
const FORMAT_TIMEOUT: Duration = Duration::from_secs(2);

/// A save waiting on its formatting reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingFormat {
    /// Numbers the request, so a timer for an earlier one is ignored.
    format: u64,
    doc: u64,
    /// The buffer's revision when asked; edits for older text can't apply.
    revision: u64,
}

/// Where the cursor was before a go to definition, for Alt+Left to return to.
#[derive(Debug, Clone)]
struct Jump {
    doc: u64,
    /// So the place can still be reached after its tab was closed.
    path: Option<PathBuf>,
    cursor: usize,
}

/// `path` made absolute, so two spellings of one file compare equal.
fn absolute(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// All editor state, owned by the main task.
#[derive(Debug, Default)]
pub struct App {
    keymap: Keymap,
    editor: EditorConfig,
    theme: Theme,
    tabs: Tabs,
    /// The terminal's size as of the last draw; movement needs the editor pane's
    /// height for paging and both dimensions for scrolling.
    screen: Rect,
    /// Shown in the status line, e.g. why the config fell back to defaults.
    message: Option<String>,
    /// How `message` went; picks its glyph and colour.
    message_tone: Tone,
    /// While set, every key goes to the prompt instead of the editor.
    prompt: Option<Prompt>,
    /// The prompt's focused button, the one Enter presses.
    prompt_button: usize,
    /// The project's file tree, read from disk as folders expand.
    tree: Tree,
    /// The tree's root as its brand row names it; worked out once, as it reads
    /// the home folder from the environment.
    project: String,
    tree_visible: bool,
    focus: Focus,
    /// The prompt bar, while it's asking for a name; it takes every key.
    name_prompt: Option<NamePrompt>,
    /// The picker, while it's open; it takes every key.
    picker: Option<Picker>,
    picker_for: PickerFor,
    /// The folder browser, while it's open; it takes every key.
    folders: Option<DirPicker>,
    /// The latest run, whose output the run panel shows.
    run: Option<RunView>,
    /// The latest run's entry, for Ctrl+F5 to start again.
    run_entry: Option<RunEntry>,
    /// The latest run's processes; dropping it kills them.
    run_tree: Option<crate::run::ProcessTree>,
    /// How many runs have started, numbering them so output from an earlier one
    /// is dropped.
    runs: u64,
    run_panel_visible: bool,
    /// The find bar, while it's open; it takes every key, so the active buffer
    /// can't change under its matches.
    find: Option<FindBar>,
    /// The project search panel, while it's open; it takes every key.
    project_search: Option<ProjectSearch>,
    /// How many Ctrl+P file walks have started, numbering them so a list from
    /// one that was overtaken (perhaps of the old project) is dropped.
    walks: u64,
    /// How many project searches have started, numbering them so hits from one
    /// that was replaced are dropped.
    searches: u64,
    /// Set to stop the running project search's walk.
    search_cancel: Option<Arc<AtomicBool>>,
    /// The project replace the replace prompt will do once answered.
    pending_replace: Option<PendingReplace>,
    /// The entry the trash prompt will move once answered.
    pending_trash: Option<PathBuf>,
    /// Tabs whose changes the user chose to discard while quitting, so the quit
    /// prompt moves on to the next one. Emptied when the quit is cancelled.
    quit_discarded: Vec<u64>,
    trash: Box<dyn Trash>,
    clipboard: Box<dyn Clipboard>,
    /// The previous left press, until a double-click uses it up.
    last_click: Option<Click>,
    /// Char index a left press landed on while the button is held, so a drag
    /// selects from there.
    drag_from: Option<usize>,
    backups: Backups,
    /// The backup text the recover prompt is offering.
    recovery: Option<String>,
    /// When that backup was written, for the recover prompt to say.
    recovery_from: Option<SystemTime>,
    /// A backup write failed and the status line said so; cleared by the next
    /// success, so a run of failures shows one message instead of one per edit.
    backup_failed: bool,
    /// Tells the backup timer about each edit. `None` outside the event loop (unit
    /// tests), where `AppEvent::BackupDue` is sent by hand.
    edits: Option<mpsc::UnboundedSender<()>>,
    /// The app channel, for background backup writes to report back on. `None`
    /// outside the event loop, where writes run inline.
    events: Option<mpsc::UnboundedSender<AppEvent>>,
    /// Language servers and the buffers they follow.
    lsp: Lsp,
    /// Places go to definition left, newest last, at most `JUMP_LIST_LEN`.
    jumps: Vec<Jump>,
    /// The hover popup's text, while it's showing. Any key or click closes it.
    hover: Option<HoverText>,
    /// The hover awaiting its reply, as (buffer id, cursor) when asked; an answer
    /// for a cursor that has since moved is dropped.
    hover_for: Option<(u64, usize)>,
    /// The completion popup, while it's open. It takes the keys it uses before
    /// the editor does; any other key or a click closes it.
    completion: Option<Completion>,
    /// The completion awaiting its reply, as (buffer id, word start, cursor)
    /// when asked; an answer arriving once the cursor has left the word is
    /// dropped.
    completion_for: Option<(u64, usize, usize)>,
    /// The save waiting on format on save, at most `FORMAT_TIMEOUT`.
    formatting: Option<PendingFormat>,
    /// Format requests made, for numbering the next.
    formats: u64,
    should_quit: bool,
}

impl App {
    /// `path` is what the command line named, if anything: a folder opens as the
    /// project with the tree showing, a file opens in the editor. A file that can't
    /// be opened leaves an untitled buffer and says why in the status line.
    pub fn new(
        keymap: Keymap,
        editor: EditorConfig,
        path: Option<PathBuf>,
        message: Option<String>,
    ) -> Self {
        let mut messages: Vec<String> = message.into_iter().collect();
        let launch = Launch::from_arg(path.as_deref());
        let buffer = match launch.file {
            Some(path) => Buffer::open(&path).unwrap_or_else(|err| {
                messages.push(format!("cannot open {}: {err}", path.display()));
                Buffer::empty()
            }),
            None => Buffer::empty(),
        };
        let message = (!messages.is_empty()).then(|| messages.join(" · "));
        Self {
            keymap,
            editor,
            tabs: Tabs::new(buffer),
            message,
            // Startup only has something to say when a config or file failed.
            message_tone: Tone::Err,
            tree: Tree::new(&launch.root),
            project: project_label(&launch.root, std::env::home_dir().as_deref()),
            tree_visible: launch.show_tree,
            // With only a folder open there's nothing to edit yet.
            focus: if launch.show_tree {
                Focus::Tree
            } else {
                Focus::Editor
            },
            ..Self::default()
        }
    }

    /// Draws with `theme` instead of the default.
    pub fn with_theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Starts language servers from `[lsp.<lang>]` as files of each language open.
    pub fn with_lsp(mut self, lsp: Lsp) -> Self {
        self.lsp = lsp;
        self
    }

    /// Turns crash backups on, and offers to recover the opened file's backup if
    /// it has a newer one.
    pub fn with_backups(mut self, backups: Backups) -> Self {
        self.backups = backups;
        if let Some(path) = self.buffer().path.clone()
            && let Some(text) = self.backups.recoverable(&path)
        {
            self.recovery = Some(text);
            self.recovery_from = self.backups.written(&path);
            self.ask(Prompt::Recover);
        }
        self
    }

    fn buffer(&self) -> &Buffer {
        &self.tabs.active().buffer
    }

    fn buffer_mut(&mut self) -> &mut Buffer {
        &mut self.tabs.active_mut().buffer
    }

    #[cfg(test)]
    fn view(&self) -> &View {
        &self.tabs.active_tab().view
    }

    pub async fn run(&mut self, terminal: &mut Tui) -> Result<()> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        self.edits = Some(backup::spawn_timer(tx.clone()));
        self.events = Some(tx.clone());
        self.lsp.connect(tx.clone());
        let input = tokio::spawn(read_input(tx));

        let result = self.event_loop(terminal, &mut rx).await;
        input.abort();
        self.lsp.finish().await;
        result
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Tui,
        rx: &mut mpsc::UnboundedReceiver<AppEvent>,
    ) -> Result<()> {
        self.sync_lsp();
        self.draw(terminal)?;
        while !self.should_quit {
            let Some(event) = rx.recv().await else {
                // The input task ended (stdin closed or errored); nothing more can
                // arrive, so stop rather than hang.
                break;
            };
            if let AppEvent::Input(Event::Resize(width, height)) = event {
                self.screen = Rect::new(0, 0, width, height);
            }
            self.handle_event(event);
            self.sync_lsp();
            self.draw(terminal)?;
        }
        Ok(())
    }

    /// Draws one frame as a synchronized update, so a terminal shows it whole: a
    /// themed frame repaints every cell, and drawn piecemeal it tears.
    fn draw(&mut self, terminal: &mut Tui) -> Result<()> {
        self.sync_highlights();
        execute!(terminal.backend_mut(), BeginSynchronizedUpdate)?;
        self.screen = terminal.draw(|frame| self.render(frame))?.area;
        execute!(terminal.backend_mut(), EndSynchronizedUpdate)?;
        Ok(())
    }

    /// Tells language servers about whatever the last event did to the open
    /// buffers: files opened, edited, saved or closed. Never waits on a server.
    fn sync_lsp(&mut self) {
        let root = absolute(self.tree.root());
        let docs: Vec<(u64, &Buffer)> = self
            .tabs
            .docs
            .iter()
            .map(|doc| (doc.id, &doc.buffer))
            .collect();
        if let Some(message) = self.lsp.sync(&root, &docs).pop() {
            self.say(Tone::Warn, message);
        }
    }

    /// Brings every buffer's syntax tree up to date. Rendering only reads the
    /// trees, so this runs before each frame.
    fn sync_highlights(&mut self) {
        for doc in &mut self.tabs.docs {
            doc.buffer.sync_highlight();
        }
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Input(Event::Key(key)) => self.handle_key(key),
            AppEvent::Input(Event::Paste(text)) if self.prompt.is_none() => {
                self.hover = None;
                self.completion = None;
                if let Some(browser) = &mut self.folders {
                    if let Some(step) = browser.paste(&text) {
                        self.folders_step(step);
                    }
                    return;
                }
                if let Some(picker) = &mut self.picker {
                    if let Some(picked) = picker.paste(&text) {
                        self.finish_picker(picked);
                    }
                    return;
                }
                if let Some(panel) = &mut self.project_search {
                    if let Some(step) = panel.paste(&text) {
                        self.project_search_step(step);
                    }
                    return;
                }
                if let Some(name_prompt) = &mut self.name_prompt {
                    if let Some(outcome) = name_prompt.bar.paste(&text) {
                        self.finish_name_prompt(outcome);
                    }
                    return;
                }
                if let Some(find) = &mut self.find {
                    let step = find.paste(&text, &self.tabs.active().buffer.rope);
                    self.find_step(step);
                    return;
                }
                // Bracketed paste bypasses the keymap: it is text, not a key.
                // Windows Terminal's own Ctrl+V paste arrives this way too.
                self.edit(|buffer| buffer.paste(&text));
            }
            // A click on a confirm card's button presses it; outside the card
            // it's Esc (SPEC_V1_LAYOUT §7).
            AppEvent::Input(Event::Mouse(mouse)) if self.prompt.is_some() => {
                if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                    && let Some(prompt) = self.prompt
                    && let Some(step) = self
                        .confirm(prompt)
                        .click(self.screen, Position::new(mouse.column, mouse.row))
                {
                    self.prompt_step(prompt, step);
                }
            }
            // A click outside a dialog is Esc (SPEC_V1_LAYOUT §7).
            AppEvent::Input(Event::Mouse(mouse)) if self.project_search.is_some() => {
                let card = ProjectSearch::card(self.screen);
                if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                    && !card.contains(Position::new(mouse.column, mouse.row))
                {
                    self.close_project_search();
                }
            }
            // The folder browser closes on a click outside it, as the cast does.
            AppEvent::Input(Event::Mouse(mouse)) if self.folders.is_some() => {
                let outside = self.folders.as_ref().is_some_and(|browser| {
                    !browser
                        .card(self.screen)
                        .contains(Position::new(mouse.column, mouse.row))
                });
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) && outside {
                    self.folders = None;
                }
            }
            // The cast palette closes on a click outside it, as a dialog does.
            AppEvent::Input(Event::Mouse(mouse))
                if self.picker.is_some()
                    && self.picker_for == PickerFor::File
                    && self.prompt.is_none() =>
            {
                let outside = self.picker.as_ref().is_some_and(|picker| {
                    !picker
                        .card(self.screen)
                        .contains(Position::new(mouse.column, mouse.row))
                });
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) && outside {
                    self.finish_picker(Picked::Close);
                }
            }
            // The mouse bypasses the keymap too: only keys are remappable.
            AppEvent::Input(Event::Mouse(mouse)) if !self.modal_open() => {
                if mouse.kind != MouseEventKind::Moved {
                    self.hover = None;
                    self.completion = None;
                }
                self.handle_mouse(mouse, Instant::now());
            }
            // A smaller pane can leave the cursor outside it.
            AppEvent::Input(Event::Resize(..)) => self.follow_cursor(),
            AppEvent::Input(_) => {}
            AppEvent::BackupDue => self.back_up(),
            AppEvent::BackupWritten(result) => self.backup_written(result),
            // A list for a picker that has since closed is dropped; the next
            // Ctrl+P walks again. So is one from an older walk: it may be of a
            // folder that is no longer the project, and its paths would be
            // joined onto the new root.
            AppEvent::FilesListed { walk, files } => {
                if let Some(picker) = &mut self.picker
                    && self.picker_for == PickerFor::File
                    && walk == self.walks
                {
                    picker.set_files(files);
                }
            }
            // The panel drops hits from a search it has replaced; with the panel
            // closed there's nothing to show them in.
            AppEvent::SearchHits { search, hits } => {
                if let Some(panel) = &mut self.project_search {
                    panel.add(search, hits);
                }
            }
            AppEvent::SearchDone { search } => {
                if let Some(panel) = &mut self.project_search {
                    panel.finish(search);
                }
            }
            AppEvent::RunOutput { run, line } => {
                if let Some(view) = &mut self.run
                    && view.id == run
                {
                    view.push(&line);
                }
            }
            // A stopped run keeps saying so rather than show the kill's exit code.
            AppEvent::RunExited { run, code } => {
                if let Some(view) = &mut self.run
                    && view.id == run
                    && view.status == RunStatus::Running
                {
                    view.status = RunStatus::Exited(code);
                }
            }
            AppEvent::FormatTimedOut { format } => {
                if let Some(pending) = self.formatting.take_if(|p| p.format == format) {
                    self.lsp.cancel_format();
                    let seconds = FORMAT_TIMEOUT.as_secs();
                    self.save_unformatted(pending.doc, &format!("no answer in {seconds} s"));
                }
            }
            AppEvent::Lsp(event) => {
                for news in self.lsp.handle(event) {
                    match news {
                        LspNews::Message(message) => self.say(Tone::Warn, message),
                        // A publish is the whole set for the file, so it replaces.
                        LspNews::Diagnostics { doc, diagnostics } => {
                            if let Some(open) = self.tabs.docs.iter_mut().find(|d| d.id == doc) {
                                open.diagnostics = diagnostics;
                            }
                        }
                        // Something else took the keys since F12; moving the
                        // buffer under it would surprise.
                        LspNews::Definition(_) if self.modal_open() => {}
                        LspNews::Definition(Some(location)) => self.go_to(&location),
                        LspNews::Definition(None) => {
                            self.say(Tone::Warn, "No definition found");
                        }
                        LspNews::Hover(text) => {
                            let asked = self.hover_for.take();
                            let here = (self.tabs.active().id, self.buffer().cursor);
                            if asked == Some(here) && !self.modal_open() {
                                self.hover = text;
                            }
                        }
                        LspNews::Completion { doc, items } => self.show_completion(doc, items),
                        LspNews::Formatted { doc, edits } => self.formatted(doc, edits),
                    }
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Tree letters are only actions when nothing else is reading keys as text.
        let scope = if self.folders.is_some() && self.prompt.is_none() {
            Scope::Folders
        } else if self.project_search.is_some() && self.prompt.is_none() {
            Scope::Search
        } else if self.find.is_some() && self.prompt.is_none() {
            Scope::Find
        } else if self.focus == Focus::Tree
            && self.prompt.is_none()
            && self.name_prompt.is_none()
            && self.picker.is_none()
        {
            Scope::Tree
        } else {
            Scope::Global
        };
        let input = self.keymap.resolve_in(&key, scope);
        // A release is no key: on Windows every press is followed by one, and a
        // hover reply landing between Alt+K's press and release would otherwise
        // close the popup before it was ever drawn.
        if key.kind == KeyEventKind::Release {
            return;
        }
        // A message is transient: it holds the path's place until the next key,
        // and whatever that key does may say something new.
        self.message = None;
        // Any key closes the hover popup; Esc does nothing else, the rest still
        // do what they do.
        if self.hover.take().is_some() && input == Input::Action(Action::Cancel) {
            return;
        }
        if let Some(prompt) = self.prompt {
            if let Some(step) = self.confirm(prompt).handle(input, self.prompt_button) {
                self.prompt_step(prompt, step);
            }
            return;
        }
        if let Some(browser) = &mut self.folders {
            if let Some(step) = browser.handle(input) {
                self.folders_step(step);
            }
            return;
        }
        if let Some(picker) = &mut self.picker {
            if let Some(picked) = picker.handle(input) {
                self.finish_picker(picked);
            }
            return;
        }
        if let Some(panel) = &mut self.project_search {
            if let Some(step) = panel.handle(input) {
                self.project_search_step(step);
            }
            return;
        }
        if let Some(name_prompt) = &mut self.name_prompt {
            if let Some(outcome) = name_prompt.bar.handle(input) {
                self.finish_name_prompt(outcome);
            }
            return;
        }
        if let Some(find) = &mut self.find {
            let step = find.handle(input, &self.tabs.active().buffer.rope);
            self.find_step(step);
            return;
        }
        // The completion popup sits over the editor, so Enter, Tab and the arrows
        // reach it first.
        if self.completion.is_some() && self.complete_key(input) {
            return;
        }
        match input {
            Input::Action(action) if self.focus == Focus::Tree => self.handle_tree_action(action),
            Input::Action(action) => self.handle_action(action),
            // Typing in the tree does nothing until type-to-find exists.
            Input::Text(_) if self.focus == Focus::Tree => {}
            Input::Text(ch) => {
                self.edit(|buffer| buffer.type_text(ch.encode_utf8(&mut [0; 4])));
                if self.lsp.is_trigger(self.tabs.active().id, ch) {
                    self.request_completion();
                }
            }
            Input::Ignored => {}
        }
    }

    /// A key while the completion popup is open. Returns whether the popup used
    /// it; otherwise the popup has closed and the key goes on to the editor.
    fn complete_key(&mut self, input: Input) -> bool {
        let Some(open) = &self.completion else {
            return false;
        };
        let buffer = self.buffer();
        let typed = completion::typed_word(&buffer.rope, open.start, buffer.cursor);
        let Some(typed) = typed.filter(|_| open.doc == self.tabs.active().id) else {
            self.completion = None;
            return false;
        };
        let has_items = !open.shown(&typed).is_empty();
        match input {
            // More of the word: it stays open and filters on it.
            Input::Text(ch) if completion::is_word_char(ch) => {
                self.edit(|buffer| buffer.type_text(ch.encode_utf8(&mut [0; 4])));
                self.refilter();
                true
            }
            Input::Action(Action::Backspace) => {
                self.buffer_mut().seal_undo_group();
                self.edit(Buffer::backspace);
                self.refilter();
                true
            }
            Input::Action(Action::Move(motion @ (Motion::Up | Motion::Down))) if has_items => {
                if let Some(open) = &mut self.completion {
                    open.step(&typed, motion == Motion::Down);
                }
                true
            }
            Input::Action(Action::Newline | Action::Tab) if has_items => {
                self.accept_completion(&typed);
                true
            }
            Input::Action(Action::Cancel) => {
                self.completion = None;
                true
            }
            // Key releases and unbound keys: nothing happened.
            Input::Ignored => true,
            _ => {
                self.completion = None;
                false
            }
        }
    }

    /// After the word under an open popup changed: back to the best match, or
    /// closed once the cursor has left the word.
    fn refilter(&mut self) {
        let buffer = self.buffer();
        let on_word = self.completion.as_ref().is_some_and(|c| {
            completion::typed_word(&buffer.rope, c.start, buffer.cursor).is_some()
        });
        match &mut self.completion {
            Some(open) if on_word => open.reset(),
            _ => self.completion = None,
        }
    }

    /// Replaces the word with the selected item's text as one undo step, and
    /// closes the popup.
    fn accept_completion(&mut self, typed: &str) {
        let Some(open) = self.completion.take() else {
            return;
        };
        let Some(item) = open.selected(typed) else {
            return;
        };
        let cursor = self.buffer().cursor;
        let len = self.buffer().rope.len_chars();
        let range = open.replaced(item, cursor);
        let range = range.start.min(len)..range.end.min(len);
        let text = item.text.clone();
        self.edit(|buffer| {
            buffer.seal_undo_group();
            buffer.replace_ranges(&[(range, text)]);
        });
    }

    /// Asks the active buffer's language server for completions at the cursor.
    /// The answer arrives later as an `AppEvent::Lsp`; with no server, nothing
    /// shows.
    fn request_completion(&mut self) {
        self.completion = None;
        // The server must have the text as it is now, including the key that
        // asked, before it reads the position.
        self.sync_lsp();
        let doc = self.tabs.active().id;
        let buffer = self.buffer();
        let (cursor, start) = (
            buffer.cursor,
            completion::word_start(&buffer.rope, buffer.cursor),
        );
        if self.lsp.completion(doc, cursor) {
            self.completion_for = Some((doc, start, cursor));
        }
    }

    /// Opens the popup with a completion answer, unless something else has the
    /// keys, the buffer changed, or the cursor has left the word it was asked
    /// about.
    fn show_completion(&mut self, doc: u64, items: Vec<CompletionItem>) {
        let Some((asked_doc, start, asked_at)) = self.completion_for.take() else {
            return;
        };
        let buffer = self.buffer();
        let on_word = completion::typed_word(&buffer.rope, start, buffer.cursor).is_some();
        if asked_doc == doc
            && self.tabs.active().id == doc
            && on_word
            && self.focus == Focus::Editor
            && !self.modal_open()
            && !items.is_empty()
        {
            self.completion = Some(Completion::new(doc, start, asked_at, items));
        }
    }

    /// Keys while the tree has focus: arrows browse, Enter opens; the actions that
    /// make sense anywhere go to `handle_action`, the editing ones are dropped.
    fn handle_tree_action(&mut self, action: Action) {
        match action {
            Action::Move(Motion::Up) => self.tree.move_by(-1),
            Action::Move(Motion::Down) => self.tree.move_by(1),
            Action::Move(Motion::PageUp) => self.tree.move_by(-self.tree_page()),
            Action::Move(Motion::PageDown) => self.tree.move_by(self.tree_page()),
            Action::Move(Motion::DocStart) => self.tree.select(0),
            Action::Move(Motion::DocEnd) => self.tree.move_by(isize::MAX),
            Action::Move(Motion::Right) => self.tree.expand(),
            Action::Move(Motion::Left) => self.tree.collapse(),
            Action::Newline => self.activate_tree_row(),
            Action::Cancel => self.focus = Focus::Editor,
            Action::TreeNewFile => self.start_create(false),
            Action::TreeNewFolder => self.start_create(true),
            Action::TreeRename => self.start_rename(),
            Action::TreeDelete => self.start_trash(),
            Action::Quit
            | Action::Save
            | Action::ToggleTree
            | Action::FocusTree
            | Action::PrevTab
            | Action::NextTab
            | Action::GoToTab(_)
            | Action::CloseTab
            | Action::NewFile
            | Action::SaveAs
            | Action::ToggleSplit
            | Action::CycleFocus
            | Action::GoToFile
            | Action::GoToLine
            | Action::Find
            | Action::Replace
            | Action::ProjectSearch
            | Action::Run
            | Action::ToggleRunPanel
            | Action::StopRun
            | Action::RestartRun
            | Action::OpenDirectory => {
                self.handle_action(action);
            }
            _ => {}
        }
        self.follow_tree();
    }

    /// Enter or a click on the selected row: a folder opens or closes, a file opens
    /// in a tab.
    fn activate_tree_row(&mut self) {
        let Some(row) = self.tree.selected_row() else {
            return;
        };
        if row.entry.is_dir {
            self.tree.toggle();
        } else {
            let path = row.entry.path.clone();
            self.open(&path);
        }
    }

    /// Shows `path` in the editor: switches to its tab when it's already open,
    /// otherwise opens it in a new one. An untouched untitled tab is replaced rather
    /// than left behind.
    /// Both happen in the focused split; a file open only in the other split gets
    /// a tab here too, on the same buffer.
    fn open(&mut self, path: &Path) {
        if let Some(doc) = self.tabs.find(path) {
            self.tabs.show(doc);
            self.reset_mouse();
            self.focus = Focus::Editor;
            self.follow_cursor();
            return;
        }
        match Buffer::open(path) {
            Ok(buffer) => {
                let active = self.tabs.active();
                if active.is_pristine() && !self.tabs.shown_elsewhere(active.id) {
                    self.tabs.active_mut().buffer = buffer;
                    self.tabs.active_tab_mut().view = View::default();
                } else {
                    self.tabs.push(buffer);
                }
                self.reset_mouse();
                self.focus = Focus::Editor;
                if let Some(text) = self.backups.recoverable(path) {
                    self.recovery = Some(text);
                    self.recovery_from = self.backups.written(path);
                    self.ask(Prompt::Recover);
                }
                self.follow_cursor();
            }
            Err(err) => self.say(Tone::Err, format!("cannot open {}: {err}", path.display())),
        }
    }

    /// Switches the focused split to its tab `index`, if it has one.
    fn switch_tab(&mut self, index: usize) {
        let split = self.tabs.split();
        if index < split.tabs.len() && index != split.active {
            self.switch_to(self.tabs.focused, index);
        }
    }

    /// Focuses tab `index` of split `split`.
    fn switch_to(&mut self, split: usize, index: usize) {
        self.tabs.activate(split, index);
        self.reset_mouse();
        // The pane may have changed size while this tab was hidden.
        self.follow_cursor();
    }

    /// Steps `delta` tabs along the focused split, wrapping at either end.
    fn cycle_tab(&mut self, delta: isize) {
        let split = self.tabs.split();
        let len = split.tabs.len();
        let index = split.active.checked_add_signed(delta).unwrap_or(len - 1) % len;
        self.switch_tab(index);
    }

    /// A click or drag in one buffer means nothing in another.
    fn reset_mouse(&mut self) {
        self.last_click = None;
        self.drag_from = None;
    }

    /// Ctrl+W or a middle click: closes the active tab, asking first when that
    /// would throw away unsaved changes. A buffer the other split still shows
    /// loses nothing, so that close doesn't ask.
    fn request_close(&mut self) {
        let doc = self.tabs.active();
        if doc.buffer.dirty && !self.tabs.shown_elsewhere(doc.id) {
            self.ask(Prompt::UnsavedClose);
        } else {
            self.close_active();
        }
    }

    /// Closes the active tab without asking; its edits, if any, were saved or
    /// discarded, so its backup goes too, unless the other split still shows it.
    fn close_active(&mut self) {
        if let Some(doc) = self.tabs.close_active() {
            let _ = self.backups.delete(doc.buffer.path.as_deref(), doc.id);
        }
        self.reset_mouse();
        self.follow_cursor();
    }

    /// Ctrl+Q, and each answer to the quit prompt: asks about the next tab with
    /// unsaved changes (switching to it, so the user sees what they're deciding
    /// about), or quits when none is left.
    fn continue_quit(&mut self) {
        let next = self
            .tabs
            .docs
            .iter()
            .find(|doc| doc.buffer.dirty && !self.quit_discarded.contains(&doc.id))
            .map(|doc| doc.id);
        match next {
            Some(doc) => {
                self.tabs.reveal(doc);
                self.reset_mouse();
                self.follow_cursor();
                self.ask(Prompt::UnsavedQuit);
            }
            None => self.quit(),
        }
    }

    fn toggle_tree(&mut self) {
        self.tree_visible = !self.tree_visible;
        if !self.tree_visible {
            self.focus = Focus::Editor;
        }
        // The editor pane just changed width.
        self.follow_cursor();
        self.follow_tree();
    }

    fn switch_focus(&mut self) {
        match self.focus {
            Focus::Tree => self.focus = Focus::Editor,
            Focus::Editor => {
                if !self.tree_visible {
                    self.toggle_tree();
                }
                self.focus = Focus::Tree;
            }
        }
    }

    /// F6: focus moves tree, left split, right split and round again, skipping
    /// the tree while it's hidden.
    fn cycle_focus(&mut self) {
        let mut stops: Vec<Option<usize>> = Vec::new();
        if self.tree_visible {
            stops.push(None);
        }
        stops.extend((0..self.tabs.splits.len()).map(Some));
        let here = match self.focus {
            Focus::Tree => None,
            Focus::Editor => Some(self.tabs.focused),
        };
        let at = stops.iter().position(|&stop| stop == here).unwrap_or(0);
        match stops[(at + 1) % stops.len()] {
            None => self.focus = Focus::Tree,
            Some(split) => {
                self.focus = Focus::Editor;
                if split != self.tabs.focused {
                    self.switch_to(split, self.tabs.splits[split].active);
                }
            }
        }
    }

    /// Alt+V: opens the right split on the focused tab, or closes it.
    fn toggle_split(&mut self) {
        self.tabs.toggle_split();
        self.focus = Focus::Editor;
        self.reset_mouse();
        // Every split just changed width.
        self.follow_cursor();
    }

    /// Rows one PageUp/PageDown moves in the tree: the node rows below the brand,
    /// so a page never skips rows the user hasn't seen.
    fn tree_page(&self) -> isize {
        let rows = self
            .panes()
            .tree
            .map_or(0, |area| nodes_area(area).height)
            .max(1);
        isize::try_from(rows).unwrap_or(isize::MAX)
    }

    fn follow_tree(&mut self) {
        if let Some(area) = self.panes().tree {
            self.tree.follow(usize::from(nodes_area(area).height));
        }
    }

    fn handle_action(&mut self, action: Action) {
        // Enter joins the run of typing it ends; every other action closes it.
        if action != Action::Newline {
            self.buffer_mut().seal_undo_group();
        }
        match action {
            Action::Quit => {
                self.quit_discarded.clear();
                self.continue_quit();
            }
            Action::Save if self.buffer().path.is_some() => self.save_formatted(),
            Action::Save => self.save_or_ask(AfterSave::Nothing),
            // Nothing to cancel outside a prompt.
            Action::Cancel => {}
            Action::Move(motion) => {
                let page = usize::from(self.editor_area().height);
                let tab_width = self.editor.tab_width;
                self.buffer_mut().move_cursor(motion, page, tab_width);
                self.follow_cursor();
            }
            Action::Newline => self.edit(Buffer::newline),
            Action::Backspace => self.edit(Buffer::backspace),
            Action::Delete => self.edit(Buffer::delete),
            Action::Tab => {
                let (width, spaces) = (self.editor.tab_width, self.editor.insert_spaces);
                self.edit(|buffer| buffer.tab(width, spaces));
            }
            Action::Undo => self.edit(|buffer| {
                buffer.undo();
            }),
            Action::Redo => self.edit(|buffer| {
                buffer.redo();
            }),
            Action::Select(motion) => {
                let page = usize::from(self.editor_area().height);
                let tab_width = self.editor.tab_width;
                self.buffer_mut().select(motion, page, tab_width);
                self.follow_cursor();
            }
            Action::SelectAll => {
                self.buffer_mut().select_all();
                self.follow_cursor();
            }
            Action::Copy => {
                self.copy();
            }
            Action::Cut => {
                if self.copy() {
                    self.edit(|buffer| {
                        buffer.delete_selection();
                    });
                }
            }
            Action::Paste => match self.clipboard.get() {
                Ok(text) => self.edit(|buffer| buffer.paste(&text)),
                Err(err) => self.say(Tone::Err, format!("cannot paste: {err}")),
            },
            Action::ToggleTree => self.toggle_tree(),
            Action::FocusTree => self.switch_focus(),
            Action::PrevTab => self.cycle_tab(-1),
            Action::NextTab => self.cycle_tab(1),
            Action::GoToTab(n) => self.switch_tab(usize::from(n).saturating_sub(1)),
            Action::CloseTab => self.request_close(),
            Action::NewFile => {
                self.tabs.push(Buffer::empty());
                self.reset_mouse();
                self.focus = Focus::Editor;
                self.follow_cursor();
            }
            Action::SaveAs => self.start_save_as(AfterSave::Nothing),
            Action::ToggleSplit => self.toggle_split(),
            Action::CycleFocus => self.cycle_focus(),
            Action::GoToFile => self.open_picker(""),
            Action::GoToLine => self.open_picker(":"),
            Action::Find => self.open_find(false),
            Action::Replace => self.open_find(true),
            Action::ProjectSearch => self.project_search = Some(ProjectSearch::new()),
            Action::Run => self.start_run(),
            Action::ToggleRunPanel => self.toggle_run_panel(),
            Action::StopRun => self.stop_run(),
            Action::RestartRun => self.restart_run(),
            Action::NextDiagnostic => self.jump_to_diagnostic(true),
            Action::PrevDiagnostic => self.jump_to_diagnostic(false),
            Action::GoToDefinition => self.request_definition(),
            Action::JumpBack => self.jump_back(),
            Action::Hover => self.request_hover(),
            Action::Complete => self.request_completion(),
            Action::OpenDirectory => self.open_folders(),
            // Bound only in tree, find bar or folder browser scope, so they never
            // reach the editor.
            Action::TreeNewFile
            | Action::TreeNewFolder
            | Action::TreeRename
            | Action::TreeDelete
            | Action::FindNext
            | Action::FindPrev
            | Action::FindCase
            | Action::FindRegex
            | Action::ReplaceAll
            | Action::ProjectReplace
            | Action::OpenFolderHere => {}
        }
    }

    /// The card a confirm prompt shows (SPEC_V1_LAYOUT §7.4).
    fn confirm(&self, prompt: Prompt) -> Confirm {
        let name = self
            .buffer()
            .path
            .as_deref()
            .map_or_else(|| "untitled".to_string(), file_name);
        match prompt {
            Prompt::UnsavedQuit | Prompt::UnsavedClose => Confirm {
                question: Cow::Owned(format!("{name} has unsaved changes")),
                explanation: Cow::Borrowed("Closing discards them unless you save."),
                choices: UNSAVED,
            },
            Prompt::Recover => Confirm {
                question: Cow::Owned(format!("Recover unsaved changes to {name}?")),
                explanation: Cow::Owned(match self.recovery_from {
                    Some(time) => format!(
                        "A backup from {} is newer than the file on disk.",
                        chrono::DateTime::<chrono::Local>::from(time).format("%H:%M")
                    ),
                    None => "A backup is newer than the file on disk.".to_string(),
                }),
                choices: RECOVER,
            },
            Prompt::Trash => Confirm {
                question: Cow::Owned(format!(
                    "Move {} to the trash?",
                    self.pending_trash
                        .as_deref()
                        .map(file_name)
                        .unwrap_or_default()
                )),
                explanation: Cow::Borrowed("You can restore it from the system trash."),
                choices: TRASH,
            },
            Prompt::ProjectReplace => {
                let (matches, files) = self
                    .pending_replace
                    .as_ref()
                    .map_or((0, 0), |pending| (pending.matches, pending.files.len()));
                Confirm {
                    question: Cow::Owned(format!(
                        "Replace {} in {}?",
                        plural(matches, "match", "matches"),
                        plural(files, "file", "files")
                    )),
                    explanation: Cow::Borrowed(
                        "Open buffers are edited in place (undo with Ctrl+Z); other files are saved.",
                    ),
                    choices: REPLACE,
                }
            }
        }
    }

    /// Opens the confirm card for `prompt`, its first button focused.
    fn ask(&mut self, prompt: Prompt) {
        self.prompt = Some(prompt);
        self.prompt_button = 0;
    }

    /// Acts on a key or click while `prompt`'s card is open.
    fn prompt_step(&mut self, prompt: Prompt, step: Reply) {
        match step {
            Reply::Answer(answer) => self.answer_prompt(prompt, answer),
            Reply::Focus(button) => self.prompt_button = button,
        }
    }

    /// The folder a new entry goes in: the selected folder, or the selected file's
    /// folder, or the root when the tree is empty.
    fn target_folder(&self) -> PathBuf {
        match self.tree.selected_row() {
            Some(row) if row.entry.is_dir => row.entry.path.clone(),
            Some(row) => row
                .entry
                .path
                .parent()
                .map_or_else(|| self.tree.root().to_path_buf(), Path::to_path_buf),
            None => self.tree.root().to_path_buf(),
        }
    }

    fn start_create(&mut self, folder: bool) {
        let dir = self.target_folder();
        let (op, label) = if folder {
            (BarOp::NewFolder(dir), "New folder")
        } else {
            (BarOp::NewFile(dir), "New file")
        };
        self.open_name_prompt(op, PromptBar::new(label, ""));
    }

    fn start_rename(&mut self) {
        let Some(row) = self.tree.selected_row() else {
            return;
        };
        let bar = PromptBar::new("Rename", &row.entry.name);
        self.open_name_prompt(BarOp::Rename(row.entry.path.clone()), bar);
    }

    /// Opens the prompt bar for a path to save the active buffer to, holding its
    /// current path relative to the project root.
    fn start_save_as(&mut self, after: AfterSave) {
        let current = self.buffer().path.as_deref().map(|path| {
            path.strip_prefix(self.tree.root())
                .unwrap_or(path)
                .display()
                .to_string()
        });
        let bar = PromptBar::new("Save as", current.as_deref().unwrap_or_default());
        self.open_name_prompt(BarOp::SaveAs(after), bar);
    }

    fn open_name_prompt(&mut self, op: BarOp, bar: PromptBar) {
        self.name_prompt = Some(NamePrompt { op, bar });
        // The bar takes a row from the panes above it.
        self.follow_cursor();
        self.follow_tree();
    }

    fn start_trash(&mut self) {
        if let Some(row) = self.tree.selected_row() {
            self.pending_trash = Some(row.entry.path.clone());
            self.ask(Prompt::Trash);
        }
    }

    /// Closes the prompt bar and, on Enter, does what it asked for. A failure says
    /// why in the status line; the operations check before they act, so it has
    /// changed nothing.
    fn finish_name_prompt(&mut self, outcome: Outcome) {
        let Some(NamePrompt { op, bar }) = self.name_prompt.take() else {
            return;
        };
        self.follow_cursor();
        self.follow_tree();
        if let BarOp::SaveAs(after) = op {
            let saved = outcome == Outcome::Submit && self.save_as(bar.text());
            self.after_save(after, saved);
            return;
        }
        if outcome == Outcome::Cancel {
            return;
        }
        let name = bar.text();
        let result = match &op {
            BarOp::NewFile(dir) => ops::create_file(dir, name),
            BarOp::NewFolder(dir) => ops::create_folder(dir, name),
            BarOp::Rename(path) => ops::rename(path, name),
            // Handled above.
            BarOp::SaveAs(_) => return,
        };
        let path = match result {
            Ok(path) => path,
            Err(err) => {
                self.say(Tone::Err, err.to_string());
                return;
            }
        };
        self.tree.reload(Some(&path));
        self.follow_tree();
        match op {
            BarOp::NewFile(_) => {
                self.say(Tone::Ok, format!("created {name}"));
                self.open(&path);
            }
            BarOp::NewFolder(_) => self.say(Tone::Ok, format!("created {name}")),
            BarOp::Rename(old) => {
                self.say(Tone::Ok, format!("renamed to {name}"));
                self.follow_rename(&old, &path);
            }
            BarOp::SaveAs(_) => {}
        }
    }

    /// Moves the cursor to `line`, counting from 1.
    fn go_to_line(&mut self, line: usize) {
        self.focus = Focus::Editor;
        self.buffer_mut().go_to_line(line);
        self.follow_cursor();
    }

    /// Ctrl+F (or Ctrl+R, `replace`, which adds the Replace field): opens the find
    /// bar over the active buffer, holding the selection when it's on one line,
    /// and jumps to the first match from the cursor.
    fn open_find(&mut self, replace: bool) {
        self.focus = Focus::Editor;
        let buffer = self.buffer();
        let text = buffer
            .selected_text()
            .filter(|text| !text.contains('\n'))
            .unwrap_or_default();
        let origin = buffer.selection().map_or(buffer.cursor, |s| s.start);
        let mut find = FindBar::new(&text, origin, &buffer.rope);
        if replace {
            // Typing starts in Find; Tab moves to Replace.
            find.show_replace(false);
        }
        self.find = Some(find);
        // The bar takes a row from the panes above it.
        self.find_step(Step::Jump);
    }

    /// Follows what a key did in the find bar: the cursor goes to the current
    /// match, or stays where it is when the bar closes; replacing edits the buffer
    /// and then goes to the match after it.
    fn find_step(&mut self, step: Step) {
        match step {
            Step::Stay => {}
            Step::Jump => self.jump_to_match(),
            Step::Close => self.find = None,
            Step::Replace => {
                self.replace_current();
                self.jump_to_match();
            }
            Step::ReplaceAll => {
                self.replace_all();
                self.jump_to_match();
            }
        }
        self.follow_cursor();
    }

    fn jump_to_match(&mut self) {
        if let Some(at) = self.find.as_ref().and_then(FindBar::current_match) {
            self.buffer_mut().place_cursor(at.start);
        }
    }

    /// Replaces the current match and makes the next one after the new text
    /// current, so a replacement that contains the pattern isn't matched again.
    fn replace_current(&mut self) {
        let Some(edit) = self
            .find
            .as_ref()
            .and_then(|find| find.current_replacement(&self.tabs.active().buffer.rope))
        else {
            return;
        };
        let next = edit.0.start + edit.1.chars().count();
        self.edit(|buffer| buffer.replace_ranges(std::slice::from_ref(&edit)));
        if let Some(find) = &mut self.find {
            find.research_from(&self.tabs.active().buffer.rope, next);
        }
    }

    /// Replaces every match as one undo step and says how many in the status line.
    fn replace_all(&mut self) {
        let Some(find) = &self.find else {
            return;
        };
        let edits = find.replacements(&self.tabs.active().buffer.rope);
        if !edits.is_empty() {
            self.edit(|buffer| buffer.replace_ranges(&edits));
        }
        self.say(Tone::Ok, format!("Replaced {}", edits.len()));
        let buffer = &self.tabs.active().buffer;
        if let Some(find) = &mut self.find {
            find.research_from(&buffer.rope, buffer.cursor);
        }
    }

    /// Ctrl+P (or Ctrl+G, with `:` typed): opens the cast palette holding
    /// `query`, with every global command and its key, and lists the project's
    /// files on a background thread, so a large project never stalls typing
    /// (ADR-0001). The files are walked whatever the query, as deleting its
    /// prefix goes back to listing them. Outside the event loop (unit tests) the
    /// walk runs inline.
    fn open_picker(&mut self, query: &str) {
        self.picker_for = PickerFor::File;
        let commands = Action::commands()
            .map(|action| picker::Command {
                action,
                title: action.title(),
                key: self.keymap.key_label(action),
            })
            .collect();
        let lines = self.buffer().rope.len_lines();
        let mut picker = Picker::cast(commands, lines, query);
        let root = self.tree.root().to_path_buf();
        self.walks += 1;
        let walk = self.walks;
        match &self.events {
            Some(events) => {
                let events = events.clone();
                tokio::task::spawn_blocking(move || {
                    let files = list_files(&root);
                    let _ = events.send(AppEvent::FilesListed { walk, files });
                });
            }
            None => picker.set_files(list_files(&root)),
        }
        self.picker = Some(picker);
    }

    /// `>open directory`: the folder browser, starting in the project folder.
    fn open_folders(&mut self) {
        match DirPicker::new(self.tree.root()) {
            Ok(browser) => self.folders = Some(browser),
            Err(why) => self.say(Tone::Err, why),
        }
    }

    /// Follows what a key did in the folder browser.
    fn folders_step(&mut self, step: Browsed) {
        match step {
            Browsed::Open(dir) => {
                self.folders = None;
                self.open_project(&dir);
            }
            // The browser stays where it was, under the message.
            Browsed::Failed(why) => self.say(Tone::Err, why),
            Browsed::Close => self.folders = None,
        }
    }

    /// Makes `dir` the project, as `glyph <dir>` would: every tab closes, the
    /// run is stopped, the old folder's language servers are shut down, and
    /// the tree shows the new folder with focus. Everything else that reads
    /// the project (Ctrl+P, project search, F5, save as, new servers) asks the
    /// tree for its root, so it follows. Unsaved changes would be lost, so with
    /// any it refuses and says so.
    fn open_project(&mut self, dir: &Path) {
        if self.tabs.docs.iter().any(|doc| doc.buffer.dirty) {
            self.say(Tone::Warn, "save or close unsaved files first");
            return;
        }
        // The run's command was started in the old folder and belongs to it.
        if let Some(view) = &mut self.run
            && view.status == RunStatus::Running
        {
            view.status = RunStatus::Stopped;
        }
        if let Some(tree) = self.run_tree.take() {
            tree.kill();
        }
        // Ctrl+F5 would run the old command in the new folder; without it,
        // Ctrl+F5 is F5, which reads the new folder's `.glyph.toml`.
        self.run_entry = None;
        self.close_project_search();
        self.find = None;
        self.hover = None;
        self.hover_for = None;
        self.completion = None;
        self.completion_for = None;
        // Jumps lead back into the old folder's files.
        self.jumps.clear();
        if self.tabs.splits.len() > 1 {
            self.tabs.toggle_split();
        }
        // Every tab is saved, so no backup is needed any more.
        for doc in self.tabs.close_where(|_| true) {
            let _ = self.backups.delete(doc.buffer.path.as_deref(), doc.id);
        }
        // The servers' exit is waited for on a task of its own (R29), so the
        // handle isn't needed here.
        drop(self.lsp.stop_root(&absolute(self.tree.root())));
        self.tree = Tree::new(dir);
        self.project = project_label(dir, std::env::home_dir().as_deref());
        self.tree_visible = true;
        self.focus = Focus::Tree;
        self.reset_mouse();
        self.follow_cursor();
        self.follow_tree();
    }

    /// Cast's `/text`: opens project search holding `text` and, as Enter was
    /// already pressed on it, searches for it.
    fn open_project_search(&mut self, text: &str) {
        let panel = ProjectSearch::with_query(text);
        let query = panel.query();
        self.project_search = Some(panel);
        if !query.pattern.is_empty() {
            self.start_project_search(query);
        }
    }

    /// Follows what a key did in the project search panel.
    fn project_search_step(&mut self, step: Searched) {
        match step {
            Searched::Start(query) => self.start_project_search(query),
            Searched::Open(hit) => {
                self.close_project_search();
                self.open_hit(&hit);
            }
            Searched::Replace { query, with, files } => {
                self.ask_project_replace(query, with, files)
            }
            Searched::Close => self.close_project_search(),
        }
    }

    /// The file at `relative`, a `/`-separated path under the project root.
    fn project_path(&self, relative: &str) -> PathBuf {
        let mut path = self.tree.root().to_path_buf();
        path.extend(relative.split('/'));
        path
    }

    /// How many matches of `query` the file at `relative` holds: in its open
    /// buffer, edits and all, or else on disk. A file that can't be read counts
    /// none.
    fn count_matches(&self, relative: &str, query: &Query) -> usize {
        let path = self.project_path(relative);
        match self.tabs.find(&path) {
            Some(doc) => search::find_all(&self.tabs.doc(doc).buffer.rope, query)
                .map_or(0, |found| found.len()),
            None => search::count_in_file(&path, query).unwrap_or(0),
        }
    }

    /// Alt+A in project search: counts what would change and asks first.
    fn ask_project_replace(&mut self, query: Query, with: String, files: Vec<String>) {
        let mut matches = 0;
        let mut kept = Vec::new();
        for file in files {
            let count = self.count_matches(&file, &query);
            if count > 0 {
                matches += count;
                kept.push(file);
            }
        }
        if matches == 0 {
            self.say(Tone::Warn, "nothing to replace");
            return;
        }
        self.pending_replace = Some(PendingReplace {
            query,
            with,
            files: kept,
            matches,
        });
        self.ask(Prompt::ProjectReplace);
    }

    /// The replace prompt's yes: open buffers are edited in place, one undo step
    /// each and left unsaved; other files are rewritten and saved atomically. A
    /// file that fails is named in the status line and the rest still go ahead.
    /// Then the search runs again, so the list shows what's left.
    fn replace_in_project(&mut self) {
        let Some(pending) = self.pending_replace.take() else {
            return;
        };
        let mut replaced = 0;
        let mut files = 0;
        let mut failures = Vec::new();
        let mut edited = false;
        for relative in &pending.files {
            let path = self.project_path(relative);
            let count = match self.tabs.find(&path) {
                Some(id) => {
                    let doc = self.tabs.doc_mut(id);
                    let edits =
                        search::replacements(&doc.buffer.rope, &pending.query, &pending.with)
                            .unwrap_or_default();
                    if !edits.is_empty() {
                        doc.buffer.replace_ranges(&edits);
                        doc.backup_due = true;
                        edited = true;
                    }
                    edits.len()
                }
                None => match search::replace_in_file(&path, &pending.query, &pending.with) {
                    Ok(count) => count,
                    Err(err) => {
                        failures.push(format!("cannot write {relative}: {err}"));
                        0
                    }
                },
            };
            if count > 0 {
                replaced += count;
                files += 1;
            }
        }
        if edited {
            self.follow_cursor();
            if let Some(edits) = &self.edits {
                // The timer only stops with the loop, after which nothing edits.
                let _ = edits.send(());
            }
        }
        let mut message = vec![format!(
            "Replaced {replaced} in {}",
            plural(files, "file", "files")
        )];
        message.extend(failures);
        // A file that could not be written turns the report into a warning.
        let tone = if message.len() > 1 {
            Tone::Warn
        } else {
            Tone::Ok
        };
        self.say(tone, message.join(" · "));
        self.start_project_search(pending.query);
    }

    fn close_project_search(&mut self) {
        self.stop_project_search();
        self.project_search = None;
    }

    fn stop_project_search(&mut self) {
        if let Some(cancel) = self.search_cancel.take() {
            cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Searches the project for `query` on a background thread, so a large
    /// project never stalls typing (ADR-0001); hits stream back in batches.
    /// Open buffers are searched as they are in memory, edits and all, and their
    /// files left out of the walk. Outside the event loop (unit tests) the search
    /// runs inline.
    fn start_project_search(&mut self, query: Query) {
        self.stop_project_search();
        self.searches += 1;
        let id = self.searches;
        let Some(panel) = &mut self.project_search else {
            return;
        };
        panel.start(id, query.clone());
        let re = match search::compile(&query) {
            Ok(re) => re,
            Err(err) => {
                panel.fail(err);
                return;
            }
        };
        let root = self.tree.root().to_path_buf();
        let root_path = absolute(&root);
        let mut skip = HashSet::new();
        let mut open = Vec::new();
        for doc in &self.tabs.docs {
            let Some(path) = doc.buffer.path.as_deref().map(absolute) else {
                continue;
            };
            if let Ok(relative) = path.strip_prefix(&root_path) {
                open.push((relative_name(relative), doc.buffer.rope.to_string()));
                skip.insert(path);
            }
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.search_cancel = Some(Arc::clone(&cancel));
        let run = move |send: &(dyn Fn(Vec<Hit>) + Sync)| {
            for (path, text) in &open {
                let hits = search::hits_in(path, text, &re);
                if !hits.is_empty() {
                    send(hits);
                }
            }
            search::search_project(&root, &re, &skip, &cancel, send);
        };
        match &self.events {
            Some(events) => {
                let events = events.clone();
                tokio::task::spawn_blocking(move || {
                    // The loop has stopped if a send fails; nobody is left to tell.
                    let send = |hits| {
                        let _ = events.send(AppEvent::SearchHits { search: id, hits });
                    };
                    run(&send);
                    let _ = events.send(AppEvent::SearchDone { search: id });
                });
            }
            None => {
                let found = Mutex::new(Vec::new());
                run(&|hits| {
                    found
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .extend(hits);
                });
                panel.add(
                    id,
                    found
                        .into_inner()
                        .unwrap_or_else(std::sync::PoisonError::into_inner),
                );
                panel.finish(id);
            }
        }
    }

    /// Opens a hit's file in a tab with the cursor on the match.
    fn open_hit(&mut self, hit: &Hit) {
        let path = self.project_path(&hit.path);
        self.open(&path);
        // A file that failed to open left some other buffer active.
        let opened = self.buffer().path.as_deref().map(absolute) == Some(absolute(&path));
        if opened {
            self.buffer_mut()
                .go_to_line_col(hit.line.line, hit.line.col);
            self.follow_cursor();
        }
    }

    /// Closes the picker and, on Enter, opens the chosen file in a tab, runs the
    /// chosen command or action, goes to the line or searches the project.
    fn finish_picker(&mut self, picked: Picked) {
        self.picker = None;
        let picker_for = std::mem::take(&mut self.picker_for);
        let choice = match picked {
            Picked::Open(choice) => choice,
            Picked::Close => return,
            Picked::Run(action) => {
                self.handle_action(action);
                return;
            }
            Picked::Line(line) => {
                self.go_to_line(line);
                return;
            }
            Picked::Search(text) => {
                self.open_project_search(&text);
                return;
            }
        };
        match picker_for {
            PickerFor::File => {
                let mut path = self.tree.root().to_path_buf();
                path.extend(choice.split('/'));
                self.open(&path);
            }
            PickerFor::Run(entries) => {
                if let Some(entry) = entries.into_iter().find(|e| e.name == choice) {
                    self.run_entry(entry);
                }
            }
        }
    }

    /// F5: reads `.glyph.toml` afresh, so edits to it count without a restart,
    /// then runs its one entry or asks which of several. Without `[[run]]` it
    /// offers commands detected from the project files instead.
    fn start_run(&mut self) {
        if let Some(run) = &self.run
            && run.status == RunStatus::Running
        {
            self.say(Tone::Warn, format!("{} is already running", run.name));
            return;
        }
        let loaded = config::load_project(self.tree.root());
        if let Some(error) = loaded.error {
            self.say(Tone::Err, error);
            return;
        }
        let configured = !loaded.config.run.is_empty();
        let mut entries = crate::run::run_choices(loaded.config.run, self.tree.root());
        match entries.len() {
            0 => {
                self.say(
                    Tone::Warn,
                    format!(
                        "nothing to run: add a [[run]] entry to {}",
                        config::PROJECT_FILE
                    ),
                );
            }
            // A guessed command is only offered, never started unasked.
            1 if configured => {
                // The arm matched a length of one, so index 0 exists.
                let entry = entries.remove(0);
                self.run_entry(entry);
            }
            _ => {
                let source = if configured {
                    config::PROJECT_FILE
                } else {
                    "detected"
                };
                let choices = entries
                    .iter()
                    .map(|e| picker::Choice {
                        name: e.name.clone(),
                        detail: e.command.clone(),
                        source,
                    })
                    .collect();
                self.picker = Some(Picker::choices("Run", choices));
                self.picker_for = PickerFor::Run(entries);
            }
        }
    }

    /// Starts `entry` and shows the panel for its output. Outside the event loop
    /// (unit tests) there's no channel for output to come back on, so nothing runs.
    fn run_entry(&mut self, entry: RunEntry) {
        self.start_entry(entry, false);
    }

    /// Starts `entry` in a fresh panel, marked `restarted` when it replaces a run
    /// of itself.
    fn start_entry(&mut self, entry: RunEntry, restarted: bool) {
        let Some(events) = self.events.clone() else {
            return;
        };
        // The previous run's leftovers die with its tree.
        self.run_tree = None;
        self.runs += 1;
        match crate::run::spawn(self.runs, &entry, self.tree.root(), events) {
            Ok(tree) => {
                self.run_tree = Some(tree);
                let mut view = RunView::new(self.runs, entry.name.clone(), entry.command.clone());
                if restarted {
                    view.mark_restarted(chrono::Local::now().format("%H:%M:%S").to_string());
                }
                self.run = Some(view);
                self.run_entry = Some(entry);
                if !self.run_panel_visible {
                    self.toggle_run_panel();
                }
            }
            Err(err) => self.say(Tone::Err, format!("cannot run {}: {err}", entry.name)),
        }
    }

    /// Shift+F5: kills the running command and everything it started.
    fn stop_run(&mut self) {
        match &mut self.run {
            Some(view) if view.status == RunStatus::Running => {
                view.status = RunStatus::Stopped;
                if let Some(tree) = &self.run_tree {
                    tree.kill();
                }
            }
            _ => self.say(Tone::Warn, "nothing is running"),
        }
    }

    /// Ctrl+F5: stops the latest run if it's still going and starts it again;
    /// with nothing run yet it's F5.
    fn restart_run(&mut self) {
        let Some(entry) = self.run_entry.clone() else {
            self.start_run();
            return;
        };
        if let Some(tree) = self.run_tree.take() {
            tree.kill();
        }
        self.start_entry(entry, true);
    }

    /// F4: shows or hides the run panel.
    fn toggle_run_panel(&mut self) {
        self.run_panel_visible = !self.run_panel_visible;
        // The editor pane just changed height.
        self.follow_cursor();
        self.follow_tree();
    }

    /// Carries on with what a save from a question was for. A save that failed or
    /// was cancelled stops a close or a quit, leaving everything open.
    fn after_save(&mut self, after: AfterSave, saved: bool) {
        match (after, saved) {
            (AfterSave::Close, true) => self.close_active(),
            (AfterSave::Quit, true) => self.continue_quit(),
            (AfterSave::Quit, false) => self.quit_discarded.clear(),
            _ => {}
        }
    }

    /// Saves the active buffer to `relative` (to the project root) and points its
    /// tab there. Refuses a path another tab has open or that names some other file
    /// already on disk, so save as never overwrites anything by surprise.
    fn save_as(&mut self, relative: &str) -> bool {
        let relative = relative.trim();
        if relative.is_empty() {
            self.say(Tone::Warn, "no file name");
            return false;
        }
        let root = self.tree.root();
        // A root of "." would otherwise show up as a "./" in every message.
        let target = if root == Path::new(".") {
            PathBuf::from(relative)
        } else {
            root.join(relative)
        };
        let same_file = self
            .buffer()
            .path
            .as_deref()
            .is_some_and(|path| absolute(path) == absolute(&target));
        if !same_file {
            if self.tabs.find(&target).is_some() {
                self.say(Tone::Warn, format!("{relative} is open in another tab"));
                return false;
            }
            if target.exists() {
                self.say(Tone::Warn, format!("{relative} already exists"));
                return false;
            }
        }
        let (id, old) = (self.tabs.active().id, self.buffer().path.clone());
        self.buffer_mut().path = Some(target.clone());
        let name = self.buffer().name();
        match self.buffer_mut().save() {
            Ok(()) => {
                // The backup was keyed by the old path (or the tab, if untitled).
                let _ = self.backups.delete(old.as_deref(), id);
                self.say(Tone::Ok, format!("saved {name}"));
                self.tree.reload(Some(&target));
                self.follow_tree();
                true
            }
            Err(err) => {
                self.buffer_mut().path = old;
                self.say(Tone::Err, format!("cannot save {name}: {err}"));
                false
            }
        }
    }

    /// Points every tab at its file's new path when it, or a folder holding it, was
    /// renamed, moving crash backups along.
    fn follow_rename(&mut self, old: &Path, new: &Path) {
        let mut moved_any = false;
        for doc in &mut self.tabs.docs {
            let Some(moved) = doc
                .buffer
                .path
                .as_deref()
                .and_then(|path| ops::rebase(path, old, new))
            else {
                continue;
            };
            // Backups are keyed by path, so the old one would never be found again.
            let _ = self.backups.delete(doc.buffer.path.as_deref(), doc.id);
            doc.buffer.path = Some(moved);
            doc.backup_due = true;
            moved_any = true;
        }
        if moved_any {
            self.back_up();
        }
    }

    /// The trash prompt's yes: moves the entry to the OS trash and closes the tabs
    /// showing that file or anything inside that folder.
    fn trash_pending(&mut self) {
        let Some(path) = self.pending_trash.take() else {
            return;
        };
        let name = file_name(&path);
        if let Err(err) = self.trash.delete(&path) {
            self.say(Tone::Err, format!("cannot move {name} to trash: {err}"));
            return;
        }
        let closed = self.tabs.close_where(|doc| {
            doc.buffer
                .path
                .as_deref()
                .is_some_and(|open| open.starts_with(&path))
        });
        for doc in closed {
            let _ = self.backups.delete(doc.buffer.path.as_deref(), doc.id);
        }
        self.reset_mouse();
        self.follow_cursor();
        self.say(Tone::Ok, format!("moved {name} to trash"));
        self.tree.reload(None);
        self.follow_tree();
    }

    /// Click, drag, double-click and wheel in the editor panes, clicks on the tab
    /// pills. A press in a split focuses it. `now` is when the event arrived, passed
    /// in so double-click timing is testable.
    fn handle_mouse(&mut self, mouse: MouseEvent, now: Instant) {
        let panes = self.panes();
        let at = Position::new(mouse.column, mouse.row);
        if let Some(tree) = panes.tree
            && tree.contains(at)
        {
            self.handle_tree_mouse(mouse, tree);
            return;
        }
        if let Some(split) = panes.splits.iter().position(|s| s.header.contains(at)) {
            self.handle_tab_mouse(mouse, split, panes.splits[split].header);
            return;
        }
        let hovered = panes.splits.iter().position(|s| s.editor.contains(at));
        let (col, row) = (mouse.column, mouse.row);
        let tab_width = self.editor.tab_width;
        // A drag or release belongs to the split the press landed in, wherever the
        // pointer has wandered since.
        let pos = |app: &Self| {
            let area = panes.splits[app.tabs.focused].editor;
            app.tabs
                .active_tab()
                .view
                .screen_to_char(app.buffer(), area, col, row, tab_width)
        };
        match (mouse.kind, hovered) {
            (MouseEventKind::Down(MouseButton::Left), Some(split))
                if !mouse.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                if split != self.tabs.focused {
                    self.switch_to(split, self.tabs.splits[split].active);
                }
                self.focus = Focus::Editor;
                let pos = pos(self);
                let double = self.last_click.is_some_and(|last| {
                    (last.col, last.row) == (col, row)
                        && now.saturating_duration_since(last.at) <= DOUBLE_CLICK
                });
                if double {
                    self.buffer_mut().select_word_at(pos);
                    // A third click starts over rather than counting as another double.
                    self.reset_mouse();
                } else {
                    self.buffer_mut().place_cursor(pos);
                    self.last_click = Some(Click { at: now, col, row });
                    self.drag_from = Some(pos);
                }
                self.follow_cursor();
            }
            // Ctrl+click: the cursor goes to the symbol, then to its definition.
            (MouseEventKind::Down(MouseButton::Left), Some(split)) => {
                if split != self.tabs.focused {
                    self.switch_to(split, self.tabs.splits[split].active);
                }
                self.focus = Focus::Editor;
                let pos = pos(self);
                self.reset_mouse();
                self.buffer_mut().place_cursor(pos);
                self.follow_cursor();
                self.request_definition();
            }
            (MouseEventKind::Down(_), _) => self.drag_from = None,
            (MouseEventKind::Drag(MouseButton::Left), _) => {
                if let Some(from) = self.drag_from {
                    let pos = pos(self);
                    self.buffer_mut().select_to(from, pos);
                    self.follow_cursor();
                }
            }
            (MouseEventKind::Up(MouseButton::Left), _) => {
                // A release somewhere new without a drag event in between still
                // ends the selection there.
                if let Some(from) = self.drag_from.take() {
                    let pos = pos(self);
                    if pos != self.buffer().cursor {
                        self.buffer_mut().select_to(from, pos);
                        self.follow_cursor();
                    }
                }
            }
            // The wheel scrolls whichever split it's over, without focusing it.
            (MouseEventKind::ScrollUp, Some(split)) => {
                self.tabs
                    .scroll(split, panes.splits[split].editor, -WHEEL_LINES);
            }
            (MouseEventKind::ScrollDown, Some(split)) => {
                self.tabs
                    .scroll(split, panes.splits[split].editor, WHEEL_LINES);
            }
            _ => {}
        }
    }

    /// A left click selects the tab under it, focusing its split; a middle click
    /// closes it, asking first (about that tab, now active) when it has unsaved
    /// changes.
    fn handle_tab_mouse(&mut self, mouse: MouseEvent, split: usize, header: Rect) {
        self.reset_mouse();
        let labels = self.tabs.labels(split);
        let active = self.tabs.splits[split].active;
        let Some(index) = tab_at(&labels, active, header, mouse.column, mouse.row) else {
            return;
        };
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.switch_to(split, index);
                self.focus = Focus::Editor;
            }
            MouseEventKind::Down(MouseButton::Middle) => {
                self.switch_to(split, index);
                self.request_close();
            }
            _ => {}
        }
    }

    /// A click selects the row under it and opens it (a folder opens or closes, a
    /// file opens in a tab); the wheel scrolls the tree.
    fn handle_tree_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        // Whatever the editor was tracking for a drag or double-click is over.
        self.reset_mouse();
        let nodes = nodes_area(area);
        let height = usize::from(nodes.height);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.focus = Focus::Tree;
                // The brand rows above the nodes only take focus.
                let row = (mouse.row >= nodes.y).then(|| usize::from(mouse.row - nodes.y));
                if let Some(index) = row.and_then(|line| self.tree.row_at(line)) {
                    self.tree.select(index);
                    self.activate_tree_row();
                    self.follow_tree();
                }
            }
            MouseEventKind::ScrollUp => self.tree.scroll_by(-WHEEL_LINES, height),
            MouseEventKind::ScrollDown => self.tree.scroll_by(WHEEL_LINES, height),
            _ => {}
        }
    }

    /// Puts the selection on the clipboard. Returns whether something was copied,
    /// so cut never deletes text the clipboard didn't take.
    fn copy(&mut self) -> bool {
        let Some(text) = self.buffer().selected_text() else {
            return false;
        };
        match self.clipboard.set(&text) {
            Ok(()) => true,
            Err(err) => {
                self.say(Tone::Err, format!("cannot copy: {err}"));
                false
            }
        }
    }

    fn answer_prompt(&mut self, prompt: Prompt, answer: Answer) {
        self.prompt = None;
        match (prompt, answer) {
            (Prompt::UnsavedQuit, Answer::Picked('s')) => {
                self.save_or_ask(AfterSave::Quit);
            }
            (Prompt::UnsavedQuit, Answer::Picked('d')) => {
                self.quit_discarded.push(self.tabs.active().id);
                self.continue_quit();
            }
            (Prompt::UnsavedQuit, _) => self.quit_discarded.clear(),
            (Prompt::UnsavedClose, Answer::Picked('s')) => {
                self.save_or_ask(AfterSave::Close);
            }
            (Prompt::UnsavedClose, Answer::Picked('d')) => self.close_active(),
            (Prompt::UnsavedClose, _) => {}
            (Prompt::Trash, Answer::Picked('y')) => self.trash_pending(),
            (Prompt::Trash, _) => self.pending_trash = None,
            (Prompt::ProjectReplace, Answer::Picked('r')) => self.replace_in_project(),
            (Prompt::ProjectReplace, _) => self.pending_replace = None,
            (Prompt::Recover, Answer::Picked('r')) => {
                if let Some(text) = self.recovery.take() {
                    self.buffer_mut().recover(&text);
                    self.follow_cursor();
                }
            }
            (Prompt::Recover, Answer::Picked('d')) => {
                self.recovery = None;
                self.delete_backup();
            }
            // Esc would leave the backup's fate open; quitting later would then
            // delete it unasked. Only an explicit answer closes this one.
            (Prompt::Recover, _) => self.ask(Prompt::Recover),
        }
    }

    /// Saves the active buffer, or asks for a path first when it's untitled; then
    /// does `after`. A failed save leaves everything open with the error showing.
    fn save_or_ask(&mut self, after: AfterSave) {
        if self.buffer().path.is_none() {
            self.start_save_as(after);
        } else {
            let saved = self.save();
            self.after_save(after, saved);
        }
    }

    /// Ends the session cleanly: every tab was saved or its changes discarded, so
    /// no backup is needed.
    fn quit(&mut self) {
        // The runtime waits for blocking threads on the way out.
        self.stop_project_search();
        if let Some(tree) = self.run_tree.take() {
            tree.kill();
        }
        for doc in &self.tabs.docs {
            let _ = self.backups.delete(doc.buffer.path.as_deref(), doc.id);
        }
        self.should_quit = true;
    }

    /// Starts writing a crash backup of each tab edited since its last one that
    /// still has unsaved changes. Writes run on a blocking thread with a copy of
    /// the text (ADR-0001).
    fn back_up(&mut self) {
        let mut jobs = Vec::new();
        for doc in &mut self.tabs.docs {
            if !std::mem::take(&mut doc.backup_due) || !doc.buffer.dirty {
                continue;
            }
            jobs.extend(self.backups.job(
                doc.buffer.path.as_deref(),
                doc.id,
                doc.buffer.disk_text(),
            ));
        }
        for job in jobs {
            match &self.events {
                Some(events) => {
                    let events = events.clone();
                    tokio::task::spawn_blocking(move || {
                        // The loop has stopped if this fails; nobody is left to tell.
                        let _ = events.send(AppEvent::BackupWritten(job.run()));
                    });
                }
                None => self.backup_written(job.run()),
            }
        }
    }

    fn backup_written(&mut self, result: io::Result<()>) {
        match result {
            Ok(()) => self.backup_failed = false,
            Err(err) if !self.backup_failed => {
                self.backup_failed = true;
                self.say(
                    Tone::Err,
                    format!("cannot back up {}: {err}", self.buffer().name()),
                );
            }
            Err(_) => {}
        }
    }

    /// Removes the active tab's backup.
    fn delete_backup(&mut self) {
        // Best effort: a leftover backup is only offered again if it is newer than
        // the file and differs from it, so failing here loses nothing.
        let doc = self.tabs.active();
        let _ = self.backups.delete(doc.buffer.path.as_deref(), doc.id);
    }

    /// Saves the active buffer and says how it went in the status line. Returns
    /// whether the buffer is now saved.
    fn save(&mut self) -> bool {
        self.save_doc(self.tabs.active().id, "")
    }

    /// Saves buffer `id`, saying how it went with `note` after the name. Returns
    /// whether the buffer is now saved; a buffer closed meanwhile isn't.
    fn save_doc(&mut self, id: u64, note: &str) -> bool {
        let Some(doc) = self.tabs.docs.iter_mut().find(|doc| doc.id == id) else {
            return false;
        };
        let name = doc.buffer.name();
        match doc.buffer.save() {
            Ok(()) => {
                // Best effort, as in `delete_backup`.
                let _ = self.backups.delete(doc.buffer.path.as_deref(), id);
                self.say(Tone::Ok, format!("saved {name}{note}"));
                true
            }
            Err(err) => {
                self.say(Tone::Err, format!("cannot save {name}: {err}"));
                false
            }
        }
    }

    fn save_unformatted(&mut self, id: u64, why: &str) {
        self.save_doc(id, &format!(" unformatted ({why})"));
    }

    /// Ctrl+S on a buffer with a path. With `format_on_save` for its language the
    /// write waits for the server's formatting, which `formatted` or the timer
    /// carries on with; the loop keeps running meanwhile (ADR-0001). Only Ctrl+S
    /// formats: a save answering a close or quit prompt goes straight to disk.
    fn save_formatted(&mut self) {
        let doc = self.tabs.active().id;
        if self.formatting.is_some_and(|p| p.doc == doc) {
            return;
        }
        if let Some(earlier) = self.formatting.take() {
            // One request at a time; the earlier file mustn't stay unsaved.
            self.lsp.cancel_format();
            self.save_unformatted(earlier.doc, "saving another file");
        }
        let (tab_width, insert_spaces) = (self.editor.tab_width, self.editor.insert_spaces);
        match self.lsp.format_on_save(doc, tab_width, insert_spaces) {
            FormatRequest::Off => {
                self.save();
            }
            FormatRequest::Unavailable(why) => self.save_unformatted(doc, &why),
            FormatRequest::Sent => {
                self.formats += 1;
                let format = self.formats;
                self.formatting = Some(PendingFormat {
                    format,
                    doc,
                    revision: self.buffer().revision,
                });
                self.say(Tone::Ok, format!("formatting {}", self.buffer().name()));
                if let Some(events) = self.events.clone() {
                    tokio::spawn(async move {
                        tokio::time::sleep(FORMAT_TIMEOUT).await;
                        // The loop has stopped if this fails; nothing is waiting.
                        let _ = events.send(AppEvent::FormatTimedOut { format });
                    });
                }
            }
        }
    }

    /// The formatting reply for buffer `id`: applies its edits as one undo step,
    /// then saves. A failed reply, or a buffer edited since the request, saves
    /// the text as it is.
    fn formatted(&mut self, id: u64, edits: Result<Vec<(Range<usize>, String)>, String>) {
        let Some(pending) = self.formatting.take_if(|p| p.doc == id) else {
            return;
        };
        let edits = match edits {
            Ok(edits) => edits,
            Err(why) => return self.save_unformatted(id, &format!("server error: {why}")),
        };
        let Some(doc) = self.tabs.docs.iter_mut().find(|doc| doc.id == id) else {
            return;
        };
        if doc.buffer.revision != pending.revision {
            return self.save_unformatted(id, "edited while formatting");
        }
        doc.buffer.apply_edits(&edits);
        // Other views of the buffer keep their places, edit by edit from the end.
        for (range, text) in edits.iter().rev() {
            let delta = text.chars().count().cast_signed() - range.len().cast_signed();
            if delta != 0 {
                self.tabs.shift_others(id, range.start, delta);
            }
        }
        if self.tabs.active().id == id {
            self.follow_cursor();
        }
        self.save_doc(id, "");
    }

    fn edit(&mut self, edit: impl FnOnce(&mut Buffer)) {
        let doc = self.tabs.active_mut();
        let before = doc
            .buffer
            .selection()
            .map_or(doc.buffer.cursor, |s| s.start);
        let len = doc.buffer.rope.len_chars();
        edit(&mut doc.buffer);
        doc.backup_due = true;
        // Edits happen at or before the cursor, wherever it ends up; undo moves it
        // to the change, so this lands close enough for the other split's place.
        let at = before.min(doc.buffer.cursor);
        let delta = doc.buffer.rope.len_chars().cast_signed() - len.cast_signed();
        let id = doc.id;
        if delta != 0 {
            self.tabs.shift_others(id, at, delta);
        }
        self.follow_cursor();
        if let Some(edits) = &self.edits {
            // The timer only stops with the loop, after which nothing edits.
            let _ = edits.send(());
        }
    }

    /// Moves the cursor to the start of the next (or previous) diagnostic in the
    /// active buffer, wrapping around; with none, it stays put.
    fn jump_to_diagnostic(&mut self, forward: bool) {
        let cursor = self.buffer().cursor;
        let Some(target) = diagnostic_jump(&self.tabs.active().diagnostics, cursor, forward) else {
            return;
        };
        self.buffer_mut().set_caret(Caret {
            cursor: target,
            goal_col: None,
            anchor: None,
        });
        self.follow_cursor();
    }

    /// `buffer`'s path as the status bar shows it: relative to the project root
    /// when it lies inside, so the directory part says where in the project it is.
    fn shown_path(&self, buffer: &Buffer) -> String {
        let Some(path) = buffer.path.as_deref() else {
            return buffer.name();
        };
        // Absolute on both sides, so a root of "." or a path given with `..`
        // still compare.
        match absolute(path).strip_prefix(absolute(self.tree.root())) {
            Ok(relative) => relative.display().to_string(),
            Err(_) => buffer.name(),
        }
    }

    /// Shows `text` in the status bar's path slot until the next key.
    fn say(&mut self, tone: Tone, text: impl Into<String>) {
        self.message = Some(text.into());
        self.message_tone = tone;
    }

    /// Whether a prompt, picker, bar or panel is taking the keys.
    fn modal_open(&self) -> bool {
        self.prompt.is_some()
            || self.name_prompt.is_some()
            || self.picker.is_some()
            || self.folders.is_some()
            || self.project_search.is_some()
            || self.find.is_some()
    }

    /// Asks the active buffer's language server for the definition of the symbol
    /// at the cursor. The answer arrives later as an `AppEvent::Lsp`.
    fn request_definition(&mut self) {
        let doc = self.tabs.active().id;
        let cursor = self.buffer().cursor;
        if !self.lsp.definition(doc, cursor) {
            self.say(Tone::Warn, "No definition found");
        }
    }

    /// Asks the active buffer's language server about the symbol at the cursor.
    /// The answer arrives later as an `AppEvent::Lsp`; with no server, or no
    /// answer, nothing shows.
    fn request_hover(&mut self) {
        let doc = self.tabs.active().id;
        let cursor = self.buffer().cursor;
        if self.lsp.hover(doc, cursor) {
            self.hover_for = Some((doc, cursor));
        }
    }

    /// Goes to `location`, opening its file in a tab first when it isn't the
    /// active one, and remembers where the cursor was for Alt+Left.
    fn go_to(&mut self, location: &Location) {
        let from = Jump {
            doc: self.tabs.active().id,
            path: self.buffer().path.clone(),
            cursor: self.buffer().cursor,
        };
        self.open(&location.path);
        // `open` says why in the status line when the file can't be read.
        if self.tabs.find(&location.path) != Some(self.tabs.active().id) {
            return;
        }
        let cursor = char_index(&self.buffer().rope, location.position);
        self.place_caret(cursor);
        if self.jumps.len() == JUMP_LIST_LEN {
            self.jumps.remove(0);
        }
        self.jumps.push(from);
    }

    /// Returns to the place the last go to definition left, reopening its file
    /// if its tab was closed since.
    fn jump_back(&mut self) {
        let Some(jump) = self.jumps.pop() else {
            return;
        };
        if self.tabs.docs.iter().any(|doc| doc.id == jump.doc) {
            self.tabs.show(jump.doc);
            self.reset_mouse();
            self.focus = Focus::Editor;
        } else if let Some(path) = &jump.path {
            self.open(path);
            if self.tabs.find(path) != Some(self.tabs.active().id) {
                return;
            }
        } else {
            return;
        }
        // The text may have shrunk since.
        let cursor = jump.cursor.min(self.buffer().rope.len_chars());
        self.place_caret(cursor);
    }

    /// Moves the cursor to `cursor`, dropping any selection, and scrolls to it.
    fn place_caret(&mut self, cursor: usize) {
        self.buffer_mut().set_caret(Caret {
            cursor,
            goal_col: None,
            anchor: None,
        });
        self.follow_cursor();
    }

    /// Keeps every split's cursor in view; the other split may show the buffer
    /// that just changed.
    fn follow_cursor(&mut self) {
        let areas: Vec<Rect> = self.panes().splits.iter().map(|s| s.editor).collect();
        self.tabs.follow(&areas, self.editor.tab_width);
    }

    fn panes(&self) -> Panes {
        Panes::new(
            self.screen,
            self.tree_visible,
            self.run_panel_visible,
            self.name_prompt.is_some() || self.find.is_some(),
            self.tabs.splits.len(),
        )
    }

    /// The focused split's editor pane.
    fn editor_area(&self) -> Rect {
        self.panes().splits[self.tabs.focused].editor
    }

    /// Draws the whole screen. Pure: reads `self`, never changes it.
    pub fn render(&self, frame: &mut Frame) {
        let panes = Panes::new(
            frame.area(),
            self.tree_visible,
            self.run_panel_visible,
            self.name_prompt.is_some() || self.find.is_some(),
            self.tabs.splits.len(),
        );
        let theme = &self.theme;
        // The editor ground; chrome paints its own `sidebar_bg` over it.
        let screen = frame.area();
        frame
            .buffer_mut()
            .set_style(screen, Style::new().bg(theme.bg).fg(theme.fg));
        let focused = self.tabs.focused;
        // The focused split draws last, so the terminal cursor ends up in it.
        let order = (0..panes.splits.len())
            .filter(|&split| split != focused)
            .chain(std::iter::once(focused));
        for split in order {
            let area = panes.splits[split];
            let tabs = &self.tabs.splits[split];
            render_tabs(
                theme,
                &self.tabs.labels(split),
                tabs.active,
                split == focused,
                area.header,
                frame,
            );
            // Matches belong to the active buffer, which the focused split shows.
            let (highlights, current) = match &self.find {
                Some(find) if split == focused => (find.matches(), find.current_match()),
                _ => (&[][..], None),
            };
            render_buffer(
                theme,
                self.tabs.shown(split).buffer(),
                &tabs.active().view,
                self.editor.tab_width,
                Marks {
                    highlights,
                    current,
                    diagnostics: &self.tabs.doc(tabs.active().doc).diagnostics,
                },
                split == focused,
                area.editor,
                frame,
            );
        }
        if let Some(divider) = panes.split_divider {
            render_divider(theme, divider, frame);
        }
        if let Some(tree) = panes.tree {
            let focused = self.focus == Focus::Tree;
            let active = self.buffer().path.as_deref().map(absolute);
            let dirty: Vec<PathBuf> = self
                .tabs
                .docs
                .iter()
                .filter(|doc| doc.buffer.dirty)
                .filter_map(|doc| doc.buffer.path.as_deref().map(absolute))
                .collect();
            let marks = TreeMarks {
                project: &self.project,
                active: active.as_deref(),
                dirty: &dirty,
            };
            let selected = render_tree(theme, &self.tree, &marks, focused, tree, frame);
            // The cursor marks the focused pane; `render_buffer` put it in the editor.
            if focused && let Some(at) = selected {
                frame.set_cursor_position(at);
            }
        }
        if let Some(area) = panes.run {
            render_run_panel(theme, self.run.as_ref(), area, frame);
        }
        let buffer = self.buffer();
        let diagnostics = &self.tabs.active().diagnostics;
        let count = |severity| {
            diagnostics
                .iter()
                .filter(|d| d.severity == severity)
                .count()
        };
        // What the last key did comes first; otherwise the diagnostic under the
        // cursor says what's wrong there.
        let message = match &self.message {
            Some(text) => Some((self.message_tone, text.as_str())),
            None => diagnostic_at(diagnostics, buffer.cursor).map(|d| {
                // The bar has no glyph for info or a hint; they're advice.
                let tone = match d.severity {
                    Severity::Error => Tone::Err,
                    _ => Tone::Warn,
                };
                (tone, d.message.as_str())
            }),
        };
        let language = buffer
            .path
            .as_deref()
            .and_then(languages::for_path)
            .map_or("Plain text", |lang| lang.display_name());
        let server = self.lsp.server_state(self.tabs.active().id);
        render_status(
            frame,
            theme,
            panes.status,
            &Status {
                path: &self.shown_path(buffer),
                dirty: buffer.dirty,
                message,
                position: buffer.cursor_line_col(),
                language,
                server: server.as_ref().map(|(state, name)| (*state, name.as_str())),
                warnings: count(Severity::Warning),
                errors: count(Severity::Error),
            },
        );
        if let Some(text) = &self.hover
            && !self.modal_open()
        {
            let area = panes.splits[focused].editor;
            let view = &self.tabs.splits[focused].active().view;
            let shown = self.tabs.shown(focused);
            // Over everything but the status line, which stays readable.
            let bounds = Rect {
                height: screen.height.saturating_sub(1),
                ..screen
            };
            if let Some(at) = cursor_cell(shown.buffer(), view, self.editor.tab_width, area) {
                render_hover(theme, text, at, bounds, frame);
            }
        }
        if let Some(open) = &self.completion
            && open.doc == self.tabs.active().id
            && !self.modal_open()
        {
            let area = panes.splits[focused].editor;
            let view = &self.tabs.splits[focused].active().view;
            let shown = self.tabs.shown(focused);
            let buffer = shown.buffer();
            let bounds = Rect {
                height: screen.height.saturating_sub(1),
                ..screen
            };
            if let Some(typed) = completion::typed_word(&buffer.rope, open.start, buffer.cursor)
                && let Some(at) = cursor_cell(buffer, view, self.editor.tab_width, area)
            {
                open.render(theme, &typed, at, bounds, frame);
            }
        }
        if let (Some(name_prompt), Some(area)) = (&self.name_prompt, panes.bar) {
            let at = name_prompt
                .bar
                .render(theme, frame, area, name_prompt.op.hint());
            frame.set_cursor_position(at);
        }
        if let (Some(find), Some(area)) = (&self.find, panes.bar) {
            let at = find.render(theme, frame, area);
            frame.set_cursor_position(at);
        }
        if let Some(picker) = &self.picker {
            let at = picker.render(theme, frame, frame.area());
            frame.set_cursor_position(at);
        }
        if let Some(browser) = &self.folders {
            let at = browser.render(theme, frame, frame.area());
            frame.set_cursor_position(at);
        }
        if let Some(panel) = &self.project_search {
            let at = panel.render(theme, frame, frame.area());
            frame.set_cursor_position(at);
        }
        if let Some(prompt) = self.prompt {
            self.confirm(prompt)
                .render(theme, frame, frame.area(), self.prompt_button);
        }
    }
}

/// `count` and the noun that goes with it, e.g. `1 file` or `2 files`.
fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// The last part of `path`, for messages and tab names.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// One editor split's place on screen: its tab header (a blank row, the pills and
/// the aurora thread) and the editor below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SplitArea {
    header: Rect,
    editor: Rect,
}

/// Where each part of the screen goes: the tree (when shown) from row 0 down, a
/// blank column, then the editor splits, each with its tab header on rows 0-2 and a `│`
/// between the two; below them the run panel (while shown, the full width and
/// about 30% of the height), the prompt bar (while open) and a one-row status
/// line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Panes {
    /// The whole column, brand rows included (see `nodes_area`).
    tree: Option<Rect>,
    /// One per split, left to right.
    splits: Vec<SplitArea>,
    /// Between the two splits, while there are two.
    split_divider: Option<Rect>,
    run: Option<Rect>,
    bar: Option<Rect>,
    status: Rect,
}

impl Panes {
    fn new(
        screen: Rect,
        tree_visible: bool,
        run_visible: bool,
        bar_open: bool,
        splits: usize,
    ) -> Self {
        let run_height = if run_visible {
            (screen.height * 3 / 10).max(2)
        } else {
            0
        };
        let [main, run, bar, status] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(run_height),
            Constraint::Length(u16::from(bar_open)),
            Constraint::Length(1),
        ])
        .areas(screen);
        let run = run_visible.then_some(run);
        let bar = bar_open.then_some(bar);
        let (tree, right) = if tree_visible {
            // No divider: the column after the tree is editor ground, which the
            // tree's `surface` stands out against (README §2).
            let [tree, _, right] = Layout::horizontal([
                Constraint::Length(TREE_WIDTH),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .areas(main);
            (Some(tree), right)
        } else {
            (None, main)
        };
        let (columns, split_divider) = if splits > 1 {
            let [left, divider, right] = Layout::horizontal([
                Constraint::Fill(1),
                Constraint::Length(1),
                Constraint::Fill(1),
            ])
            .areas(right);
            (vec![left, right], Some(divider))
        } else {
            (vec![right], None)
        };
        let splits = columns
            .into_iter()
            .map(|column| {
                let [header, editor] =
                    Layout::vertical([Constraint::Length(HEADER_HEIGHT), Constraint::Min(0)])
                        .areas(column);
                SplitArea { header, editor }
            })
            .collect();
        Panes {
            tree,
            splits,
            split_divider,
            run,
            bar,
            status,
        }
    }
}

async fn read_input(tx: mpsc::UnboundedSender<AppEvent>) {
    let mut events = EventStream::new();
    while let Some(Ok(first)) = events.next().await {
        for event in with_burst(&mut events, first).await {
            if tx.send(AppEvent::Input(event)).is_err() {
                return;
            }
        }
    }
}

/// `first` plus every event already waiting behind it, which arrived together. On
/// Windows crossterm never reports bracketed paste, so a paste shows up as such a
/// burst of keys and is turned back into one `Event::Paste` (see `burst_as_paste`).
#[cfg(windows)]
async fn with_burst(events: &mut EventStream, first: Event) -> Vec<Event> {
    let mut burst = vec![first];
    // A zero timeout polls once with this task's waker; `now_or_never` would poll
    // with a dummy one that crossterm keeps, and input would stall.
    while let Ok(Some(Ok(event))) = timeout(Duration::ZERO, events.next()).await {
        burst.push(event);
    }
    match burst_as_paste(&burst) {
        Some(text) => vec![Event::Paste(text)],
        None => burst,
    }
}

/// Elsewhere terminals deliver pastes as `Event::Paste` already.
#[cfg(not(windows))]
async fn with_burst(_events: &mut EventStream, first: Event) -> Vec<Event> {
    vec![first]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::clipboard::FakeClipboard;
    use crate::config::{KeyBinding, KeysConfig};
    use crate::keymap::key_event;

    fn key(notation: &str) -> AppEvent {
        AppEvent::Input(Event::Key(key_event(notation)))
    }

    #[test]
    fn ctrl_q_quits() {
        let mut app = App::default();
        app.handle_event(key("ctrl+q"));
        assert!(app.should_quit);
    }

    #[test]
    fn ctrl_q_with_unsaved_changes_asks_first() {
        let mut app = App::default();
        app.handle_event(key("x"));
        app.handle_event(key("ctrl+q"));
        assert!(!app.should_quit);
        assert_eq!(app.prompt, Some(Prompt::UnsavedQuit));
        // Keys answer the prompt instead of editing.
        app.handle_event(key("y"));
        assert_eq!(app.buffer().rope.to_string(), "x");
        app.handle_event(key("esc"));
        assert_eq!(app.prompt, None);
        app.handle_event(key("ctrl+q"));
        app.handle_event(key("c"));
        assert_eq!(app.prompt, None);
        assert!(!app.should_quit);
        app.handle_event(key("ctrl+q"));
        app.handle_event(key("shift+d"));
        assert!(app.should_quit);
    }

    #[test]
    fn save_then_quit_on_an_untitled_buffer_asks_for_a_path() {
        let mut app = App::default();
        app.handle_event(key("x"));
        app.handle_event(key("ctrl+q"));
        app.handle_event(key("s"));
        assert!(!app.should_quit);
        let label = app.name_prompt.as_ref().map(|p| p.bar.label);
        assert_eq!(label, Some("Save as"));
    }

    #[test]
    fn ctrl_s_saves_and_clears_dirty() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "hi\r\n")?;
        let mut app = App::new(
            Keymap::default(),
            EditorConfig::default(),
            Some(path.clone()),
            None,
        );
        app.handle_event(key("x"));
        assert!(app.buffer().dirty);
        app.handle_event(key("ctrl+s"));
        assert!(!app.buffer().dirty);
        assert_eq!(std::fs::read_to_string(&path)?, "xhi\r\n");
        let message = app.message.unwrap_or_default();
        assert!(message.starts_with("saved "), "{message}");
        Ok(())
    }

    #[test]
    fn plain_q_does_not_quit() {
        let mut app = App::default();
        app.handle_event(key("q"));
        assert!(!app.should_quit);
    }

    #[test]
    fn key_release_does_not_quit() {
        let mut app = App::default();
        let mut release = key_event("ctrl+q");
        release.kind = KeyEventKind::Release;
        app.handle_event(AppEvent::Input(Event::Key(release)));
        assert!(!app.should_quit);
    }

    #[test]
    fn a_key_release_leaves_the_hover_open() {
        let mut app = App {
            hover: Some(HoverText {
                code: "pub fn greet()".into(),
                docs: String::new(),
            }),
            ..App::default()
        };
        let mut release = key_event("alt+k");
        release.kind = KeyEventKind::Release;
        app.handle_event(AppEvent::Input(Event::Key(release)));
        assert!(app.hover.is_some());
        app.handle_event(key("esc"));
        assert_eq!(app.hover, None);
    }

    #[test]
    fn keys_go_through_the_configured_keymap() -> Result<()> {
        let mut keys = KeysConfig::new();
        keys.insert("quit".into(), KeyBinding::One("alt+q".into()));
        let mut app = App::new(Keymap::new(&keys)?, EditorConfig::default(), None, None);
        app.handle_event(key("ctrl+q"));
        assert!(!app.should_quit);
        app.handle_event(key("alt+q"));
        assert!(app.should_quit);
        Ok(())
    }

    #[test]
    fn movement_keys_move_the_cursor_and_scroll_the_view() {
        let text: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        let mut app = App {
            tabs: Tabs::new(Buffer {
                rope: ropey::Rope::from_str(&text),
                ..Buffer::empty()
            }),
            screen: Rect::new(0, 0, 100, 30),
            ..App::default()
        };
        app.handle_event(key("pagedown"));
        // The editor pane is 26 rows: the tab header takes three, the status line one.
        assert_eq!(app.buffer().cursor_line_col(), (26, 0));
        assert_eq!(app.view().scroll_row, 1);
        app.handle_event(key("ctrl+end"));
        assert_eq!(app.buffer().cursor_line_col(), (100, 0));
        assert_eq!(app.view().scroll_row, 75);
        app.handle_event(key("ctrl+home"));
        assert_eq!(app.buffer().cursor, 0);
        assert_eq!(app.view().scroll_row, 0);
    }

    #[test]
    fn editing_keys_edit_the_buffer() {
        let mut app = App {
            screen: Rect::new(0, 0, 100, 30),
            ..App::default()
        };
        for k in [
            "f",
            "n",
            "space",
            "{",
            "enter",
            "tab",
            "y",
            "x",
            "backspace",
            "delete",
        ] {
            app.handle_event(key(k));
        }
        assert_eq!(app.buffer().rope.to_string(), "fn {\n    y");
        assert!(app.buffer().dirty);
    }

    #[test]
    fn ctrl_z_and_ctrl_y_undo_and_redo() {
        let mut app = App {
            screen: Rect::new(0, 0, 100, 30),
            ..App::default()
        };
        for k in ["a", "b", "ctrl+c", "c", "d"] {
            app.handle_event(key(k));
        }
        // Copying is a non-typing action, so it split the run.
        app.handle_event(key("ctrl+z"));
        assert_eq!(app.buffer().rope.to_string(), "ab");
        app.handle_event(key("ctrl+z"));
        assert_eq!(app.buffer().rope.to_string(), "");
        app.handle_event(key("ctrl+y"));
        assert_eq!(app.buffer().rope.to_string(), "ab");
        assert_eq!(app.buffer().cursor, 2);
    }

    fn app_with(text: &str, clipboard: &FakeClipboard) -> App {
        App {
            tabs: Tabs::new(Buffer {
                rope: ropey::Rope::from_str(text),
                ..Buffer::empty()
            }),
            screen: Rect::new(0, 0, 100, 30),
            clipboard: Box::new(clipboard.clone()),
            ..App::default()
        }
    }

    fn press(app: &mut App, keys: &[&str]) {
        for k in keys {
            app.handle_event(key(k));
        }
    }

    #[test]
    fn shift_right_then_typing_replaces_the_selection() {
        let mut app = app_with("hello world", &FakeClipboard::default());
        press(&mut app, &["shift+right"; 5]);
        assert_eq!(app.buffer().selected_text().as_deref(), Some("hello"));
        press(&mut app, &["b", "y", "e"]);
        assert_eq!(app.buffer().rope.to_string(), "bye world");
        press(&mut app, &["ctrl+z"]);
        assert_eq!(app.buffer().rope.to_string(), "hello world");
    }

    #[test]
    fn a_plain_movement_clears_the_selection() {
        let mut app = app_with("hello", &FakeClipboard::default());
        press(&mut app, &["ctrl+shift+right", "left"]);
        assert_eq!(app.buffer().selection(), None);
        assert_eq!(app.buffer().cursor, 4);
    }

    #[test]
    fn ctrl_c_copies_the_selection_and_nothing_without_one() {
        let clipboard = FakeClipboard::default();
        *clipboard.text.borrow_mut() = "old".into();
        let mut app = app_with("hello world", &clipboard);
        press(&mut app, &["ctrl+c"]);
        assert_eq!(*clipboard.text.borrow(), "old");
        press(&mut app, &["ctrl+shift+right", "ctrl+c"]);
        assert_eq!(*clipboard.text.borrow(), "hello");
        assert_eq!(app.buffer().rope.to_string(), "hello world");
        assert!(!app.buffer().dirty);
    }

    #[test]
    fn ctrl_x_copies_and_deletes_as_one_step() {
        let clipboard = FakeClipboard::default();
        let mut app = app_with("one\ntwo", &clipboard);
        press(&mut app, &["ctrl+a", "ctrl+x"]);
        assert_eq!(*clipboard.text.borrow(), "one\ntwo");
        assert_eq!(app.buffer().rope.to_string(), "");
        press(&mut app, &["ctrl+z"]);
        assert_eq!(app.buffer().rope.to_string(), "one\ntwo");
        // Nothing selected: cut leaves the clipboard and the text alone.
        press(&mut app, &["ctrl+x"]);
        assert_eq!(app.buffer().rope.to_string(), "one\ntwo");
    }

    #[test]
    fn ctrl_v_pastes_normalised_text_as_one_step() {
        let clipboard = FakeClipboard::default();
        *clipboard.text.borrow_mut() = "a\r\nb\r\n".into();
        let mut app = app_with("xy", &clipboard);
        press(&mut app, &["right", "ctrl+v"]);
        assert_eq!(app.buffer().rope.to_string(), "xa\nb\ny");
        press(&mut app, &["ctrl+z"]);
        assert_eq!(app.buffer().rope.to_string(), "xy");
        // Pasting over a selection replaces it, still one step.
        press(&mut app, &["ctrl+a", "ctrl+v"]);
        assert_eq!(app.buffer().rope.to_string(), "a\nb\n");
        press(&mut app, &["ctrl+z"]);
        assert_eq!(app.buffer().rope.to_string(), "xy");
    }

    #[test]
    fn copy_then_paste_round_trips_through_the_clipboard() {
        let clipboard = FakeClipboard::default();
        let mut app = app_with("ab", &clipboard);
        press(
            &mut app,
            &["shift+right", "ctrl+c", "end", "ctrl+v", "ctrl+v"],
        );
        assert_eq!(app.buffer().rope.to_string(), "abaa");
    }

    #[test]
    fn bracketed_paste_is_one_step_replacing_the_selection() {
        let mut app = app_with("hello world", &FakeClipboard::default());
        press(&mut app, &["shift+end"]);
        app.handle_event(AppEvent::Input(Event::Paste("bye\r\nnow".into())));
        assert_eq!(app.buffer().rope.to_string(), "bye\nnow");
        press(&mut app, &["ctrl+z"]);
        assert_eq!(app.buffer().rope.to_string(), "hello world");
    }

    #[test]
    fn bracketed_paste_is_ignored_while_a_prompt_is_open() {
        let mut app = app_with("", &FakeClipboard::default());
        press(&mut app, &["x", "ctrl+q"]);
        app.handle_event(AppEvent::Input(Event::Paste("d".into())));
        assert_eq!(app.buffer().rope.to_string(), "x");
        assert!(!app.should_quit);
    }

    fn mouse(kind: MouseEventKind, col: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: col,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    const LEFT_DOWN: MouseEventKind = MouseEventKind::Down(MouseButton::Left);
    const LEFT_UP: MouseEventKind = MouseEventKind::Up(MouseButton::Left);

    fn click(app: &mut App, col: u16, row: u16, at: Instant) {
        app.handle_mouse(mouse(LEFT_DOWN, col, row), at);
        app.handle_mouse(mouse(LEFT_UP, col, row), at);
    }

    #[test]
    fn click_places_the_cursor_past_the_gutter() {
        // Gutter "   1  " is 6 cells; the tab header takes rows 0-2.
        let mut app = app_with(
            "hello
world",
            &FakeClipboard::default(),
        );
        click(&mut app, 8, 4, Instant::now());
        assert_eq!(app.buffer().cursor_line_col(), (1, 2));
        assert_eq!(app.buffer().selection(), None);
        // The status line isn't the editor.
        click(&mut app, 5, 29, Instant::now());
        assert_eq!(app.buffer().cursor_line_col(), (1, 2));
    }

    #[test]
    fn double_click_needs_the_same_cell_within_400_ms() {
        let mut app = app_with("hello world", &FakeClipboard::default());
        let t0 = Instant::now();
        click(&mut app, 7, 3, t0);
        click(&mut app, 7, 3, t0 + Duration::from_millis(400));
        assert_eq!(app.buffer().selected_text().as_deref(), Some("hello"));

        // Too slow: a second plain click.
        let t1 = t0 + Duration::from_secs(5);
        click(&mut app, 13, 3, t1);
        click(&mut app, 13, 3, t1 + Duration::from_millis(401));
        assert_eq!(app.buffer().selection(), None);
        assert_eq!(app.buffer().cursor, 7);

        // Another cell: a plain click.
        let t2 = t1 + Duration::from_secs(5);
        click(&mut app, 13, 3, t2);
        click(&mut app, 14, 3, t2 + Duration::from_millis(10));
        assert_eq!(app.buffer().selection(), None);

        // A third quick click is a plain click again.
        let t3 = t2 + Duration::from_secs(5);
        for i in 0..3 {
            click(&mut app, 7, 3, t3 + Duration::from_millis(i * 10));
        }
        assert_eq!(app.buffer().selection(), None);
        assert_eq!(app.buffer().cursor, 1);
    }

    #[test]
    fn drag_selects_from_press_to_release() {
        let mut app = app_with("hello world", &FakeClipboard::default());
        let now = Instant::now();
        app.handle_mouse(mouse(LEFT_DOWN, 12, 3), now);
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 8, 3), now);
        app.handle_mouse(mouse(LEFT_UP, 8, 3), now);
        assert_eq!(app.buffer().selected_text().as_deref(), Some("llo "));
        assert_eq!(app.buffer().cursor, 2);
        // A drag without a press in the editor selects nothing.
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 16, 3), now);
        assert_eq!(app.buffer().cursor, 2);
    }

    #[test]
    fn ctrl_click_places_the_cursor_and_asks_for_the_definition() {
        let mut app = app_with("hello", &FakeClipboard::default());
        let mut event = mouse(LEFT_DOWN, 9, 3);
        event.modifiers = KeyModifiers::CONTROL;
        app.handle_mouse(event, Instant::now());
        assert_eq!(app.buffer().cursor, 3);
        // No drag starts, and without a server there's nothing to go to.
        assert!(app.drag_from.is_none());
        assert_eq!(app.message.as_deref(), Some("No definition found"));
    }

    #[test]
    fn the_jump_list_keeps_the_last_50_places() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "x".repeat(100))?;
        let mut app = App::new(
            Keymap::default(),
            EditorConfig::default(),
            Some(path.clone()),
            None,
        );
        for i in 0..60 {
            app.place_caret(i);
            app.go_to(&Location {
                path: path.clone(),
                position: lsp_types::Position::new(0, 99),
            });
        }
        assert_eq!(app.buffer().cursor, 99);
        assert_eq!(app.jumps.len(), JUMP_LIST_LEN);
        app.handle_action(Action::JumpBack);
        assert_eq!(app.buffer().cursor, 59);
        for _ in 0..60 {
            app.handle_action(Action::JumpBack);
        }
        // The oldest ten were forgotten.
        assert_eq!(app.buffer().cursor, 10);
        Ok(())
    }

    #[test]
    fn wheel_scrolls_without_moving_the_cursor() {
        let text: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        let mut app = app_with(&text, &FakeClipboard::default());
        app.handle_mouse(mouse(MouseEventKind::ScrollDown, 10, 10), Instant::now());
        assert_eq!(app.view().scroll_row, 3);
        assert_eq!(app.buffer().cursor, 0);
        app.handle_mouse(mouse(MouseEventKind::ScrollUp, 10, 10), Instant::now());
        app.handle_mouse(mouse(MouseEventKind::ScrollUp, 10, 10), Instant::now());
        assert_eq!(app.view().scroll_row, 0);
    }

    #[test]
    fn mouse_is_ignored_while_a_prompt_is_open() {
        let mut app = app_with("", &FakeClipboard::default());
        press(&mut app, &["x", "x", "ctrl+q"]);
        app.handle_event(AppEvent::Input(Event::Mouse(mouse(LEFT_DOWN, 5, 1))));
        assert_eq!(app.buffer().cursor, 2);
    }

    /// `a.txt` holding `contents` in a temp folder, opened with backups in another.
    struct Backed {
        files: tempfile::TempDir,
        data: tempfile::TempDir,
        app: App,
    }

    impl Backed {
        fn new(contents: &str) -> Result<Self> {
            let files = tempfile::tempdir()?;
            let data = tempfile::tempdir()?;
            std::fs::write(files.path().join("a.txt"), contents)?;
            let app = Self::open(files.path(), data.path());
            Ok(Self { files, data, app })
        }

        fn open(files: &std::path::Path, data: &std::path::Path) -> App {
            App::new(
                Keymap::default(),
                EditorConfig::default(),
                Some(files.join("a.txt")),
                None,
            )
            .with_backups(Backups::new(Some(data.to_path_buf())))
        }

        fn backup(&self) -> PathBuf {
            Backups::new(Some(self.data.path().to_path_buf()))
                .path_for(Some(&self.files.path().join("a.txt")), 0)
                .expect("backups are on")
        }
    }

    #[test]
    fn a_due_backup_writes_the_dirty_buffer_only() -> Result<()> {
        let mut t = Backed::new("hi\r\n")?;
        t.app.handle_event(AppEvent::BackupDue);
        assert!(!t.backup().exists(), "a clean buffer needs no backup");
        press(&mut t.app, &["x"]);
        t.app.handle_event(AppEvent::BackupDue);
        // Same bytes the file would get, line endings included.
        assert_eq!(std::fs::read_to_string(t.backup())?, "xhi\r\n");
        Ok(())
    }

    #[test]
    fn saving_deletes_the_backup() -> Result<()> {
        let mut t = Backed::new("hi\n")?;
        press(&mut t.app, &["x"]);
        t.app.handle_event(AppEvent::BackupDue);
        assert!(t.backup().exists());
        press(&mut t.app, &["ctrl+s"]);
        assert!(!t.backup().exists());
        Ok(())
    }

    #[test]
    fn closing_cleanly_deletes_the_backup() -> Result<()> {
        // Quit after discarding...
        let mut t = Backed::new("hi\n")?;
        press(&mut t.app, &["x"]);
        t.app.handle_event(AppEvent::BackupDue);
        press(&mut t.app, &["ctrl+q", "d"]);
        assert!(t.app.should_quit);
        assert!(!t.backup().exists());

        // ...after saving from the prompt...
        let mut t = Backed::new("hi\n")?;
        press(&mut t.app, &["x"]);
        t.app.handle_event(AppEvent::BackupDue);
        press(&mut t.app, &["ctrl+q", "s"]);
        assert!(t.app.should_quit);
        assert!(!t.backup().exists());

        // ...and a plain quit once the buffer is clean.
        let mut t = Backed::new("hi\n")?;
        press(&mut t.app, &["x"]);
        t.app.handle_event(AppEvent::BackupDue);
        t.app.buffer_mut().dirty = false;
        press(&mut t.app, &["ctrl+q"]);
        assert!(t.app.should_quit);
        assert!(!t.backup().exists());
        Ok(())
    }

    #[test]
    fn a_failed_backup_says_so_once_and_editing_carries_on() -> Result<()> {
        let parent = tempfile::tempdir()?;
        // A file where the data dir should be, so `backups/` can't be created.
        let data = parent.path().join("data");
        std::fs::write(&data, "not a folder")?;
        let mut app = App {
            screen: Rect::new(0, 0, 100, 30),
            ..App::default()
        }
        .with_backups(Backups::new(Some(data)));

        press(&mut app, &["a"]);
        app.handle_event(AppEvent::BackupDue);
        let message = app.message.take().unwrap_or_default();
        assert!(
            message.starts_with("cannot back up untitled: "),
            "{message}"
        );

        // More failures don't repeat it.
        press(&mut app, &["b"]);
        app.handle_event(AppEvent::BackupDue);
        assert_eq!(app.message, None);
        assert_eq!(app.buffer().rope.to_string(), "ab");
        assert_eq!(app.prompt, None);

        // A success resets it, so the next failure is reported again.
        app.handle_event(AppEvent::BackupWritten(Ok(())));
        app.handle_event(AppEvent::BackupWritten(Err(io::Error::other("disk full"))));
        assert_eq!(
            app.message.as_deref(),
            Some("cannot back up untitled: disk full")
        );
        Ok(())
    }

    #[test]
    fn reopening_offers_recover_which_loads_the_backup_dirty() -> Result<()> {
        let mut t = Backed::new("hi\n")?;
        press(&mut t.app, &["x"]);
        t.app.handle_event(AppEvent::BackupDue);
        let backup_time = std::fs::metadata(t.backup())?.modified()?;
        // Make sure the backup reads as newer even on a coarse-clock filesystem.
        std::fs::File::options()
            .write(true)
            .open(t.files.path().join("a.txt"))?
            .set_modified(backup_time - Duration::from_secs(5))?;

        // Crash: the old app just goes away.
        let mut app = Backed::open(t.files.path(), t.data.path());
        assert_eq!(app.prompt, Some(Prompt::Recover));
        assert_eq!(app.buffer().rope.to_string(), "hi\n");
        // Esc doesn't decide for the user.
        press(&mut app, &["esc", "x"]);
        assert_eq!(app.prompt, Some(Prompt::Recover));
        press(&mut app, &["r"]);
        assert_eq!(app.prompt, None);
        assert_eq!(app.buffer().rope.to_string(), "xhi\n");
        assert!(app.buffer().dirty);
        assert!(t.backup().exists(), "still the only copy of the edits");
        Ok(())
    }

    #[test]
    fn discard_deletes_the_backup_and_keeps_the_file() -> Result<()> {
        let mut t = Backed::new("hi\n")?;
        press(&mut t.app, &["x"]);
        t.app.handle_event(AppEvent::BackupDue);
        let backup_time = std::fs::metadata(t.backup())?.modified()?;
        std::fs::File::options()
            .write(true)
            .open(t.files.path().join("a.txt"))?
            .set_modified(backup_time - Duration::from_secs(5))?;

        let mut app = Backed::open(t.files.path(), t.data.path());
        press(&mut app, &["d"]);
        assert_eq!(app.prompt, None);
        assert_eq!(app.buffer().rope.to_string(), "hi\n");
        assert!(!app.buffer().dirty);
        assert!(!t.backup().exists());
        Ok(())
    }

    /// A folder holding `a.txt` ("a") and `b.txt` ("b"), opened as the project.
    fn project() -> Result<(tempfile::TempDir, App)> {
        let dir = tempfile::tempdir()?;
        std::fs::write(dir.path().join("a.txt"), "a")?;
        std::fs::write(dir.path().join("b.txt"), "b")?;
        let mut app = App::new(
            Keymap::default(),
            EditorConfig::default(),
            Some(dir.path().to_path_buf()),
            None,
        );
        app.screen = Rect::new(0, 0, 100, 30);
        Ok((dir, app))
    }

    #[test]
    fn a_folder_opens_with_the_tree_focused_and_typing_does_not_edit() -> Result<()> {
        let (_dir, mut app) = project()?;
        assert!(app.tree_visible);
        assert_eq!(app.focus, Focus::Tree);
        press(&mut app, &["x", "backspace", "ctrl+v"]);
        assert_eq!(app.buffer().rope.to_string(), "");
        assert!(!app.buffer().dirty);
        Ok(())
    }

    fn tab_names(app: &App) -> Vec<String> {
        app.tabs
            .labels(app.tabs.focused)
            .into_iter()
            .map(|label| {
                if label.dirty {
                    format!("{} ●", label.name)
                } else {
                    label.name
                }
            })
            .collect()
    }

    #[test]
    fn opening_a_folder_closes_every_tab_and_shows_its_tree() -> Result<()> {
        let (dir, mut app) = project()?;
        let other = tempfile::tempdir()?;
        std::fs::write(other.path().join("c.txt"), "c")?;
        app.open(&dir.path().join("a.txt"));
        app.handle_action(Action::ToggleSplit);
        app.focus = Focus::Editor;
        app.handle_action(Action::OpenDirectory);
        assert!(app.folders.is_some());
        app.folders_step(Browsed::Open(other.path().to_path_buf()));
        assert!(app.folders.is_none());
        assert_eq!(app.tree.root(), other.path());
        assert_eq!(
            app.project,
            project_label(other.path(), std::env::home_dir().as_deref())
        );
        assert_eq!(app.tabs.splits.len(), 1);
        assert_eq!(tab_names(&app), ["untitled"]);
        assert_eq!(app.tabs.docs.len(), 1);
        assert!(app.tree_visible);
        assert_eq!(app.focus, Focus::Tree);
        assert_eq!(selected_name(&app), Some("c.txt"));
        Ok(())
    }

    #[test]
    fn opening_a_folder_with_unsaved_changes_refuses() -> Result<()> {
        let (dir, mut app) = project()?;
        let other = tempfile::tempdir()?;
        app.open(&dir.path().join("a.txt"));
        app.focus = Focus::Editor;
        type_keys(&mut app, "x");
        app.folders_step(Browsed::Open(other.path().to_path_buf()));
        assert_eq!(app.tree.root(), dir.path());
        assert_eq!(tab_names(&app), ["a.txt ●"]);
        assert_eq!(
            app.message.as_deref(),
            Some("save or close unsaved files first")
        );
        Ok(())
    }

    #[test]
    fn the_picker_takes_keys_from_the_tree_and_opens_its_pick() -> Result<()> {
        let (dir, mut app) = project()?;
        std::fs::create_dir(dir.path().join("sub"))?;
        std::fs::write(dir.path().join("sub").join("deep.txt"), "deep")?;
        press(&mut app, &["ctrl+p"]);
        // Tree letters such as `d` are query text while the picker is open.
        press(&mut app, &["d", "e", "e", "p"]);
        let picker = app.picker.as_ref().expect("picker is open");
        assert_eq!(picker.selected(), Some("sub/deep.txt"));
        press(&mut app, &["enter"]);
        assert!(app.picker.is_none());
        assert_eq!(app.focus, Focus::Editor);
        assert_eq!(app.buffer().rope.to_string(), "deep");
        // A walk that finishes after the picker closed changes nothing.
        app.handle_event(AppEvent::FilesListed {
            walk: app.walks,
            files: vec!["a.txt".into()],
        });
        assert!(app.picker.is_none());
        Ok(())
    }

    #[test]
    fn a_list_from_an_older_walk_is_dropped() -> Result<()> {
        let (_dir, mut app) = project()?;
        press(&mut app, &["ctrl+p"]);
        let stale = app.walks;
        press(&mut app, &["esc", "ctrl+p"]);
        app.handle_event(AppEvent::FilesListed {
            walk: stale,
            files: vec!["gone.txt".into()],
        });
        press(&mut app, &["g", "o", "n", "e"]);
        let picker = app.picker.as_ref().expect("picker is open");
        // Commands match the query too, so only the file is looked for.
        assert_ne!(picker.selected(), Some("gone.txt"));
        Ok(())
    }

    #[test]
    fn opening_from_the_tree_opens_tabs_and_reuses_open_ones() -> Result<()> {
        let (_dir, mut app) = project()?;
        press(&mut app, &["enter"]);
        // The untouched untitled tab made way for the file.
        assert_eq!(tab_names(&app), ["a.txt"]);
        assert_eq!(app.focus, Focus::Editor);
        press(&mut app, &["x", "ctrl+e", "down", "enter"]);
        assert_eq!(app.prompt, None);
        assert_eq!(tab_names(&app), ["a.txt ●", "b.txt"]);
        assert_eq!(app.buffer().rope.to_string(), "b");
        // Opening a.txt again switches back to its tab, edits intact.
        press(&mut app, &["ctrl+e", "up", "enter"]);
        assert_eq!(app.tabs.split().tabs.len(), 2);
        assert_eq!(app.tabs.split().active, 0);
        assert_eq!(app.buffer().rope.to_string(), "xa");
        Ok(())
    }

    #[test]
    fn tab_keys_move_between_tabs_and_wrap() {
        let mut app = app_with("one", &FakeClipboard::default());
        press(&mut app, &["ctrl+n", "2", "ctrl+n", "3"]);
        assert_eq!(app.tabs.split().active, 2);
        press(&mut app, &["alt+."]);
        assert_eq!(app.buffer().rope.to_string(), "one");
        press(&mut app, &["alt+,"]);
        assert_eq!(app.buffer().rope.to_string(), "3");
        press(&mut app, &["alt+2"]);
        assert_eq!(app.buffer().rope.to_string(), "2");
        // No tab 9: nothing happens.
        press(&mut app, &["alt+9"]);
        assert_eq!(app.tabs.split().active, 1);
    }

    #[test]
    fn ctrl_w_closes_clean_tabs_and_asks_about_dirty_ones() {
        let mut app = app_with("", &FakeClipboard::default());
        press(&mut app, &["ctrl+n", "x", "ctrl+n", "alt+2", "ctrl+w"]);
        assert_eq!(app.prompt, Some(Prompt::UnsavedClose));
        press(&mut app, &["c"]);
        assert_eq!(app.tabs.split().tabs.len(), 3);
        press(&mut app, &["ctrl+w", "d"]);
        assert_eq!(app.tabs.split().tabs.len(), 2);
        // The tab to the right took over.
        assert_eq!(app.tabs.split().active, 1);
        press(&mut app, &["ctrl+w", "ctrl+w"]);
        // Closing the last tab leaves a fresh untitled one.
        assert_eq!(tab_names(&app), ["untitled"]);
        assert!(!app.should_quit);
    }

    #[test]
    fn ctrl_q_asks_about_each_dirty_tab_in_turn() -> Result<()> {
        let (dir, mut app) = project()?;
        press(&mut app, &["enter", "x", "ctrl+e", "down", "enter", "y"]);
        press(&mut app, &["ctrl+n", "ctrl+q"]);
        // The first dirty tab comes up first.
        assert_eq!(app.prompt, Some(Prompt::UnsavedQuit));
        assert_eq!(app.tabs.split().active, 0);
        press(&mut app, &["d"]);
        assert_eq!(app.prompt, Some(Prompt::UnsavedQuit));
        assert_eq!(app.tabs.split().active, 1);
        // Cancel stops the whole quit, and a new Ctrl+Q starts over.
        press(&mut app, &["c"]);
        assert!(!app.should_quit);
        press(&mut app, &["ctrl+q"]);
        assert_eq!(app.tabs.split().active, 0);
        press(&mut app, &["d", "s"]);
        assert!(app.should_quit);
        assert_eq!(std::fs::read_to_string(dir.path().join("a.txt"))?, "a");
        assert_eq!(std::fs::read_to_string(dir.path().join("b.txt"))?, "yb");
        Ok(())
    }

    #[test]
    fn save_as_writes_the_new_path_and_renames_the_tab() -> Result<()> {
        let (dir, mut app) = project()?;
        press(&mut app, &["ctrl+n", "h", "i", "alt+s"]);
        let bar = app.name_prompt.as_ref().map(|p| p.bar.text().to_string());
        assert_eq!(bar.as_deref(), Some(""));
        type_keys(&mut app, "docs/new.txt");
        press(&mut app, &["enter"]);
        // No such folder: nothing written, still untitled.
        let message = app.message.clone().unwrap_or_default();
        assert!(message.starts_with("cannot save"), "{message}");
        assert_eq!(app.buffer().path, None);

        press(&mut app, &["alt+s"]);
        type_keys(&mut app, "a.txt");
        press(&mut app, &["enter"]);
        assert_eq!(app.message.as_deref(), Some("a.txt already exists"));
        assert_eq!(std::fs::read_to_string(dir.path().join("a.txt"))?, "a");

        press(&mut app, &["alt+s"]);
        type_keys(&mut app, "new.txt");
        press(&mut app, &["enter"]);
        let new = dir.path().join("new.txt");
        assert_eq!(std::fs::read_to_string(&new)?, "hi");
        assert_eq!(app.buffer().path.as_deref(), Some(new.as_path()));
        assert_eq!(tab_names(&app), ["untitled", "new.txt"]);
        // The bar now offers the current path, relative to the root.
        press(&mut app, &["alt+s"]);
        let bar = app.name_prompt.as_ref().map(|p| p.bar.text().to_string());
        assert_eq!(bar.as_deref(), Some("new.txt"));
        Ok(())
    }

    #[test]
    fn ctrl_s_on_an_untitled_tab_asks_for_a_path_and_quit_waits_for_it() -> Result<()> {
        let (dir, mut app) = project()?;
        press(&mut app, &["ctrl+n", "z", "ctrl+q", "s"]);
        assert!(app.name_prompt.is_some());
        // Esc on the path cancels the quit.
        press(&mut app, &["esc"]);
        assert!(!app.should_quit);
        press(&mut app, &["ctrl+q", "s"]);
        type_keys(&mut app, "z.txt");
        press(&mut app, &["enter"]);
        assert!(app.should_quit);
        assert_eq!(std::fs::read_to_string(dir.path().join("z.txt"))?, "z");
        Ok(())
    }

    fn backup_texts(dir: &Path) -> Result<Vec<String>> {
        let mut texts = std::fs::read_dir(dir)?
            .map(|entry| std::fs::read_to_string(entry?.path()))
            .collect::<io::Result<Vec<_>>>()?;
        texts.sort();
        Ok(texts)
    }

    #[test]
    fn each_untitled_tab_has_its_own_backup() -> Result<()> {
        let data = tempfile::tempdir()?;
        let mut app = App {
            screen: Rect::new(0, 0, 100, 30),
            ..App::default()
        }
        .with_backups(Backups::new(Some(data.path().to_path_buf())));
        press(&mut app, &["a", "ctrl+n", "b"]);
        app.handle_event(AppEvent::BackupDue);
        let dir = data.path().join("backups");
        assert_eq!(backup_texts(&dir)?, ["a", "b"]);
        // Closing one with discard removes only its backup.
        press(&mut app, &["ctrl+w", "d"]);
        assert_eq!(backup_texts(&dir)?, ["a"]);
        Ok(())
    }

    #[test]
    fn tab_bar_click_selects_and_middle_click_closes() {
        let mut app = app_with("one", &FakeClipboard::default());
        press(&mut app, &["ctrl+n"]);
        // The pills sit on row 1: " untitled " and its caps are 12 cells from
        // column 1, then a gap, so the second pill starts at column 14.
        app.handle_mouse(mouse(LEFT_DOWN, 2, 0), Instant::now());
        assert_eq!(app.tabs.split().active, 1);
        app.handle_mouse(mouse(LEFT_DOWN, 2, 1), Instant::now());
        assert_eq!(app.tabs.split().active, 0);
        let middle = MouseEventKind::Down(MouseButton::Middle);
        app.handle_mouse(mouse(middle, 13, 1), Instant::now());
        assert_eq!(app.tabs.split().tabs.len(), 2);
        app.handle_mouse(mouse(middle, 14, 1), Instant::now());
        assert_eq!(app.tabs.split().tabs.len(), 1);
        assert_eq!(app.buffer().rope.to_string(), "one");
        press(&mut app, &["x"]);
        app.handle_mouse(mouse(middle, 2, 1), Instant::now());
        assert_eq!(app.prompt, Some(Prompt::UnsavedClose));
    }

    fn type_keys(app: &mut App, text: &str) {
        for c in text.chars() {
            app.handle_event(key(&c.to_string()));
        }
    }

    fn selected_name(app: &App) -> Option<&str> {
        app.tree.selected_row().map(|row| row.entry.name.as_str())
    }

    #[test]
    fn d_asks_then_trashes_and_closes_the_open_buffer() -> Result<()> {
        let (dir, mut app) = project()?;
        let trash = ops::FakeTrash::default();
        app.trash = Box::new(trash.clone());
        let a = dir.path().join("a.txt");
        press(&mut app, &["enter", "ctrl+e"]);
        assert_eq!(app.buffer().path.as_deref(), Some(a.as_path()));

        press(&mut app, &["d"]);
        assert_eq!(app.prompt, Some(Prompt::Trash));
        assert_eq!(
            app.confirm(Prompt::Trash).question,
            "Move a.txt to the trash?"
        );
        // `n` and Esc both leave everything alone.
        press(&mut app, &["n"]);
        assert_eq!(app.prompt, None);
        press(&mut app, &["d", "esc"]);
        assert_eq!(app.prompt, None);
        assert!(trash.trashed.borrow().is_empty());
        assert!(a.exists());
        assert_eq!(app.buffer().path.as_deref(), Some(a.as_path()));

        press(&mut app, &["d", "y"]);
        assert_eq!(trash.trashed.borrow().as_slice(), std::slice::from_ref(&a));
        assert!(!a.exists());
        assert_eq!(app.buffer().path, None);
        assert_eq!(app.message.as_deref(), Some("moved a.txt to trash"));
        // The tree read the disk again and kept a selection.
        assert_eq!(app.tree.rows().len(), 1);
        assert_eq!(selected_name(&app), Some("b.txt"));
        Ok(())
    }

    #[test]
    fn a_failed_trash_says_why_and_keeps_the_buffer() -> Result<()> {
        let (dir, mut app) = project()?;
        app.trash = Box::new(ops::FakeTrash {
            fail: true,
            ..ops::FakeTrash::default()
        });
        press(&mut app, &["enter", "ctrl+e", "d", "y"]);
        assert_eq!(
            app.message.as_deref(),
            Some("cannot move a.txt to trash: trash unavailable")
        );
        assert!(dir.path().join("a.txt").exists());
        assert_eq!(app.buffer().rope.to_string(), "a");
        assert_eq!(app.tree.rows().len(), 2);
        Ok(())
    }

    #[test]
    fn tree_letters_type_into_the_editor_and_the_bar() -> Result<()> {
        let (dir, mut app) = project()?;
        // In the tree `a` opens the bar; inside the bar `a`, `r`, `d` are text.
        press(&mut app, &["a"]);
        assert!(app.name_prompt.is_some());
        assert_eq!(app.panes().splits[0].editor.height, 25);
        type_keys(&mut app, "dra.txt");
        press(&mut app, &["enter"]);
        assert_eq!(app.name_prompt, None);
        assert!(dir.path().join("dra.txt").is_file());
        // The new file is open with focus in the editor, where letters are text.
        assert_eq!(app.focus, Focus::Editor);
        type_keys(&mut app, "ard");
        assert_eq!(app.buffer().rope.to_string(), "ard");
        assert_eq!(selected_name(&app), Some("dra.txt"));
        Ok(())
    }

    #[test]
    fn rename_moves_the_open_buffer_to_the_new_path() -> Result<()> {
        let (dir, mut app) = project()?;
        press(&mut app, &["enter", "x", "ctrl+e", "r"]);
        let bar = app.name_prompt.as_ref().map(|p| p.bar.text().to_string());
        assert_eq!(bar.as_deref(), Some("a.txt"));
        press(&mut app, &["backspace"; 3]);
        type_keys(&mut app, "md");
        press(&mut app, &["enter"]);
        let renamed = dir.path().join("a.md");
        assert!(renamed.exists() && !dir.path().join("a.txt").exists());
        assert_eq!(app.buffer().path.as_deref(), Some(renamed.as_path()));
        assert_eq!(selected_name(&app), Some("a.md"));
        // Unsaved edits survive and save to the new name.
        press(&mut app, &["ctrl+e", "ctrl+s"]);
        assert_eq!(std::fs::read_to_string(&renamed)?, "xa");

        // A clash changes nothing and says why.
        press(&mut app, &["ctrl+e", "r"]);
        press(&mut app, &["backspace"; 4]);
        type_keys(&mut app, "b.txt");
        press(&mut app, &["enter"]);
        assert_eq!(app.message.as_deref(), Some("b.txt already exists"));
        assert_eq!(std::fs::read_to_string(dir.path().join("b.txt"))?, "b");
        assert!(renamed.exists());
        Ok(())
    }

    #[test]
    fn the_tree_takes_29_columns_from_the_editor_while_shown() -> Result<()> {
        let (_dir, mut app) = project()?;
        assert_eq!(app.panes().splits[0].editor, Rect::new(29, 3, 71, 26));
        assert_eq!(app.panes().splits[0].header, Rect::new(29, 0, 71, 3));
        assert_eq!(app.panes().tree, Some(Rect::new(0, 0, 28, 29)));
        press(&mut app, &["ctrl+b"]);
        assert_eq!(app.panes().splits[0].editor, Rect::new(0, 3, 100, 26));
        assert_eq!(app.panes().tree, None);
        assert_eq!(app.focus, Focus::Editor);
        // Ctrl+E brings a hidden tree back with focus.
        press(&mut app, &["ctrl+e"]);
        assert!(app.tree_visible);
        assert_eq!(app.focus, Focus::Tree);
        press(&mut app, &["esc"]);
        assert_eq!(app.focus, Focus::Editor);
        Ok(())
    }

    #[test]
    fn alt_v_splits_on_the_same_buffer_and_closing_keeps_every_buffer() {
        let mut app = app_with("one", &FakeClipboard::default());
        press(&mut app, &["alt+v"]);
        assert_eq!(app.tabs.splits.len(), 2);
        assert_eq!(app.tabs.focused, 1);
        assert_eq!(
            app.tabs.splits[0].active().doc,
            app.tabs.splits[1].active().doc
        );
        // Roughly half each, with a one-column divider between.
        let panes = app.panes();
        assert_eq!(panes.splits[0].editor, Rect::new(0, 3, 50, 26));
        assert_eq!(panes.splits[1].editor, Rect::new(51, 3, 49, 26));
        assert_eq!(panes.split_divider, Some(Rect::new(50, 0, 1, 29)));

        // New tabs go to the focused split only.
        press(&mut app, &["ctrl+n", "x"]);
        assert_eq!(tab_names(&app), ["untitled", "untitled ●"]);
        assert_eq!(app.tabs.labels(0).len(), 1);
        // Closing the split moves the tab only it had to the left one.
        press(&mut app, &["alt+v"]);
        assert_eq!(app.tabs.splits.len(), 1);
        assert_eq!(app.tabs.focused, 0);
        assert_eq!(tab_names(&app), ["untitled", "untitled ●"]);
        assert_eq!(app.buffer().rope.to_string(), "one");
    }

    #[test]
    fn each_split_keeps_its_own_cursor_on_a_shared_buffer() {
        let mut app = app_with("hello", &FakeClipboard::default());
        press(&mut app, &["alt+v", "end"]);
        assert_eq!(app.buffer().cursor, 5);
        // F6 to the left split, which still has its cursor at the start.
        press(&mut app, &["f6"]);
        assert_eq!(app.tabs.focused, 0);
        assert_eq!(app.buffer().cursor, 0);
        press(&mut app, &["x", "y"]);
        // The right split's place moved along with the text before it.
        press(&mut app, &["f6"]);
        assert_eq!(app.tabs.focused, 1);
        assert_eq!(app.buffer().cursor, 7);
        press(&mut app, &["!"]);
        assert_eq!(app.buffer().rope.to_string(), "xyhello!");
        press(&mut app, &["f6"]);
        assert_eq!(app.buffer().cursor, 2);
    }

    #[test]
    fn f6_cycles_tree_left_right_skipping_what_is_hidden() -> Result<()> {
        let (_dir, mut app) = project()?;
        let at = |app: &App| match app.focus {
            Focus::Tree => "tree",
            Focus::Editor if app.tabs.focused == 0 => "left",
            Focus::Editor => "right",
        };
        assert_eq!(at(&app), "tree");
        press(&mut app, &["f6"]);
        assert_eq!(at(&app), "left");
        press(&mut app, &["f6"]);
        assert_eq!(at(&app), "tree");
        press(&mut app, &["alt+v"]);
        assert_eq!(at(&app), "right");
        let mut seen = Vec::new();
        for _ in 0..3 {
            press(&mut app, &["f6"]);
            seen.push(at(&app));
        }
        assert_eq!(seen, ["tree", "left", "right"]);
        press(&mut app, &["ctrl+b", "f6"]);
        assert_eq!(at(&app), "left");
        press(&mut app, &["f6"]);
        assert_eq!(at(&app), "right");
        Ok(())
    }

    #[test]
    fn a_click_focuses_its_split_and_opening_goes_there() -> Result<()> {
        let (_dir, mut app) = project()?;
        press(&mut app, &["enter", "alt+v"]);
        assert_eq!(app.tabs.focused, 1);
        // The left split's editor starts at column 29, below its three header rows.
        click(&mut app, 40, 3, Instant::now());
        assert_eq!(app.tabs.focused, 0);
        // Opening b.txt from the tree puts it in the left split only.
        press(&mut app, &["ctrl+e", "down", "enter"]);
        assert_eq!(tab_names(&app), ["a.txt", "b.txt"]);
        assert_eq!(app.tabs.labels(1).len(), 1);
        // A click on the right split's pill focuses that split.
        let right = app.panes().splits[1].header;
        app.handle_mouse(mouse(LEFT_DOWN, right.x + 2, 1), Instant::now());
        assert_eq!(app.tabs.focused, 1);
        assert_eq!(app.buffer().rope.to_string(), "a");
        Ok(())
    }

    #[test]
    fn closing_a_tab_the_other_split_shows_keeps_the_buffer() {
        let mut app = app_with("", &FakeClipboard::default());
        press(&mut app, &["x", "alt+v", "ctrl+w"]);
        // Still open on the left, so nothing asked and nothing lost.
        assert_eq!(app.prompt, None);
        assert_eq!(tab_names(&app), ["untitled"]);
        press(&mut app, &["f6"]);
        assert_eq!(tab_names(&app), ["untitled ●"]);
        assert_eq!(app.buffer().rope.to_string(), "x");
    }

    #[test]
    #[ignore = "timing bench; run with cargo test -- --ignored"]
    fn bench_typing_10k_lines_full_frame() -> Result<()> {
        let sample = std::fs::read_to_string("tests/fixtures/highlight/sample.rs")?;
        let mut text = String::new();
        while text.lines().count() < 10_000 {
            text.push_str(&sample);
        }
        let mut app = App {
            tabs: Tabs::new(Buffer {
                rope: ropey::Rope::from_str(&text),
                path: Some("big.rs".into()),
                ..Buffer::empty()
            }),
            screen: Rect::new(0, 0, 100, 30),
            ..App::default()
        };
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        let line = 5_000;
        let at = app.buffer().rope.line_to_char(line) + 4;
        app.buffer_mut().cursor = at;
        let xs = |app: &App| {
            app.buffer()
                .rope
                .line(line)
                .chars()
                .filter(|&c| c == 'x')
                .count()
        };
        let before = xs(&app);
        app.sync_highlights();
        terminal.draw(|frame| app.render(frame))?;

        // One typed char in the middle of the file, then the frame it costs: the
        // key handling, the tree edit and reparse, and drawing the whole screen.
        let mut worst = Duration::ZERO;
        for i in 0..20 {
            let start = Instant::now();
            app.handle_event(key("x"));
            app.sync_highlights();
            terminal.draw(|frame| app.render(frame))?;
            let took = start.elapsed();
            worst = worst.max(took);

            let screen = terminal.backend().buffer();
            let keyword = app.theme.syntax.keyword.fg.unwrap_or_default();
            let coloured = (0..30u16)
                .flat_map(|y| (0..100u16).map(move |x| (x, y)))
                .any(|(x, y)| screen[(x, y)].fg == keyword);
            assert!(coloured, "keystroke {i} drew no keyword colour");
        }
        assert_eq!(
            xs(&app),
            before + 20,
            "every keystroke typed on line {line}"
        );
        println!("worst keystroke with a full frame: {worst:?}");
        assert!(
            worst < Duration::from_millis(16),
            "a keystroke took {worst:?}"
        );
        Ok(())
    }

    #[test]
    fn status_line_shows_the_message() -> Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        let app = App::new(
            Keymap::default(),
            EditorConfig::default(),
            None,
            Some("config error: boom".into()),
        );
        terminal.draw(|frame| app.render(frame))?;
        let buffer = terminal.backend().buffer();
        let last: String = (0..100).map(|x| buffer[(x, 29)].symbol()).collect();
        assert!(last.starts_with(" ✦ glyph"), "{last}");
        // The message holds the path slot at x = 20 until the next key.
        let slot: String = last.chars().skip(20).collect();
        assert!(slot.starts_with("✕ config error: boom"), "{last}");
        assert!(!last.contains("untitled"), "{last}");
        Ok(())
    }

    #[test]
    fn the_next_key_gives_the_path_slot_back() -> Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        let mut app = App::new(
            Keymap::default(),
            EditorConfig::default(),
            None,
            Some("config error: boom".into()),
        );
        press(&mut app, &["right"]);
        terminal.draw(|frame| app.render(frame))?;
        let buffer = terminal.backend().buffer();
        let last: String = (0..100).map(|x| buffer[(x, 29)].symbol()).collect();
        assert!(!last.contains("config error"), "{last}");
        let slot: String = last.chars().skip(20).collect();
        assert!(slot.starts_with("untitled"), "{last}");
        Ok(())
    }

    #[test]
    fn status_line_is_on_the_last_row() -> Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        let app = App::default();
        terminal.draw(|frame| app.render(frame))?;

        let buffer = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..100).map(|x| buffer[(x, y)].symbol()).collect() };
        // A blank row, the pill, the thread, then an empty buffer: one numbered
        // line and nothing below.
        assert_eq!(row(0).trim(), "");
        assert_eq!(row(1).trim(), "▐ untitled ▌");
        assert!(row(2).chars().all(|c| c == '─'));
        assert_eq!(row(3).trim_end(), "   1");
        for y in 4..29 {
            assert_eq!(row(y).trim(), "", "row {y} should be blank");
        }
        assert!(row(29).starts_with(" ✦ glyph"));
        Ok(())
    }

    #[test]
    fn unopenable_file_leaves_an_untitled_buffer_and_says_why() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("bad.txt");
        std::fs::write(&path, [0xff_u8, 0xfe])?;
        let app = App::new(
            Keymap::default(),
            EditorConfig::default(),
            Some(path.clone()),
            Some("config error: boom".into()),
        );
        assert!(app.buffer().path.is_none());
        let message = app.message.unwrap_or_default();
        assert!(message.contains("config error: boom"), "{message}");
        assert!(
            message.contains(&format!("cannot open {}: not UTF-8", path.display())),
            "{message}"
        );
        Ok(())
    }

    fn panel(app: &App) -> &ProjectSearch {
        app.project_search.as_ref().expect("project search is open")
    }

    fn hit_rows(app: &App) -> Vec<String> {
        panel(app)
            .hits()
            .iter()
            .map(|hit| format!("{}:{}: {}", hit.path, hit.line.line + 1, hit.line.text))
            .collect()
    }

    #[test]
    fn project_search_reads_open_buffers_from_memory_and_opens_a_hit() -> Result<()> {
        let (dir, mut app) = project()?;
        std::fs::create_dir(dir.path().join("sub"))?;
        std::fs::write(
            dir.path().join("sub").join("c.txt"),
            "one
  two a
",
        )?;
        app.open(&dir.path().join("a.txt"));
        // Unsaved: the file on disk still holds just `a`.
        press(&mut app, &["ctrl+end", "x"]);
        press(&mut app, &["alt+f", "a", "enter"]);
        // `a.txt` shows once, as the buffer has it, not again as the disk does.
        assert_eq!(hit_rows(&app), ["a.txt:1: ax", "sub/c.txt:2: two a"]);
        assert_eq!(panel(&app).status(), "2 matches in 2 files");
        press(&mut app, &["down", "enter"]);
        assert!(app.project_search.is_none());
        assert_eq!(
            app.buffer().path,
            Some(dir.path().join("sub").join("c.txt"))
        );
        assert_eq!(app.buffer().cursor_line_col(), (1, 6));
        assert_eq!(app.focus, Focus::Editor);
        Ok(())
    }

    #[test]
    fn project_search_toggles_research_and_esc_closes() -> Result<()> {
        let (_dir, mut app) = project()?;
        press(&mut app, &["alt+f", "shift+a", "enter"]);
        assert_eq!(hit_rows(&app), ["a.txt:1: a"]);
        press(&mut app, &["alt+c"]);
        assert!(hit_rows(&app).is_empty());
        assert_eq!(panel(&app).status(), "no matches");
        press(&mut app, &["alt+c", "alt+r", "backspace", "("]);
        press(&mut app, &["enter"]);
        assert_eq!(panel(&app).status(), "invalid regex");
        press(&mut app, &["esc"]);
        assert!(app.project_search.is_none());
        // Keys go back to where they went before.
        assert_eq!(app.focus, Focus::Tree);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn project_search_streams_hits_while_keys_keep_working() -> Result<()> {
        let dir = tempfile::tempdir()?;
        for n in 0..2000 {
            std::fs::write(
                dir.path().join(format!("f{n:04}.txt")),
                "x
needle
",
            )?;
        }
        let mut app = App::new(
            Keymap::default(),
            EditorConfig::default(),
            Some(dir.path().to_path_buf()),
            None,
        );
        app.screen = Rect::new(0, 0, 100, 30);
        let (tx, mut rx) = mpsc::unbounded_channel();
        app.events = Some(tx);
        press(&mut app, &["alt+f", "n", "e", "e", "d", "l", "e", "enter"]);
        // Enter came straight back; the search goes on in the background.
        assert!(panel(&app).status().starts_with("searching"));
        let mut batches = 0;
        loop {
            let event = tokio::time::timeout(Duration::from_secs(20), rx.recv())
                .await?
                .expect("the search reports back");
            let done = matches!(event, AppEvent::SearchDone { .. });
            batches += usize::from(matches!(event, AppEvent::SearchHits { .. }));
            app.handle_event(event);
            if done {
                break;
            }
            // A key between batches is handled at once, before the next arrives.
            let before = panel(&app).selected().cloned();
            press(&mut app, &["down"]);
            let after = panel(&app).selected().cloned();
            assert!(after.is_some() && (after != before || panel(&app).hits().len() < 2));
        }
        assert!(batches > 1, "every hit came in one batch");
        assert_eq!(panel(&app).hits().len(), 2000);
        assert_eq!(panel(&app).status(), "2000 matches in 2000 files");
        Ok(())
    }

    fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let target = to.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                std::fs::create_dir(&target)?;
                copy_dir(&entry.path(), &target)?;
            } else {
                std::fs::copy(entry.path(), &target)?;
            }
        }
        Ok(())
    }

    /// A temp copy of the project replace fixture, open as the project, with
    /// `notes.txt` in CRLF: three `todo`s there, one in `src/app.rs`.
    fn replace_project() -> Result<(tempfile::TempDir, App)> {
        let dir = tempfile::tempdir()?;
        copy_dir(Path::new("tests/fixtures/project_replace"), dir.path())?;
        let notes = dir.path().join("notes.txt");
        let lf = std::fs::read_to_string(&notes)?.replace("\r\n", "\n");
        std::fs::write(&notes, lf.replace('\n', "\r\n"))?;
        let mut app = App::new(
            Keymap::default(),
            EditorConfig::default(),
            Some(dir.path().to_path_buf()),
            None,
        );
        app.screen = Rect::new(0, 0, 100, 30);
        Ok((dir, app))
    }

    /// Searches for `todo`, then asks to replace it with `done`.
    fn replace_todo(app: &mut App) {
        press(app, &["alt+f", "t", "o", "d", "o", "enter"]);
        assert_eq!(panel(app).status(), "4 matches in 2 files");
        press(app, &["tab", "d", "o", "n", "e", "alt+a"]);
    }

    #[test]
    fn project_replace_edits_open_buffers_in_place_and_saves_the_rest() -> Result<()> {
        let (dir, mut app) = replace_project()?;
        let notes = dir.path().join("notes.txt");
        let code = dir.path().join("src").join("app.rs");
        let code_before = std::fs::read(&code)?;
        app.open(&code);
        replace_todo(&mut app);
        assert_eq!(app.prompt, Some(Prompt::ProjectReplace));
        assert_eq!(
            app.confirm(Prompt::ProjectReplace).question,
            "Replace 4 matches in 2 files?"
        );
        press(&mut app, &["r"]);
        assert_eq!(app.prompt, None);
        assert_eq!(app.message.as_deref(), Some("Replaced 4 in 2 files"));
        // Rewritten on disk, still CRLF, and no temp file left beside it.
        assert_eq!(
            std::fs::read(&notes)?,
            b"done: buy milk\r\nnothing here\r\ndone later, done soon\r\n"
        );
        let mut names: Vec<_> = std::fs::read_dir(dir.path())?
            .map(|entry| entry.map(|e| e.file_name()))
            .collect::<io::Result<_>>()?;
        names.sort();
        assert_eq!(names, ["README.md", "notes.txt", "src"]);
        // The open buffer changed in memory only, left unsaved.
        assert_eq!(std::fs::read(&code)?, code_before);
        assert_eq!(
            app.buffer().rope.to_string(),
            "fn main() {\n    // done tidy\n}\n"
        );
        assert!(app.buffer().dirty);
        // The list was searched again: nothing is left to find.
        assert_eq!(panel(&app).status(), "no matches");
        // One undo step takes the buffer's whole replace back.
        press(&mut app, &["esc", "ctrl+z"]);
        assert_eq!(
            app.buffer().rope.to_string(),
            "fn main() {\n    // TODO tidy\n}\n"
        );
        Ok(())
    }

    #[test]
    fn project_replace_reports_a_file_it_cannot_write_and_does_the_others() -> Result<()> {
        let (dir, mut app) = replace_project()?;
        let notes = dir.path().join("notes.txt");
        let code = dir.path().join("src").join("app.rs");
        let notes_before = std::fs::read(&notes)?;
        let mut perms = std::fs::metadata(&notes)?.permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&notes, perms.clone())?;
        replace_todo(&mut app);
        press(&mut app, &["r"]);
        let notes_after = std::fs::read(&notes)?;
        // tempdir can't delete a read-only file on Windows.
        #[expect(
            clippy::permissions_set_readonly_false,
            reason = "only to let the temp dir clean up"
        )]
        perms.set_readonly(false);
        std::fs::set_permissions(&notes, perms)?;
        assert_eq!(
            app.message.as_deref(),
            Some("Replaced 1 in 1 file · cannot write notes.txt: read-only")
        );
        assert_eq!(notes_after, notes_before);
        assert_eq!(
            std::fs::read_to_string(&code)?.replace("\r\n", "\n"),
            "fn main() {\n    // done tidy\n}\n"
        );
        // What's left is what the failed file still holds.
        assert_eq!(panel(&app).status(), "3 matches in 1 file");
        Ok(())
    }

    #[test]
    fn project_replace_does_nothing_when_cancelled() -> Result<()> {
        let (dir, mut app) = replace_project()?;
        let notes = dir.path().join("notes.txt");
        let before = std::fs::read(&notes)?;
        replace_todo(&mut app);
        press(&mut app, &["c"]);
        assert_eq!(app.prompt, None);
        assert_eq!(app.pending_replace, None);
        assert_eq!(std::fs::read(&notes)?, before);
        assert_eq!(panel(&app).status(), "4 matches in 2 files");
        Ok(())
    }
}
