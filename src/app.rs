use anyhow::Result;
use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use tokio::sync::mpsc;

use crate::Tui;
use crate::ui::status::render_status;

/// Everything the event loop reacts to. Background work (LSP, run output, timers)
/// adds variants here instead of touching `App` (ADR-0001).
#[derive(Debug)]
pub enum AppEvent {
    Input(Event),
}

/// All editor state, owned by the main task.
#[derive(Debug, Default)]
pub struct App {
    should_quit: bool,
}

impl App {
    pub fn new() -> Self {
        Self::default()
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
        terminal.draw(|frame| self.render(frame))?;
        while !self.should_quit {
            let Some(event) = rx.recv().await else {
                // The input task ended (stdin closed or errored); nothing more can
                // arrive, so stop rather than hang.
                break;
            };
            self.handle_event(event);
            terminal.draw(|frame| self.render(frame))?;
        }
        Ok(())
    }

    fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Input(Event::Key(key)) => self.handle_key(key),
            AppEvent::Input(_) => {}
        }
    }

    // Temporary: Ctrl+Q is matched here until #3 moves key handling into
    // src/keymap.rs (plan rule R5, "not yet: #3").
    fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Press
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q'))
        {
            self.should_quit = true;
        }
    }

    /// Draws the whole screen. Pure: reads `self`, never changes it.
    pub fn render(&self, frame: &mut Frame) {
        let [_editor, status] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(frame.area());
        render_status(frame, status);
    }
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

    fn key(code: KeyCode, modifiers: KeyModifiers) -> AppEvent {
        AppEvent::Input(Event::Key(KeyEvent::new(code, modifiers)))
    }

    #[test]
    fn ctrl_q_quits() {
        let mut app = App::new();
        app.handle_event(key(KeyCode::Char('q'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn plain_q_does_not_quit() {
        let mut app = App::new();
        app.handle_event(key(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(!app.should_quit);
    }

    #[test]
    fn key_release_does_not_quit() {
        let mut app = App::new();
        let mut release = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        release.kind = KeyEventKind::Release;
        app.handle_event(AppEvent::Input(Event::Key(release)));
        assert!(!app.should_quit);
    }

    #[test]
    fn status_line_is_on_the_last_row() -> Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        let app = App::new();
        terminal.draw(|frame| app.render(frame))?;

        let buffer = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..100).map(|x| buffer[(x, y)].symbol()).collect() };
        for y in 0..29 {
            assert_eq!(row(y).trim(), "", "row {y} should be blank");
        }
        assert!(row(29).starts_with("griffin"));
        Ok(())
    }
}
