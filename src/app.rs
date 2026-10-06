use std::path::PathBuf;
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
use crate::buffer::Buffer;
use crate::clipboard::Clipboard;
use crate::config::EditorConfig;
#[cfg(windows)]
use crate::keymap::burst_as_paste;
use crate::keymap::{Action, Input, Keymap};
use crate::ui::confirm::{Answer, Choice, Confirm};
use crate::ui::status::render_status;
use crate::view::{View, render_buffer};

/// Everything the event loop reacts to. Background work (LSP, run output, timers)
/// adds variants here instead of touching `App` (ADR-0001).
#[derive(Debug)]
pub enum AppEvent {
    Input(Event),
}

/// A question that takes over the keyboard until it's answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Prompt {
    /// Ctrl+Q with unsaved changes.
    UnsavedQuit,
}

const UNSAVED_QUIT: Confirm = Confirm {
    question: "Unsaved changes",
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
};

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

impl Prompt {
    fn confirm(self) -> Confirm {
        match self {
            Prompt::UnsavedQuit => UNSAVED_QUIT,
        }
    }
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
    clipboard: Box<dyn Clipboard>,
    /// The previous left press, until a double-click uses it up.
    last_click: Option<Click>,
    /// Char index a left press landed on while the button is held, so a drag
    /// selects from there.
    drag_from: Option<usize>,
    should_quit: bool,
}

impl App {
    /// `path` is what the command line named, if anything. A file that can't be
    /// opened leaves an untitled buffer and says why in the status line.
    pub fn new(
        keymap: Keymap,
        editor: EditorConfig,
        path: Option<PathBuf>,
        message: Option<String>,
    ) -> Self {
        let mut messages: Vec<String> = message.into_iter().collect();
        let buffer = match path {
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
            clipboard: Box::default(),
            last_click: None,
            drag_from: None,
            should_quit: false,
        }
    }

    pub async fn run(&mut self, terminal: &mut Tui) -> Result<()> {
        let (tx, mut rx) = mpsc::unbounded_channel();
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
            // Bracketed paste bypasses the keymap: it is text, not a key. Windows
            // Terminal's own Ctrl+V paste arrives this way too.
            AppEvent::Input(Event::Paste(text)) if self.prompt.is_none() => {
                self.edit(|buffer| buffer.paste(&text));
            }
            // The mouse bypasses the keymap too: only keys are remappable.
            AppEvent::Input(Event::Mouse(mouse)) if self.prompt.is_none() => {
                self.handle_mouse(mouse, Instant::now());
            }
            // A smaller pane can leave the cursor outside it.
            AppEvent::Input(Event::Resize(..)) => self.follow_cursor(),
            AppEvent::Input(_) => {}
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        let input = self.keymap.resolve(&key);
        if let Some(prompt) = self.prompt {
            if let Some(answer) = prompt.confirm().answer(input) {
                self.answer_prompt(prompt, answer);
            }
            return;
        }
        match input {
            Input::Action(action) => self.handle_action(action),
            Input::Text(ch) => {
                self.buffer.type_text(ch.encode_utf8(&mut [0; 4]));
                self.follow_cursor();
            }
            Input::Ignored => {}
        }
    }

    fn handle_action(&mut self, action: Action) {
        // Enter joins the run of typing it ends; every other action closes it.
        if action != Action::Newline {
            self.buffer.seal_undo_group();
        }
        match action {
            Action::Quit if self.buffer.dirty => self.prompt = Some(Prompt::UnsavedQuit),
            Action::Quit => self.should_quit = true,
            Action::Save => {
                self.save();
            }
            // Nothing to cancel outside a prompt.
            Action::Cancel => {}
            Action::Move(motion) => {
                let page = usize::from(editor_area(self.screen).height);
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
                let page = usize::from(editor_area(self.screen).height);
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
        }
    }

    /// Click, drag, double-click and wheel in the editor pane. `now` is when the
    /// event arrived, passed in so double-click timing is testable.
    fn handle_mouse(&mut self, mouse: MouseEvent, now: Instant) {
        let area = editor_area(self.screen);
        let (col, row) = (mouse.column, mouse.row);
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
                self.should_quit = self.save();
            }
            (Prompt::UnsavedQuit, Answer::Picked('d')) => self.should_quit = true,
            (Prompt::UnsavedQuit, _) => {}
        }
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
    }

    fn follow_cursor(&mut self) {
        self.view.follow(
            &self.buffer,
            editor_area(self.screen),
            self.editor.tab_width,
        );
    }

    /// Draws the whole screen. Pure: reads `self`, never changes it.
    pub fn render(&self, frame: &mut Frame) {
        let [editor, status] = screen_layout(frame.area());
        render_buffer(
            &self.buffer,
            &self.view,
            self.editor.tab_width,
            editor,
            frame,
        );
        render_status(
            frame,
            status,
            &self.buffer.name(),
            self.buffer.dirty,
            self.message.as_deref(),
            self.buffer.cursor_line_col(),
        );
        if let Some(prompt) = self.prompt {
            prompt.confirm().render(frame, frame.area());
        }
    }
}

/// The editor pane above a one-row status line.
fn screen_layout(screen: Rect) -> [Rect; 2] {
    Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(screen)
}

fn editor_area(screen: Rect) -> Rect {
    screen_layout(screen)[0]
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
