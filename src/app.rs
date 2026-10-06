use std::borrow::Cow;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    Event, EventStream, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
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
use crate::config::EditorConfig;
#[cfg(windows)]
use crate::keymap::burst_as_paste;
use crate::keymap::{Action, Input, Keymap, Scope};
use crate::theme::Theme;
use crate::ui::confirm::{Answer, Choice, Confirm, Labels};
use crate::ui::picker::{Picked, Picker};
use crate::ui::prompt::{Outcome, PromptBar};
use crate::ui::status::render_status;
use crate::ui::tabs::{TabLabel, render_tabs, tab_at};
use crate::ui::tree::{TREE_WIDTH, render_divider, render_tree};
use crate::view::{View, render_buffer};
use crate::workspace::ops::{self, Trash};
use crate::workspace::walk::list_files;
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
    /// The background walk for the go-to-file picker finished: every file in the
    /// project, relative to the root.
    FilesListed(Vec<String>),
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
    /// A line number to move the cursor to.
    GoToLine,
}

/// The prompt bar while it asks for a name.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NamePrompt {
    op: BarOp,
    bar: PromptBar,
}

/// Which pane keys go to.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Focus {
    #[default]
    Editor,
    Tree,
}

const UNSAVED_QUIT: Confirm = Confirm {
    question: Cow::Borrowed("Unsaved changes"),
    choices: &[
        Choice {
            key: 's',
            label: "save",
        },
        Choice {
            key: 'd',
            label: "discard",
        },
        Choice {
            key: 'c',
            label: "cancel",
        },
    ],
    labels: Labels::Bracketed,
};

const RECOVER: Confirm = Confirm {
    question: Cow::Borrowed("Recover unsaved changes?"),
    choices: &[
        Choice {
            key: 'r',
            label: "recover",
        },
        Choice {
            key: 'd',
            label: "discard",
        },
    ],
    labels: Labels::Words,
};

const TRASH_CHOICES: &[Choice] = &[
    Choice {
        key: 'y',
        label: "yes",
    },
    Choice {
        key: 'n',
        label: "no",
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

    /// What split `split`'s tab bar shows.
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
            Shown::Copy(buffer.with_caret(tab.caret))
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
    Copy(Buffer),
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
    /// While set, every key goes to the prompt instead of the editor.
    prompt: Option<Prompt>,
    /// The project's file tree, read from disk as folders expand.
    tree: Tree,
    tree_visible: bool,
    focus: Focus,
    /// The prompt bar, while it's asking for a name; it takes every key.
    name_prompt: Option<NamePrompt>,
    /// The go-to-file picker, while it's open; it takes every key.
    picker: Option<Picker>,
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
    /// A backup write failed and the status line said so; cleared by the next
    /// success, so a run of failures shows one message instead of one per edit.
    backup_failed: bool,
    /// Tells the backup timer about each edit. `None` outside the event loop (unit
    /// tests), where `AppEvent::BackupDue` is sent by hand.
    edits: Option<mpsc::UnboundedSender<()>>,
    /// The app channel, for background backup writes to report back on. `None`
    /// outside the event loop, where writes run inline.
    events: Option<mpsc::UnboundedSender<AppEvent>>,
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
            tree: Tree::new(&launch.root),
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

    /// Turns crash backups on, and offers to recover the opened file's backup if
    /// it has a newer one.
    pub fn with_backups(mut self, backups: Backups) -> Self {
        self.backups = backups;
        if let Some(path) = self.buffer().path.clone()
            && let Some(text) = self.backups.recoverable(&path)
        {
            self.recovery = Some(text);
            self.prompt = Some(Prompt::Recover);
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
        let input = tokio::spawn(read_input(tx));

        let result = self.event_loop(terminal, &mut rx).await;
        input.abort();
        result
    }

    async fn event_loop(
        &mut self,
        terminal: &mut Tui,
        rx: &mut mpsc::UnboundedReceiver<AppEvent>,
    ) -> Result<()> {
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
            self.draw(terminal)?;
        }
        Ok(())
    }

    /// Draws one frame as a synchronized update, so a terminal shows it whole: a
    /// themed frame repaints every cell, and drawn piecemeal it tears.
    fn draw(&mut self, terminal: &mut Tui) -> Result<()> {
        execute!(terminal.backend_mut(), BeginSynchronizedUpdate)?;
        self.screen = terminal.draw(|frame| self.render(frame))?.area;
        execute!(terminal.backend_mut(), EndSynchronizedUpdate)?;
        Ok(())
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Input(Event::Key(key)) => self.handle_key(key),
            AppEvent::Input(Event::Paste(text)) if self.prompt.is_none() => {
                if let Some(picker) = &mut self.picker {
                    if let Some(picked) = picker.paste(&text) {
                        self.finish_picker(picked);
                    }
                    return;
                }
                if let Some(name_prompt) = &mut self.name_prompt {
                    if let Some(outcome) = name_prompt.bar.paste(&text) {
                        self.finish_name_prompt(outcome);
                    }
                    return;
                }
                // Bracketed paste bypasses the keymap: it is text, not a key.
                // Windows Terminal's own Ctrl+V paste arrives this way too.
                self.edit(|buffer| buffer.paste(&text));
            }
            // The mouse bypasses the keymap too: only keys are remappable.
            AppEvent::Input(Event::Mouse(mouse))
                if self.prompt.is_none() && self.name_prompt.is_none() && self.picker.is_none() =>
            {
                self.handle_mouse(mouse, Instant::now());
            }
            // A smaller pane can leave the cursor outside it.
            AppEvent::Input(Event::Resize(..)) => self.follow_cursor(),
            AppEvent::Input(_) => {}
            AppEvent::BackupDue => self.back_up(),
            AppEvent::BackupWritten(result) => self.backup_written(result),
            // A list for a picker that has since closed is dropped; the next
            // Ctrl+P walks again.
            AppEvent::FilesListed(files) => {
                if let Some(picker) = &mut self.picker {
                    picker.set_files(files);
                }
            }
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Tree letters are only actions when nothing else is reading keys as text.
        let scope = if self.focus == Focus::Tree
            && self.prompt.is_none()
            && self.name_prompt.is_none()
            && self.picker.is_none()
        {
            Scope::Tree
        } else {
            Scope::Global
        };
        let input = self.keymap.resolve_in(&key, scope);
        if let Some(prompt) = self.prompt {
            if let Some(answer) = self.confirm(prompt).answer(input) {
                self.answer_prompt(prompt, answer);
            }
            return;
        }
        if let Some(picker) = &mut self.picker {
            if let Some(picked) = picker.handle(input) {
                self.finish_picker(picked);
            }
            return;
        }
        if let Some(name_prompt) = &mut self.name_prompt {
            if let Some(outcome) = name_prompt.bar.handle(input) {
                self.finish_name_prompt(outcome);
            }
            return;
        }
        match input {
            Input::Action(action) if self.focus == Focus::Tree => self.handle_tree_action(action),
            Input::Action(action) => self.handle_action(action),
            // Typing in the tree does nothing until type-to-find exists.
            Input::Text(_) if self.focus == Focus::Tree => {}
            Input::Text(ch) => self.edit(|buffer| buffer.type_text(ch.encode_utf8(&mut [0; 4]))),
            Input::Ignored => {}
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
            | Action::GoToLine => {
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
                    self.prompt = Some(Prompt::Recover);
                }
                self.follow_cursor();
            }
            Err(err) => self.message = Some(format!("cannot open {}: {err}", path.display())),
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
            self.prompt = Some(Prompt::UnsavedClose);
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
                self.prompt = Some(Prompt::UnsavedQuit);
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

    /// Rows one PageUp/PageDown moves in the tree.
    fn tree_page(&self) -> isize {
        isize::try_from(self.editor_area().height).unwrap_or(isize::MAX)
    }

    fn follow_tree(&mut self) {
        if let Some(area) = self.panes().tree {
            self.tree.follow(usize::from(area.height));
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
            Action::Save => {
                self.save_or_ask(AfterSave::Nothing);
            }
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
                Err(err) => self.message = Some(format!("cannot paste: {err}")),
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
            Action::GoToFile => self.open_picker(),
            Action::GoToLine => {
                self.open_name_prompt(BarOp::GoToLine, PromptBar::new("Go to line", ""));
            }
            // Bound only in tree scope, so they never reach the editor.
            Action::TreeNewFile
            | Action::TreeNewFolder
            | Action::TreeRename
            | Action::TreeDelete => {}
        }
    }

    /// The card a confirm prompt shows.
    fn confirm(&self, prompt: Prompt) -> Confirm {
        match prompt {
            Prompt::UnsavedQuit | Prompt::UnsavedClose => UNSAVED_QUIT,
            Prompt::Recover => RECOVER,
            Prompt::Trash => Confirm {
                question: Cow::Owned(format!(
                    "Move {} to trash?",
                    self.pending_trash
                        .as_deref()
                        .map(file_name)
                        .unwrap_or_default()
                )),
                choices: TRASH_CHOICES,
                labels: Labels::Keys,
            },
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
            self.prompt = Some(Prompt::Trash);
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
        if op == BarOp::GoToLine {
            if outcome == Outcome::Submit {
                self.go_to_line(bar.text());
            }
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
            BarOp::SaveAs(_) | BarOp::GoToLine => return,
        };
        let path = match result {
            Ok(path) => path,
            Err(err) => {
                self.message = Some(err.to_string());
                return;
            }
        };
        self.tree.reload(Some(&path));
        self.follow_tree();
        match op {
            BarOp::NewFile(_) => {
                self.message = Some(format!("created {name}"));
                self.open(&path);
            }
            BarOp::NewFolder(_) => self.message = Some(format!("created {name}")),
            BarOp::Rename(old) => {
                self.message = Some(format!("renamed to {name}"));
                self.follow_rename(&old, &path);
            }
            BarOp::SaveAs(_) | BarOp::GoToLine => {}
        }
    }

    /// Moves the cursor to the line `text` names, counting from 1; past the end
    /// means the last line.
    fn go_to_line(&mut self, text: &str) {
        match text.trim().parse::<usize>() {
            Ok(line) => {
                self.focus = Focus::Editor;
                self.buffer_mut().go_to_line(line);
                self.follow_cursor();
            }
            Err(_) => self.message = Some(format!("not a line number: {}", text.trim())),
        }
    }

    /// Ctrl+P: opens the picker and lists the project's files on a background
    /// thread, so a large project never stalls typing (ADR-0001). Outside the
    /// event loop (unit tests) the walk runs inline.
    fn open_picker(&mut self) {
        let mut picker = Picker::new();
        let root = self.tree.root().to_path_buf();
        match &self.events {
            Some(events) => {
                let events = events.clone();
                tokio::task::spawn_blocking(move || {
                    let _ = events.send(AppEvent::FilesListed(list_files(&root)));
                });
            }
            None => picker.set_files(list_files(&root)),
        }
        self.picker = Some(picker);
    }

    /// Closes the picker and, on Enter, opens the chosen file in a tab.
    fn finish_picker(&mut self, picked: Picked) {
        self.picker = None;
        if let Picked::Open(relative) = picked {
            let mut path = self.tree.root().to_path_buf();
            path.extend(relative.split('/'));
            self.open(&path);
        }
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
            self.message = Some("no file name".into());
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
                self.message = Some(format!("{relative} is open in another tab"));
                return false;
            }
            if target.exists() {
                self.message = Some(format!("{relative} already exists"));
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
                self.message = Some(format!("saved {name}"));
                self.tree.reload(Some(&target));
                self.follow_tree();
                true
            }
            Err(err) => {
                self.buffer_mut().path = old;
                self.message = Some(format!("cannot save {name}: {err}"));
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
            self.message = Some(format!("cannot move {name} to trash: {err}"));
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
        self.message = Some(format!("moved {name} to trash"));
        self.tree.reload(None);
        self.follow_tree();
    }

    /// Click, drag, double-click and wheel in the editor panes, clicks on the tab
    /// bars. A press in a split focuses it. `now` is when the event arrived, passed
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
        if let Some(split) = panes.splits.iter().position(|s| s.tabs.contains(at)) {
            self.handle_tab_mouse(mouse, split, panes.splits[split].tabs);
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
            // Ctrl+click is left for go to definition (#32).
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
    fn handle_tab_mouse(&mut self, mouse: MouseEvent, split: usize, area: Rect) {
        self.reset_mouse();
        let labels = self.tabs.labels(split);
        let active = self.tabs.splits[split].active;
        let Some(index) = tab_at(&labels, active, area, mouse.column) else {
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
        let height = usize::from(area.height);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.focus = Focus::Tree;
                let line = usize::from(mouse.row.saturating_sub(area.y));
                if let Some(index) = self.tree.row_at(line) {
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
                self.message = Some(format!("cannot copy: {err}"));
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
            (Prompt::Recover, _) => self.prompt = Some(Prompt::Recover),
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
                self.message = Some(format!("cannot back up {}: {err}", self.buffer().name()));
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
        let name = self.buffer().name();
        match self.buffer_mut().save() {
            Ok(()) => {
                self.message = Some(format!("saved {name}"));
                self.delete_backup();
                true
            }
            Err(err) => {
                self.message = Some(format!("cannot save {name}: {err}"));
                false
            }
        }
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
            self.name_prompt.is_some(),
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
            self.name_prompt.is_some(),
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
                area.tabs,
                frame,
            );
            render_buffer(
                theme,
                self.tabs.shown(split).buffer(),
                &tabs.active().view,
                self.editor.tab_width,
                area.editor,
                frame,
            );
        }
        if let Some(divider) = panes.split_divider {
            render_divider(theme, divider, frame);
        }
        if let (Some(tree), Some(divider)) = (panes.tree, panes.divider) {
            let focused = self.focus == Focus::Tree;
            let selected = render_tree(theme, &self.tree, focused, tree, frame);
            render_divider(theme, divider, frame);
            // The cursor marks the focused pane; `render_buffer` put it in the editor.
            if focused && let Some(at) = selected {
                frame.set_cursor_position(at);
            }
        }
        let buffer = self.buffer();
        render_status(
            frame,
            theme,
            panes.status,
            &buffer.name(),
            buffer.dirty,
            self.message.as_deref(),
            buffer.cursor_line_col(),
        );
        if let (Some(name_prompt), Some(area)) = (&self.name_prompt, panes.bar) {
            let at = name_prompt.bar.render(theme, frame, area);
            frame.set_cursor_position(at);
        }
        if let Some(picker) = &self.picker {
            let at = picker.render(theme, frame, frame.area());
            frame.set_cursor_position(at);
        }
        if let Some(prompt) = self.prompt {
            self.confirm(prompt).render(theme, frame, frame.area());
        }
    }
}

/// The last part of `path`, for messages and tab names.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// One editor split's place on screen: its tab bar row and the editor below it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SplitArea {
    tabs: Rect,
    editor: Rect,
}

/// Where each part of the screen goes: the tree (when shown) from row 1 down, a
/// `│` divider, then the editor splits, each with its tab bar on row 0 and a `│`
/// between the two; below them the prompt bar (while open) and a one-row status
/// line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Panes {
    tree: Option<Rect>,
    divider: Option<Rect>,
    /// One per split, left to right.
    splits: Vec<SplitArea>,
    /// Between the two splits, while there are two.
    split_divider: Option<Rect>,
    bar: Option<Rect>,
    status: Rect,
}

impl Panes {
    fn new(screen: Rect, tree_visible: bool, bar_open: bool, splits: usize) -> Self {
        let [main, bar, status] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(u16::from(bar_open)),
            Constraint::Length(1),
        ])
        .areas(screen);
        let bar = bar_open.then_some(bar);
        let (tree, divider, right) = if tree_visible {
            let [tree, divider, right] = Layout::horizontal([
                Constraint::Length(TREE_WIDTH),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .areas(main);
            // The tree starts below the tab bar's row; the divider runs through it.
            let [_, tree] =
                Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(tree);
            (Some(tree), Some(divider), right)
        } else {
            (None, None, main)
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
                let [tabs, editor] =
                    Layout::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(column);
                SplitArea { tabs, editor }
            })
            .collect();
        Panes {
            tree,
            divider,
            splits,
            split_divider,
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
    use crossterm::event::KeyEventKind;

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
        // The editor pane is 28 rows: the tab bar and status line take two.
        assert_eq!(app.buffer().cursor_line_col(), (28, 0));
        assert_eq!(app.view().scroll_row, 1);
        app.handle_event(key("ctrl+end"));
        assert_eq!(app.buffer().cursor_line_col(), (100, 0));
        assert_eq!(app.view().scroll_row, 73);
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
        // Gutter " 1 │ " is 5 cells; the tab bar takes row 0.
        let mut app = app_with(
            "hello
world",
            &FakeClipboard::default(),
        );
        click(&mut app, 7, 2, Instant::now());
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
        click(&mut app, 6, 1, t0);
        click(&mut app, 6, 1, t0 + Duration::from_millis(400));
        assert_eq!(app.buffer().selected_text().as_deref(), Some("hello"));

        // Too slow: a second plain click.
        let t1 = t0 + Duration::from_secs(5);
        click(&mut app, 12, 1, t1);
        click(&mut app, 12, 1, t1 + Duration::from_millis(401));
        assert_eq!(app.buffer().selection(), None);
        assert_eq!(app.buffer().cursor, 7);

        // Another cell: a plain click.
        let t2 = t1 + Duration::from_secs(5);
        click(&mut app, 12, 1, t2);
        click(&mut app, 13, 1, t2 + Duration::from_millis(10));
        assert_eq!(app.buffer().selection(), None);

        // A third quick click is a plain click again.
        let t3 = t2 + Duration::from_secs(5);
        for i in 0..3 {
            click(&mut app, 6, 1, t3 + Duration::from_millis(i * 10));
        }
        assert_eq!(app.buffer().selection(), None);
        assert_eq!(app.buffer().cursor, 1);
    }

    #[test]
    fn drag_selects_from_press_to_release() {
        let mut app = app_with("hello world", &FakeClipboard::default());
        let now = Instant::now();
        app.handle_mouse(mouse(LEFT_DOWN, 11, 1), now);
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 7, 1), now);
        app.handle_mouse(mouse(LEFT_UP, 7, 1), now);
        assert_eq!(app.buffer().selected_text().as_deref(), Some("llo "));
        assert_eq!(app.buffer().cursor, 2);
        // A drag without a press in the editor selects nothing.
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 15, 1), now);
        assert_eq!(app.buffer().cursor, 2);
    }

    #[test]
    fn ctrl_click_is_not_a_click() {
        let mut app = app_with("hello", &FakeClipboard::default());
        let mut event = mouse(LEFT_DOWN, 8, 1);
        event.modifiers = KeyModifiers::CONTROL;
        app.handle_mouse(event, Instant::now());
        assert_eq!(app.buffer().cursor, 0);
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
        app.handle_event(AppEvent::FilesListed(vec!["a.txt".into()]));
        assert!(app.picker.is_none());
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
        // " untitled " is 10 cells wide; the second tab starts at column 10.
        app.handle_mouse(mouse(LEFT_DOWN, 2, 0), Instant::now());
        assert_eq!(app.tabs.split().active, 0);
        let middle = MouseEventKind::Down(MouseButton::Middle);
        app.handle_mouse(mouse(middle, 12, 0), Instant::now());
        assert_eq!(app.tabs.split().tabs.len(), 1);
        assert_eq!(app.buffer().rope.to_string(), "one");
        press(&mut app, &["x"]);
        app.handle_mouse(mouse(middle, 2, 0), Instant::now());
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
            app.confirm(Prompt::Trash).text(),
            "Move a.txt to trash? y / n"
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
        assert_eq!(app.panes().splits[0].editor.height, 27);
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
    fn the_tree_takes_31_columns_from_the_editor_while_shown() -> Result<()> {
        let (_dir, mut app) = project()?;
        assert_eq!(app.panes().splits[0].editor, Rect::new(31, 1, 69, 28));
        assert_eq!(app.panes().splits[0].tabs, Rect::new(31, 0, 69, 1));
        assert_eq!(app.panes().tree, Some(Rect::new(0, 1, 30, 28)));
        assert_eq!(app.panes().divider, Some(Rect::new(30, 0, 1, 29)));
        press(&mut app, &["ctrl+b"]);
        assert_eq!(app.panes().splits[0].editor, Rect::new(0, 1, 100, 28));
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
        assert_eq!(panes.splits[0].editor, Rect::new(0, 1, 50, 28));
        assert_eq!(panes.splits[1].editor, Rect::new(51, 1, 49, 28));
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
        // The left split's editor starts at column 31.
        click(&mut app, 40, 1, Instant::now());
        assert_eq!(app.tabs.focused, 0);
        // Opening b.txt from the tree puts it in the left split only.
        press(&mut app, &["ctrl+e", "down", "enter"]);
        assert_eq!(tab_names(&app), ["a.txt", "b.txt"]);
        assert_eq!(app.tabs.labels(1).len(), 1);
        // A click on the right split's tab bar focuses that split.
        let right = app.panes().splits[1].tabs;
        app.handle_mouse(mouse(LEFT_DOWN, right.x + 1, 0), Instant::now());
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
        assert!(last.starts_with("griffin"), "{last}");
        assert!(last.contains("config error: boom"), "{last}");
        assert!(last.contains("untitled"), "{last}");
        Ok(())
    }

    #[test]
    fn status_line_is_on_the_last_row() -> Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        let app = App::default();
        terminal.draw(|frame| app.render(frame))?;

        let buffer = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..100).map(|x| buffer[(x, y)].symbol()).collect() };
        // The tab bar, then an empty buffer: one numbered line and nothing below.
        assert_eq!(row(0).trim(), "untitled");
        assert_eq!(row(1).trim(), "1 │");
        for y in 2..29 {
            assert_eq!(row(y).trim(), "", "row {y} should be blank");
        }
        assert!(row(29).starts_with("griffin"));
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
}
