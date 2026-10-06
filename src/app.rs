use std::path::PathBuf;

use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyEvent};
use futures_util::StreamExt;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use tokio::sync::mpsc;

use crate::Tui;
use crate::buffer::Buffer;
use crate::config::EditorConfig;
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
    while let Some(Ok(event)) = events.next().await {
        if tx.send(AppEvent::Input(event)).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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
