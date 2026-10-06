//! The one-row prompt bar above the status line: a label and a single line of
//! text being typed, e.g. `New file: notes.md`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

use crate::buffer::movement::Motion;
use crate::keymap::{Action, Input};
use crate::theme::Theme;

/// What a key did to the bar, when it did more than edit the text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Enter: the text is the answer.
    Submit,
    /// Esc: close the bar and do nothing.
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptBar {
    pub label: &'static str,
    text: String,
    /// Char index into `text`.
    cursor: usize,
}

impl PromptBar {
    /// A bar holding `text` with the cursor at its end.
    pub fn new(label: &'static str, text: &str) -> Self {
        PromptBar {
            label,
            text: text.to_string(),
            cursor: text.chars().count(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Edits the text for one key; Enter and Esc are left to the caller.
    pub fn handle(&mut self, input: Input) -> Option<Outcome> {
        match input {
            Input::Text(c) => self.insert(c),
            Input::Action(Action::Newline) => return Some(Outcome::Submit),
            Input::Action(Action::Cancel) => return Some(Outcome::Cancel),
            Input::Action(Action::Backspace) if self.cursor > 0 => {
                self.cursor -= 1;
                self.remove_at(self.cursor);
            }
            Input::Action(Action::Delete) => self.remove_at(self.cursor),
            Input::Action(Action::Move(motion)) => {
                let len = self.text.chars().count();
                self.cursor = match motion {
                    Motion::Left => self.cursor.saturating_sub(1),
                    Motion::Right => (self.cursor + 1).min(len),
                    Motion::LineStart | Motion::DocStart => 0,
                    Motion::LineEnd | Motion::DocEnd => len,
                    _ => self.cursor,
                };
            }
            _ => {}
        }
        None
    }

    /// Pasted text goes in as if typed. A line break submits, since the bar holds
    /// one line and on Windows typing followed quickly by Enter arrives as a paste.
    pub fn paste(&mut self, text: &str) -> Option<Outcome> {
        for c in text.chars() {
            match c {
                '\n' => return Some(Outcome::Submit),
                c if c.is_control() => {}
                c => self.insert(c),
            }
        }
        None
    }

    fn insert(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.text.insert(at, c);
        self.cursor += 1;
    }

    fn remove_at(&mut self, index: usize) {
        if index < self.text.chars().count() {
            let at = self.byte_at(index);
            self.text.remove(at);
        }
    }

    fn byte_at(&self, index: usize) -> usize {
        self.text
            .char_indices()
            .nth(index)
            .map_or(self.text.len(), |(at, _)| at)
    }

    /// Draws `label: text` across `area` on `card2` and returns where the cursor
    /// goes.
    pub fn render(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        let head = format!("{}: ", self.label);
        let before: String = self.text.chars().take(self.cursor).collect();
        let line = format!("{head}{}", self.text);
        let width = usize::from(area.width);
        frame.buffer_mut().set_stringn(
            area.x,
            area.y,
            format!("{line:<width$}"),
            width,
            Style::new().bg(theme.card2).fg(theme.strong),
        );
        let col = u16::try_from(head.width() + before.width())
            .unwrap_or(u16::MAX)
            .min(area.width.saturating_sub(1));
        (area.x + col, area.y)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn act(action: Action) -> Input {
        Input::Action(action)
    }

    #[test]
    fn typing_and_editing_keys_change_the_text_at_the_cursor() {
        let mut bar = PromptBar::new("Rename", "notes.txt");
        for _ in 0..3 {
            assert_eq!(bar.handle(act(Action::Backspace)), None);
        }
        for c in "md".chars() {
            bar.handle(Input::Text(c));
        }
        assert_eq!(bar.text(), "notes.md");
        bar.handle(act(Action::Move(Motion::LineStart)));
        bar.handle(act(Action::Delete));
        bar.handle(Input::Text('N'));
        bar.handle(act(Action::Move(Motion::Right)));
        bar.handle(Input::Text('é'));
        assert_eq!(bar.text(), "Noétes.md");
        bar.handle(act(Action::Move(Motion::LineEnd)));
        bar.handle(act(Action::Delete));
        assert_eq!(bar.text(), "Noétes.md");
        assert_eq!(bar.handle(act(Action::Newline)), Some(Outcome::Submit));
        assert_eq!(bar.handle(act(Action::Cancel)), Some(Outcome::Cancel));
    }

    #[test]
    fn a_paste_with_a_line_break_submits_what_came_before() {
        let mut bar = PromptBar::new("New file", "");
        assert_eq!(bar.paste("notes.md\nrest"), Some(Outcome::Submit));
        assert_eq!(bar.text(), "notes.md");
        let mut bar = PromptBar::new("New file", "");
        assert_eq!(bar.paste("a\tb"), None);
        assert_eq!(bar.text(), "ab");
    }

    #[test]
    fn renders_label_text_and_cursor() -> anyhow::Result<()> {
        let bar = PromptBar::new("New file", "notes.md");
        let mut terminal = Terminal::new(TestBackend::new(30, 2))?;
        let mut at = (0, 0);
        terminal.draw(|frame| at = bar.render(&Theme::default(), frame, Rect::new(0, 1, 30, 1)))?;
        let buffer = terminal.backend().buffer();
        let row: String = (0..30).map(|x| buffer[(x, 1)].symbol()).collect();
        assert_eq!(row.trim_end(), "New file: notes.md");
        assert_eq!(at, (18, 1));
        Ok(())
    }
}
