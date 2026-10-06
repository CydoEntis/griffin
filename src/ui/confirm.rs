//! A small centred card asking a question with one-letter answers, e.g.
//! `Unsaved changes: [S]ave [D]iscard [C]ancel`. Later dialogs (tree delete,
//! backup recovery) reuse it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

use crate::keymap::{Action, Input};

/// One answer: pressing `key` (either case) picks it; `label` starts with that
/// letter, which is shown in brackets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub key: char,
    pub label: &'static str,
}

/// What a key press means while the card is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Picked(char),
    /// Esc: close the card and do nothing.
    Dismissed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Confirm {
    pub question: &'static str,
    pub choices: &'static [Choice],
}

impl Confirm {
    /// The answer `input` gives, if any. Letters arrive as typed text, so the card
    /// never looks at raw key codes.
    pub fn answer(&self, input: Input) -> Option<Answer> {
        match input {
            Input::Text(c) => {
                let c = c.to_ascii_lowercase();
                self.choices
                    .iter()
                    .find(|choice| choice.key.to_ascii_lowercase() == c)
                    .map(|choice| Answer::Picked(choice.key))
            }
            Input::Action(Action::Cancel) => Some(Answer::Dismissed),
            _ => None,
        }
    }

    /// The card's one line of text.
    pub fn text(&self) -> String {
        let choices: Vec<String> = self
            .choices
            .iter()
            .map(|choice| {
                let mut chars = choice.label.chars();
                let first = chars.next().map(|c| c.to_ascii_uppercase());
                format!("[{}]{}", first.unwrap_or(' '), chars.as_str())
            })
            .collect();
        format!("{}: {}", self.question, choices.join(" "))
    }

    /// Draws the card centred in `area`: one line of text with a blank row and two
    /// spaces of padding around it.
    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let text = self.text();
        let width = u16::try_from(text.width() + 4)
            .unwrap_or(u16::MAX)
            .min(area.width);
        let height = 3.min(area.height);
        let card = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        };
        let lines = vec![Line::raw(""), Line::raw(format!("  {text}")), Line::raw("")];
        frame.render_widget(Clear, card);
        frame.render_widget(Paragraph::new(lines).style(Style::new().reversed()), card);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const CARD: Confirm = Confirm {
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

    #[test]
    fn text_brackets_each_first_letter() {
        assert_eq!(CARD.text(), "Unsaved changes: [S]ave [D]iscard [C]ancel");
    }

    #[test]
    fn letters_pick_in_either_case_and_esc_dismisses() {
        assert_eq!(CARD.answer(Input::Text('s')), Some(Answer::Picked('s')));
        assert_eq!(CARD.answer(Input::Text('D')), Some(Answer::Picked('d')));
        assert_eq!(CARD.answer(Input::Text('x')), None);
        assert_eq!(
            CARD.answer(Input::Action(Action::Cancel)),
            Some(Answer::Dismissed)
        );
        assert_eq!(CARD.answer(Input::Action(Action::Newline)), None);
    }

    #[test]
    fn card_is_centred() -> anyhow::Result<()> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| CARD.render(frame, frame.area()))?;
        let buffer = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..100).map(|x| buffer[(x, y)].symbol()).collect() };
        let text = CARD.text();
        let line = row(14);
        let start = line.find(&text).unwrap_or_else(|| panic!("{line:?}"));
        let end = 100 - (start + text.len());
        assert!(start.abs_diff(end) <= 1, "{line:?}");
        Ok(())
    }
}
