use std::borrow::Cow;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    Event, EventStream, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use futures_util::StreamExt;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use tokio::sync::mpsc;
#[cfg(windows)]
use tokio::time::timeout;

use crate::Tui;
use crate::backup::{self, Backups};
use crate::buffer::Buffer;
use crate::buffer::movement::Motion;
use crate::clipboard::Clipboard;
use crate::config::EditorConfig;
#[cfg(windows)]
use crate::keymap::burst_as_paste;
use crate::keymap::{Action, Input, Keymap, Scope};
use crate::ui::confirm::{Answer, Choice, Confirm, Labels};
use crate::ui::prompt::{Outcome, PromptBar};
use crate::ui::status::render_status;
use crate::ui::tree::{TREE_WIDTH, render_divider, render_tree};
use crate::view::{View, render_buffer};
use crate::workspace::ops::{self, Trash};
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
}

/// A question that takes over the keyboard until it's answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prompt {
    /// Ctrl+Q with unsaved changes.
    UnsavedQuit,
    /// The file opened with a newer crash backup beside it.
    Recover,
    /// Opening another file from the tree with unsaved changes.
    UnsavedOpen,
    /// Moving the tree's selected entry to the trash.
    Trash,
}

/// What the prompt bar's answer will be used for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TreeOp {
    /// A new file in this folder.
    NewFile(PathBuf),
    /// A new folder in this folder.
    NewFolder(PathBuf),
    /// A new name for this file or folder.
    Rename(PathBuf),
}

/// The prompt bar while it asks for a name.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NamePrompt {
    op: TreeOp,
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

/// All editor state, owned by the main task.
#[derive(Debug, Default)]
pub struct App {
    keymap: Keymap,
    editor: EditorConfig,
    /// The one open buffer; tabs arrive in phase 2.
    buffer: Buffer,
    view: View,
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
    /// The file the unsaved-changes prompt will open once answered.
    pending_open: Option<PathBuf>,
    /// The prompt bar, while it's asking for a name; it takes every key.
    name_prompt: Option<NamePrompt>,
    /// The entry the trash prompt will move once answered.
    pending_trash: Option<PathBuf>,
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
            buffer,
            view: View::default(),
            screen: Rect::default(),
            message,
            prompt: None,
            tree: Tree::new(&launch.root),
            tree_visible: launch.show_tree,
            // With only a folder open there's nothing to edit yet.
            focus: if launch.show_tree {
                Focus::Tree
            } else {
                Focus::Editor
            },
            pending_open: None,
            name_prompt: None,
            pending_trash: None,
            trash: Box::default(),
            clipboard: Box::default(),
            last_click: None,
            drag_from: None,
            backups: Backups::default(),
            recovery: None,
            backup_failed: false,
            edits: None,
            events: None,
            should_quit: false,
        }
    }

    /// Turns crash backups on, and offers to recover the opened file's backup if
    /// it has a newer one.
    pub fn with_backups(mut self, backups: Backups) -> Self {
        self.backups = backups;
        if let Some(path) = &self.buffer.path
            && let Some(text) = self.backups.recoverable(path)
        {
            self.recovery = Some(text);
            self.prompt = Some(Prompt::Recover);
        }
        self
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
        self.screen = terminal.draw(|frame| self.render(frame))?.area;
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
            self.screen = terminal.draw(|frame| self.render(frame))?.area;
        }
        Ok(())
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Input(Event::Key(key)) => self.handle_key(key),
            AppEvent::Input(Event::Paste(text)) if self.prompt.is_none() => {
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
                if self.prompt.is_none() && self.name_prompt.is_none() =>
            {
                self.handle_mouse(mouse, Instant::now());
            }
            // A smaller pane can leave the cursor outside it.
            AppEvent::Input(Event::Resize(..)) => self.follow_cursor(),
            AppEvent::Input(_) => {}
            AppEvent::BackupDue => self.back_up(),
            AppEvent::BackupWritten(result) => self.backup_written(result),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Tree letters are only actions when nothing else is reading keys as text.
        let scope =
            if self.focus == Focus::Tree && self.prompt.is_none() && self.name_prompt.is_none() {
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
            Action::Quit | Action::Save | Action::ToggleTree | Action::FocusTree => {
                self.handle_action(action);
            }
            _ => {}
        }
        self.follow_tree();
    }

    /// Enter or a click on the selected row: a folder opens or closes, a file opens
    /// in the editor.
    fn activate_tree_row(&mut self) {
        let Some(row) = self.tree.selected_row() else {
            return;
        };
        if row.entry.is_dir {
            self.tree.toggle();
        } else {
            let path = row.entry.path.clone();
            self.request_open(path);
        }
    }

    /// Opens `path` in place of the current buffer (tabs come later), asking first
    /// when that would throw away unsaved changes.
    fn request_open(&mut self, path: PathBuf) {
        if self.buffer.dirty {
            self.pending_open = Some(path);
            self.prompt = Some(Prompt::UnsavedOpen);
        } else {
            self.open(&path);
        }
    }

    fn open(&mut self, path: &Path) {
        match Buffer::open(path) {
            Ok(buffer) => {
                // The old buffer goes without unsaved edits, so its backup can too.
                self.delete_backup();
                self.buffer = buffer;
                self.view = View::default();
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

    /// Rows one PageUp/PageDown moves in the tree.
    fn tree_page(&self) -> isize {
        isize::try_from(self.panes().editor.height).unwrap_or(isize::MAX)
    }

    fn follow_tree(&mut self) {
        if let Some(area) = self.panes().tree {
            self.tree.follow(usize::from(area.height));
        }
    }

    fn handle_action(&mut self, action: Action) {
        // Enter joins the run of typing it ends; every other action closes it.
        if action != Action::Newline {
            self.buffer.seal_undo_group();
        }
        match action {
            Action::Quit if self.buffer.dirty => self.prompt = Some(Prompt::UnsavedQuit),
            Action::Quit => self.quit(),
            Action::Save => {
                self.save();
            }
            // Nothing to cancel outside a prompt.
            Action::Cancel => {}
            Action::Move(motion) => {
                let page = usize::from(self.panes().editor.height);
                self.buffer.move_cursor(motion, page, self.editor.tab_width);
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
                let page = usize::from(self.panes().editor.height);
                self.buffer.select(motion, page, self.editor.tab_width);
                self.follow_cursor();
            }
            Action::SelectAll => {
                self.buffer.select_all();
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
            Prompt::UnsavedQuit | Prompt::UnsavedOpen => UNSAVED_QUIT,
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
            (TreeOp::NewFolder(dir), "New folder")
        } else {
            (TreeOp::NewFile(dir), "New file")
        };
        self.open_name_prompt(op, PromptBar::new(label, ""));
    }

    fn start_rename(&mut self) {
        let Some(row) = self.tree.selected_row() else {
            return;
        };
        let bar = PromptBar::new("Rename", &row.entry.name);
        self.open_name_prompt(TreeOp::Rename(row.entry.path.clone()), bar);
    }

    fn open_name_prompt(&mut self, op: TreeOp, bar: PromptBar) {
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
        if outcome == Outcome::Cancel {
            return;
        }
        let name = bar.text();
        let result = match &op {
            TreeOp::NewFile(dir) => ops::create_file(dir, name),
            TreeOp::NewFolder(dir) => ops::create_folder(dir, name),
            TreeOp::Rename(path) => ops::rename(path, name),
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
            TreeOp::NewFile(_) => {
                self.message = Some(format!("created {name}"));
                self.request_open(path);
            }
            TreeOp::NewFolder(_) => self.message = Some(format!("created {name}")),
            TreeOp::Rename(old) => {
                self.message = Some(format!("renamed to {name}"));
                self.follow_rename(&old, &path);
            }
        }
    }

    /// Points the open buffer at its new path when it, or a folder holding it, was
    /// renamed, moving its crash backup along with it.
    fn follow_rename(&mut self, old: &Path, new: &Path) {
        let Some(moved) = self
            .buffer
            .path
            .as_deref()
            .and_then(|path| ops::rebase(path, old, new))
        else {
            return;
        };
        // Backups are keyed by path, so the old one would never be found again.
        self.delete_backup();
        self.buffer.path = Some(moved);
        self.back_up();
    }

    /// The trash prompt's yes: moves the entry to the OS trash and closes the open
    /// buffer if it was that file or inside that folder.
    fn trash_pending(&mut self) {
        let Some(path) = self.pending_trash.take() else {
            return;
        };
        let name = file_name(&path);
        if let Err(err) = self.trash.delete(&path) {
            self.message = Some(format!("cannot move {name} to trash: {err}"));
            return;
        }
        let open = self.buffer.path.as_deref();
        if open.is_some_and(|open| open.starts_with(&path)) {
            self.delete_backup();
            self.buffer = Buffer::empty();
            self.view = View::default();
        }
        self.message = Some(format!("moved {name} to trash"));
        self.tree.reload(None);
        self.follow_tree();
    }

    /// Click, drag, double-click and wheel in the editor pane. `now` is when the
    /// event arrived, passed in so double-click timing is testable.
    fn handle_mouse(&mut self, mouse: MouseEvent, now: Instant) {
        let panes = self.panes();
        let area = panes.editor;
        let (col, row) = (mouse.column, mouse.row);
        if let Some(tree) = panes.tree
            && tree.contains(Position::new(col, row))
        {
            self.handle_tree_mouse(mouse, tree);
            return;
        }
        let inside = area.contains(Position::new(col, row));
        let tab_width = self.editor.tab_width;
        let pos = |app: &Self| {
            app.view
                .screen_to_char(&app.buffer, area, col, row, tab_width)
        };
        match mouse.kind {
            // Ctrl+click is left for go to definition (#32).
            MouseEventKind::Down(MouseButton::Left)
                if inside && !mouse.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.focus = Focus::Editor;
                let pos = pos(self);
                let double = self.last_click.is_some_and(|last| {
                    (last.col, last.row) == (col, row)
                        && now.saturating_duration_since(last.at) <= DOUBLE_CLICK
                });
                if double {
                    self.buffer.select_word_at(pos);
                    // A third click starts over rather than counting as another double.
                    self.last_click = None;
                    self.drag_from = None;
                } else {
                    self.buffer.place_cursor(pos);
                    self.last_click = Some(Click { at: now, col, row });
                    self.drag_from = Some(pos);
                }
                self.follow_cursor();
            }
            MouseEventKind::Down(_) => self.drag_from = None,
            MouseEventKind::Drag(MouseButton::Left) => {
                if let Some(from) = self.drag_from {
                    let pos = pos(self);
                    self.buffer.select_to(from, pos);
                    self.follow_cursor();
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                // A release somewhere new without a drag event in between still
                // ends the selection there.
                if let Some(from) = self.drag_from.take() {
                    let pos = pos(self);
                    if pos != self.buffer.cursor {
                        self.buffer.select_to(from, pos);
                        self.follow_cursor();
                    }
                }
            }
            MouseEventKind::ScrollUp if inside => {
                self.view.scroll_by(&self.buffer, area, -WHEEL_LINES);
            }
            MouseEventKind::ScrollDown if inside => {
                self.view.scroll_by(&self.buffer, area, WHEEL_LINES);
            }
            _ => {}
        }
    }

    /// A click selects the row under it and opens it (a folder opens or closes, a
    /// file opens in the editor); the wheel scrolls the tree.
    fn handle_tree_mouse(&mut self, mouse: MouseEvent, area: Rect) {
        // Whatever the editor was tracking for a drag or double-click is over.
        self.drag_from = None;
        self.last_click = None;
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
        let Some(text) = self.buffer.selected_text() else {
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
                // A failed save leaves the editor open with the error showing.
                if self.save() {
                    self.quit();
                }
            }
            (Prompt::UnsavedQuit, Answer::Picked('d')) => self.quit(),
            (Prompt::UnsavedQuit, _) => {}
            (Prompt::UnsavedOpen, Answer::Picked(choice @ ('s' | 'd'))) => {
                let Some(path) = self.pending_open.take() else {
                    return;
                };
                // A failed save leaves the old buffer open with the error showing.
                if choice == 'd' || self.save() {
                    self.open(&path);
                }
            }
            (Prompt::UnsavedOpen, _) => self.pending_open = None,
            (Prompt::Trash, Answer::Picked('y')) => self.trash_pending(),
            (Prompt::Trash, _) => self.pending_trash = None,
            (Prompt::Recover, Answer::Picked('r')) => {
                if let Some(text) = self.recovery.take() {
                    self.buffer.recover(&text);
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

    /// Ends the session cleanly, which means the buffer's backup isn't needed.
    fn quit(&mut self) {
        self.delete_backup();
        self.should_quit = true;
    }

    /// Starts writing the buffer's crash backup, if it has unsaved changes. The
    /// write runs on a blocking thread with a copy of the text (ADR-0001).
    fn back_up(&mut self) {
        if !self.buffer.dirty {
            return;
        }
        let Some(job) = self
            .backups
            .job(self.buffer.path.as_deref(), self.buffer.disk_text())
        else {
            return;
        };
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

    fn backup_written(&mut self, result: io::Result<()>) {
        match result {
            Ok(()) => self.backup_failed = false,
            Err(err) if !self.backup_failed => {
                self.backup_failed = true;
                self.message = Some(format!("cannot back up {}: {err}", self.buffer.name()));
            }
            Err(_) => {}
        }
    }

    fn delete_backup(&mut self) {
        // Best effort: a leftover backup is only offered again if it is newer than
        // the file and differs from it, so failing here loses nothing.
        let _ = self.backups.delete(self.buffer.path.as_deref());
    }

    /// Saves the buffer and says how it went in the status line. Returns whether
    /// the buffer is now saved.
    fn save(&mut self) -> bool {
        if self.buffer.path.is_none() {
            self.message = Some("no file name (save as comes later)".into());
            return false;
        }
        let name = self.buffer.name();
        match self.buffer.save() {
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
        edit(&mut self.buffer);
        self.follow_cursor();
        if let Some(edits) = &self.edits {
            // The timer only stops with the loop, after which nothing edits.
            let _ = edits.send(());
        }
    }

    fn follow_cursor(&mut self) {
        self.view
            .follow(&self.buffer, self.panes().editor, self.editor.tab_width);
    }

    fn panes(&self) -> Panes {
        Panes::new(self.screen, self.tree_visible, self.name_prompt.is_some())
    }

    /// Draws the whole screen. Pure: reads `self`, never changes it.
    pub fn render(&self, frame: &mut Frame) {
        let panes = Panes::new(frame.area(), self.tree_visible, self.name_prompt.is_some());
        render_buffer(
            &self.buffer,
            &self.view,
            self.editor.tab_width,
            panes.editor,
            frame,
        );
        if let (Some(tree), Some(divider)) = (panes.tree, panes.divider) {
            let focused = self.focus == Focus::Tree;
            let selected = render_tree(&self.tree, focused, tree, frame);
            render_divider(divider, frame);
            // The cursor marks the focused pane; `render_buffer` put it in the editor.
            if focused && let Some(at) = selected {
                frame.set_cursor_position(at);
            }
        }
        render_status(
            frame,
            panes.status,
            &self.buffer.name(),
            self.buffer.dirty,
            self.message.as_deref(),
            self.buffer.cursor_line_col(),
        );
        if let (Some(name_prompt), Some(area)) = (&self.name_prompt, panes.bar) {
            let at = name_prompt.bar.render(frame, area);
            frame.set_cursor_position(at);
        }
        if let Some(prompt) = self.prompt {
            self.confirm(prompt).render(frame, frame.area());
        }
    }
}

/// The last part of `path`, for messages.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Where each part of the screen goes: the tree (when shown), a `│` divider and
/// the editor side by side, above the prompt bar (while open) and a one-row status
/// line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Panes {
    tree: Option<Rect>,
    divider: Option<Rect>,
    editor: Rect,
    bar: Option<Rect>,
    status: Rect,
}

impl Panes {
    fn new(screen: Rect, tree_visible: bool, bar_open: bool) -> Self {
        let [main, bar, status] = Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(u16::from(bar_open)),
            Constraint::Length(1),
        ])
        .areas(screen);
        let bar = bar_open.then_some(bar);
        if !tree_visible {
            return Panes {
                tree: None,
                divider: None,
                editor: main,
                bar,
                status,
            };
        }
        let [tree, divider, editor] = Layout::horizontal([
            Constraint::Length(TREE_WIDTH),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .areas(main);
        Panes {
            tree: Some(tree),
            divider: Some(divider),
            editor,
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
        assert_eq!(app.buffer.rope.to_string(), "x");
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
    fn save_then_quit_on_an_untitled_buffer_stays_open() {
        let mut app = App::default();
        app.handle_event(key("x"));
        app.handle_event(key("ctrl+q"));
        app.handle_event(key("s"));
        assert!(!app.should_quit);
        assert_eq!(
            app.message.as_deref(),
            Some("no file name (save as comes later)")
        );
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
        assert!(app.buffer.dirty);
        app.handle_event(key("ctrl+s"));
        assert!(!app.buffer.dirty);
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
            buffer: Buffer {
                rope: ropey::Rope::from_str(&text),
                ..Buffer::empty()
            },
            screen: Rect::new(0, 0, 100, 30),
            ..App::default()
        };
        app.handle_event(key("pagedown"));
        assert_eq!(app.buffer.cursor_line_col(), (29, 0));
        assert_eq!(app.view.scroll_row, 1);
        app.handle_event(key("ctrl+end"));
        assert_eq!(app.buffer.cursor_line_col(), (100, 0));
        assert_eq!(app.view.scroll_row, 72);
        app.handle_event(key("ctrl+home"));
        assert_eq!(app.buffer.cursor, 0);
        assert_eq!(app.view.scroll_row, 0);
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
        assert_eq!(app.buffer.rope.to_string(), "fn {\n    y");
        assert!(app.buffer.dirty);
    }

    #[test]
    fn ctrl_z_and_ctrl_y_undo_and_redo() {
        let mut app = App {
            screen: Rect::new(0, 0, 100, 30),
            ..App::default()
        };
        for k in ["a", "b", "ctrl+s", "c", "d"] {
            app.handle_event(key(k));
        }
        // Saving is a non-typing action, so it split the run.
        app.handle_event(key("ctrl+z"));
        assert_eq!(app.buffer.rope.to_string(), "ab");
        app.handle_event(key("ctrl+z"));
        assert_eq!(app.buffer.rope.to_string(), "");
        app.handle_event(key("ctrl+y"));
        assert_eq!(app.buffer.rope.to_string(), "ab");
        assert_eq!(app.buffer.cursor, 2);
    }

    fn app_with(text: &str, clipboard: &FakeClipboard) -> App {
        App {
            buffer: Buffer {
                rope: ropey::Rope::from_str(text),
                ..Buffer::empty()
            },
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
        assert_eq!(app.buffer.selected_text().as_deref(), Some("hello"));
        press(&mut app, &["b", "y", "e"]);
        assert_eq!(app.buffer.rope.to_string(), "bye world");
        press(&mut app, &["ctrl+z"]);
        assert_eq!(app.buffer.rope.to_string(), "hello world");
    }

    #[test]
    fn a_plain_movement_clears_the_selection() {
        let mut app = app_with("hello", &FakeClipboard::default());
        press(&mut app, &["ctrl+shift+right", "left"]);
        assert_eq!(app.buffer.selection(), None);
        assert_eq!(app.buffer.cursor, 4);
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
        assert_eq!(app.buffer.rope.to_string(), "hello world");
        assert!(!app.buffer.dirty);
    }

    #[test]
    fn ctrl_x_copies_and_deletes_as_one_step() {
        let clipboard = FakeClipboard::default();
        let mut app = app_with("one\ntwo", &clipboard);
        press(&mut app, &["ctrl+a", "ctrl+x"]);
        assert_eq!(*clipboard.text.borrow(), "one\ntwo");
        assert_eq!(app.buffer.rope.to_string(), "");
        press(&mut app, &["ctrl+z"]);
        assert_eq!(app.buffer.rope.to_string(), "one\ntwo");
        // Nothing selected: cut leaves the clipboard and the text alone.
        press(&mut app, &["ctrl+x"]);
        assert_eq!(app.buffer.rope.to_string(), "one\ntwo");
    }

    #[test]
    fn ctrl_v_pastes_normalised_text_as_one_step() {
        let clipboard = FakeClipboard::default();
        *clipboard.text.borrow_mut() = "a\r\nb\r\n".into();
        let mut app = app_with("xy", &clipboard);
        press(&mut app, &["right", "ctrl+v"]);
        assert_eq!(app.buffer.rope.to_string(), "xa\nb\ny");
        press(&mut app, &["ctrl+z"]);
        assert_eq!(app.buffer.rope.to_string(), "xy");
        // Pasting over a selection replaces it, still one step.
        press(&mut app, &["ctrl+a", "ctrl+v"]);
        assert_eq!(app.buffer.rope.to_string(), "a\nb\n");
        press(&mut app, &["ctrl+z"]);
        assert_eq!(app.buffer.rope.to_string(), "xy");
    }

    #[test]
    fn copy_then_paste_round_trips_through_the_clipboard() {
        let clipboard = FakeClipboard::default();
        let mut app = app_with("ab", &clipboard);
        press(
            &mut app,
            &["shift+right", "ctrl+c", "end", "ctrl+v", "ctrl+v"],
        );
        assert_eq!(app.buffer.rope.to_string(), "abaa");
    }

    #[test]
    fn bracketed_paste_is_one_step_replacing_the_selection() {
        let mut app = app_with("hello world", &FakeClipboard::default());
        press(&mut app, &["shift+end"]);
        app.handle_event(AppEvent::Input(Event::Paste("bye\r\nnow".into())));
        assert_eq!(app.buffer.rope.to_string(), "bye\nnow");
        press(&mut app, &["ctrl+z"]);
        assert_eq!(app.buffer.rope.to_string(), "hello world");
    }

    #[test]
    fn bracketed_paste_is_ignored_while_a_prompt_is_open() {
        let mut app = app_with("", &FakeClipboard::default());
        press(&mut app, &["x", "ctrl+q"]);
        app.handle_event(AppEvent::Input(Event::Paste("d".into())));
        assert_eq!(app.buffer.rope.to_string(), "x");
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
        // Gutter " 1 │ " is 5 cells.
        let mut app = app_with(
            "hello
world",
            &FakeClipboard::default(),
        );
        click(&mut app, 7, 1, Instant::now());
        assert_eq!(app.buffer.cursor_line_col(), (1, 2));
        assert_eq!(app.buffer.selection(), None);
        // The status line isn't the editor.
        click(&mut app, 5, 29, Instant::now());
        assert_eq!(app.buffer.cursor_line_col(), (1, 2));
    }

    #[test]
    fn double_click_needs_the_same_cell_within_400_ms() {
        let mut app = app_with("hello world", &FakeClipboard::default());
        let t0 = Instant::now();
        click(&mut app, 6, 0, t0);
        click(&mut app, 6, 0, t0 + Duration::from_millis(400));
        assert_eq!(app.buffer.selected_text().as_deref(), Some("hello"));

        // Too slow: a second plain click.
        let t1 = t0 + Duration::from_secs(5);
        click(&mut app, 12, 0, t1);
        click(&mut app, 12, 0, t1 + Duration::from_millis(401));
        assert_eq!(app.buffer.selection(), None);
        assert_eq!(app.buffer.cursor, 7);

        // Another cell: a plain click.
        let t2 = t1 + Duration::from_secs(5);
        click(&mut app, 12, 0, t2);
        click(&mut app, 13, 0, t2 + Duration::from_millis(10));
        assert_eq!(app.buffer.selection(), None);

        // A third quick click is a plain click again.
        let t3 = t2 + Duration::from_secs(5);
        for i in 0..3 {
            click(&mut app, 6, 0, t3 + Duration::from_millis(i * 10));
        }
        assert_eq!(app.buffer.selection(), None);
        assert_eq!(app.buffer.cursor, 1);
    }

    #[test]
    fn drag_selects_from_press_to_release() {
        let mut app = app_with("hello world", &FakeClipboard::default());
        let now = Instant::now();
        app.handle_mouse(mouse(LEFT_DOWN, 11, 0), now);
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 7, 0), now);
        app.handle_mouse(mouse(LEFT_UP, 7, 0), now);
        assert_eq!(app.buffer.selected_text().as_deref(), Some("llo "));
        assert_eq!(app.buffer.cursor, 2);
        // A drag without a press in the editor selects nothing.
        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 15, 0), now);
        assert_eq!(app.buffer.cursor, 2);
    }

    #[test]
    fn ctrl_click_is_not_a_click() {
        let mut app = app_with("hello", &FakeClipboard::default());
        let mut event = mouse(LEFT_DOWN, 8, 0);
        event.modifiers = KeyModifiers::CONTROL;
        app.handle_mouse(event, Instant::now());
        assert_eq!(app.buffer.cursor, 0);
    }

    #[test]
    fn wheel_scrolls_without_moving_the_cursor() {
        let text: String = (1..=100).map(|n| format!("line {n}\n")).collect();
        let mut app = app_with(&text, &FakeClipboard::default());
        app.handle_mouse(mouse(MouseEventKind::ScrollDown, 10, 10), Instant::now());
        assert_eq!(app.view.scroll_row, 3);
        assert_eq!(app.buffer.cursor, 0);
        app.handle_mouse(mouse(MouseEventKind::ScrollUp, 10, 10), Instant::now());
        app.handle_mouse(mouse(MouseEventKind::ScrollUp, 10, 10), Instant::now());
        assert_eq!(app.view.scroll_row, 0);
    }

    #[test]
    fn mouse_is_ignored_while_a_prompt_is_open() {
        let mut app = app_with("", &FakeClipboard::default());
        press(&mut app, &["x", "x", "ctrl+q"]);
        app.handle_event(AppEvent::Input(Event::Mouse(mouse(LEFT_DOWN, 5, 0))));
        assert_eq!(app.buffer.cursor, 2);
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
                .path_for(Some(&self.files.path().join("a.txt")))
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
        t.app.buffer.dirty = false;
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
        assert_eq!(app.buffer.rope.to_string(), "ab");
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
        assert_eq!(app.buffer.rope.to_string(), "hi\n");
        // Esc doesn't decide for the user.
        press(&mut app, &["esc", "x"]);
        assert_eq!(app.prompt, Some(Prompt::Recover));
        press(&mut app, &["r"]);
        assert_eq!(app.prompt, None);
        assert_eq!(app.buffer.rope.to_string(), "xhi\n");
        assert!(app.buffer.dirty);
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
        assert_eq!(app.buffer.rope.to_string(), "hi\n");
        assert!(!app.buffer.dirty);
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
        assert_eq!(app.buffer.rope.to_string(), "");
        assert!(!app.buffer.dirty);
        Ok(())
    }

    #[test]
    fn opening_from_the_tree_saves_first_when_asked() -> Result<()> {
        let (dir, mut app) = project()?;
        press(&mut app, &["enter"]);
        assert_eq!(app.buffer.rope.to_string(), "a");
        assert_eq!(app.focus, Focus::Editor);
        press(&mut app, &["x", "ctrl+e", "down", "enter"]);
        assert_eq!(app.prompt, Some(Prompt::UnsavedOpen));
        press(&mut app, &["s"]);
        assert_eq!(app.prompt, None);
        assert_eq!(std::fs::read_to_string(dir.path().join("a.txt"))?, "xa");
        assert_eq!(app.buffer.rope.to_string(), "b");
        Ok(())
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
        assert_eq!(app.buffer.path.as_deref(), Some(a.as_path()));

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
        assert_eq!(app.buffer.path.as_deref(), Some(a.as_path()));

        press(&mut app, &["d", "y"]);
        assert_eq!(trash.trashed.borrow().as_slice(), std::slice::from_ref(&a));
        assert!(!a.exists());
        assert_eq!(app.buffer.path, None);
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
        assert_eq!(app.buffer.rope.to_string(), "a");
        assert_eq!(app.tree.rows().len(), 2);
        Ok(())
    }

    #[test]
    fn tree_letters_type_into_the_editor_and_the_bar() -> Result<()> {
        let (dir, mut app) = project()?;
        // In the tree `a` opens the bar; inside the bar `a`, `r`, `d` are text.
        press(&mut app, &["a"]);
        assert!(app.name_prompt.is_some());
        assert_eq!(app.panes().editor.height, 28);
        type_keys(&mut app, "dra.txt");
        press(&mut app, &["enter"]);
        assert_eq!(app.name_prompt, None);
        assert!(dir.path().join("dra.txt").is_file());
        // The new file is open with focus in the editor, where letters are text.
        assert_eq!(app.focus, Focus::Editor);
        type_keys(&mut app, "ard");
        assert_eq!(app.buffer.rope.to_string(), "ard");
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
        assert_eq!(app.buffer.path.as_deref(), Some(renamed.as_path()));
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
        assert_eq!(app.panes().editor, Rect::new(31, 0, 69, 29));
        assert_eq!(app.panes().tree, Some(Rect::new(0, 0, 30, 29)));
        press(&mut app, &["ctrl+b"]);
        assert_eq!(app.panes().editor, Rect::new(0, 0, 100, 29));
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
        // An empty buffer has one numbered line and nothing below it.
        assert_eq!(row(0).trim(), "1 │");
        for y in 1..29 {
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
        assert!(app.buffer.path.is_none());
        let message = app.message.unwrap_or_default();
        assert!(message.contains("config error: boom"), "{message}");
        assert!(
            message.contains(&format!("cannot open {}: not UTF-8", path.display())),
            "{message}"
        );
        Ok(())
    }
}
