//! A confirm card over the dimmed screen (SPEC_V1_LAYOUT §7.4): a question, a
//! line saying what answering means, and a row of buttons, e.g.
//! `notes.txt has unsaved changes` / ` Save   Discard   Cancel `. The focused
//! button (the first, until ← → or Tab move it) is the one Enter presses.

use std::borrow::Cow;

use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::buffer::movement::Motion;
use crate::keymap::{Action, Input};
use crate::theme::{Theme, grad};

/// One button: pressing `key` (either case) picks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub key: char,
    pub label: &'static str,
}

/// What the card was answered with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Picked(char),
    /// Esc or a click outside the card: close it and do nothing.
    Dismissed,
}

/// What a key press or click means while the card is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    Answer(Answer),
    /// Move the focus to the button at this index.
    Focus(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Confirm {
    /// Built at runtime when it names a file or a count.
    pub question: Cow<'static, str>,
    pub explanation: Cow<'static, str>,
    pub choices: &'static [Choice],
}

/// The card's height (SPEC_V1_LAYOUT §7.4): the lit edge, question,
/// explanation, a gap, the buttons, and two blank rows.
const HEIGHT: u16 = 7;
/// The narrowest card.
const MIN_WIDTH: u16 = 44;
/// Where text starts inside the card.
const INSET: u16 = 3;
/// The row of buttons, from the card's top.
const BUTTON_ROW: u16 = 4;
/// Blank cells between two buttons.
const BUTTON_GAP: u16 = 2;
const ESC: &str = "esc";

impl Confirm {
    /// What `input` does with `focused` the focused button. Letters arrive as
    /// typed text and the rest as actions, so the card never looks at key codes.
    pub fn handle(&self, input: Input, focused: usize) -> Option<Reply> {
        let last = self.choices.len().saturating_sub(1);
        match input {
            Input::Text(c) => {
                let c = c.to_ascii_lowercase();
                self.choices
                    .iter()
                    .find(|choice| choice.key.to_ascii_lowercase() == c)
                    .map(|choice| Reply::Answer(Answer::Picked(choice.key)))
            }
            Input::Action(Action::Cancel) => Some(Reply::Answer(Answer::Dismissed)),
            Input::Action(Action::Newline) => self
                .choices
                .get(focused)
                .map(|choice| Reply::Answer(Answer::Picked(choice.key))),
            Input::Action(Action::Move(Motion::Left)) => {
                Some(Reply::Focus(focused.saturating_sub(1)))
            }
            Input::Action(Action::Move(Motion::Right)) => {
                Some(Reply::Focus((focused + 1).min(last)))
            }
            // There's no Shift+Tab binding to go back, so Tab wraps instead of
            // getting stuck on the last button.
            Input::Action(Action::Tab) => {
                Some(Reply::Focus(if focused >= last { 0 } else { focused + 1 }))
            }
            _ => None,
        }
    }

    /// What a left click at `at` does when the card is drawn in `area`: a button
    /// presses it, anywhere outside the card is Esc (SPEC_V1_LAYOUT §7).
    pub fn click(&self, area: Rect, at: Position) -> Option<Reply> {
        let card = self.card(area);
        if !card.contains(at) {
            return Some(Reply::Answer(Answer::Dismissed));
        }
        self.buttons(card)
            .into_iter()
            .zip(self.choices)
            .find(|(button, _)| button.contains(at))
            .map(|(_, choice)| Reply::Answer(Answer::Picked(choice.key)))
    }

    /// Where the card goes in `area`: centred, `max(44, longest line + 8)` wide
    /// and 7 tall, kept a cell inside the screen on every side.
    pub fn card(&self, area: Rect) -> Rect {
        let longest = self.question.width().max(self.explanation.width());
        let width = u16::try_from(longest + 8)
            .unwrap_or(u16::MAX)
            .max(MIN_WIDTH)
            .min(area.width.saturating_sub(2));
        let height = HEIGHT.min(area.height.saturating_sub(2));
        Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        }
    }

    /// Each choice's button in `card`, ` Label ` with two cells between, cut at
    /// the card's inset.
    fn buttons(&self, card: Rect) -> Vec<Rect> {
        let y = card.y + BUTTON_ROW;
        if y >= card.bottom() {
            return Vec::new();
        }
        let right = card.right().saturating_sub(INSET);
        let mut x = card.x + INSET;
        let mut buttons = Vec::with_capacity(self.choices.len());
        for choice in self.choices {
            let width = u16::try_from(choice.label.width() + 2).unwrap_or(u16::MAX);
            let width = width.min(right.saturating_sub(x));
            buttons.push(Rect::new(x, y, width, 1));
            x = x.saturating_add(width + BUTTON_GAP).min(right);
        }
        buttons
    }

    /// Dims `area` and draws the card centred in it (SPEC_V1_LAYOUT §7.4 with
    /// README §5.4 buttons): the question in `strong` bold, the explanation in
    /// `muted`, the buttons with `focused` lit, and `esc` at the right.
    pub fn render(&self, theme: &Theme, frame: &mut Frame, area: Rect, focused: usize) {
        crate::ui::dim(theme, frame.buffer_mut(), area);
        let card = self.card(area);
        crate::ui::dialog_card(theme, frame, card);
        let card = card.intersection(frame.area());
        if card.height < HEIGHT || card.width <= INSET * 2 {
            return;
        }
        let room = usize::from(card.width - INSET * 2);
        let x = card.x + INSET;
        let buf = frame.buffer_mut();
        buf.set_stringn(
            x,
            card.y + 1,
            &self.question,
            room,
            Style::new().fg(theme.strong).add_modifier(Modifier::BOLD),
        );
        buf.set_stringn(
            x,
            card.y + 2,
            &self.explanation,
            room,
            Style::new().fg(theme.muted),
        );

        for (i, (button, choice)) in self.buttons(card).into_iter().zip(self.choices).enumerate() {
            let on = i == focused;
            let style = if on {
                Style::new().fg(theme.acc_ink).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.strong).bg(theme.raised2)
            };
            buf.set_stringn(
                button.x,
                button.y,
                format!(" {} ", choice.label),
                usize::from(button.width),
                style,
            );
            if on {
                // README §5.4: the default button is the aurora ramp, cell by cell.
                let span = f64::from(button.width.saturating_sub(1).max(1));
                for cx in button.left()..button.right() {
                    let t = f64::from(cx - button.x) / span;
                    buf[(cx, button.y)].set_bg(grad(&[theme.accent, theme.accent2], t));
                }
            }
            // The key letter is underlined where the label has it; `Move to
            // trash` (y) doesn't, and then nothing is. The underline keeps the
            // letter's colour: a separate underline colour (SGR 58) is misread
            // as other attributes by terminals that don't know it.
            let key = choice.key.to_ascii_lowercase();
            let mut cx = button.x + 1;
            for c in choice.label.chars() {
                if c.to_ascii_lowercase() == key {
                    if cx < button.right() {
                        buf[(cx, button.y)]
                            .set_style(Style::new().add_modifier(Modifier::UNDERLINED));
                    }
                    break;
                }
                cx += u16::try_from(c.width().unwrap_or(0)).unwrap_or(0);
            }
        }

        let esc_x = card.right().saturating_sub(INSET + ESC.len() as u16);
        buf.set_string(
            esc_x,
            card.y + BUTTON_ROW,
            ESC,
            Style::new().fg(theme.muted),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    const CARD: Confirm = Confirm {
        question: Cow::Borrowed("a.txt has unsaved changes"),
        explanation: Cow::Borrowed("Closing discards them unless you save."),
        choices: &[
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
        ],
    };

    fn draw(focused: usize) -> anyhow::Result<ratatui::buffer::Buffer> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| CARD.render(&Theme::default(), frame, frame.area(), focused))?;
        Ok(terminal.backend().buffer().clone())
    }

    fn row(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        (0..100).map(|x| buffer[(x, y)].symbol()).collect()
    }

    #[test]
    fn letters_pick_in_either_case_and_esc_dismisses() {
        let picked = |c| Some(Reply::Answer(Answer::Picked(c)));
        assert_eq!(CARD.handle(Input::Text('s'), 0), picked('s'));
        assert_eq!(CARD.handle(Input::Text('D'), 0), picked('d'));
        assert_eq!(CARD.handle(Input::Text('x'), 0), None);
        assert_eq!(
            CARD.handle(Input::Action(Action::Cancel), 1),
            Some(Reply::Answer(Answer::Dismissed))
        );
    }

    #[test]
    fn arrows_and_tab_move_the_focus_and_enter_presses_it() {
        let left = Input::Action(Action::Move(Motion::Left));
        let right = Input::Action(Action::Move(Motion::Right));
        let tab = Input::Action(Action::Tab);
        assert_eq!(CARD.handle(right, 0), Some(Reply::Focus(1)));
        assert_eq!(CARD.handle(right, 2), Some(Reply::Focus(2)));
        assert_eq!(CARD.handle(left, 0), Some(Reply::Focus(0)));
        assert_eq!(CARD.handle(left, 2), Some(Reply::Focus(1)));
        assert_eq!(CARD.handle(tab, 1), Some(Reply::Focus(2)));
        assert_eq!(CARD.handle(tab, 2), Some(Reply::Focus(0)));
        assert_eq!(
            CARD.handle(Input::Action(Action::Newline), 1),
            Some(Reply::Answer(Answer::Picked('d')))
        );
    }

    #[test]
    fn the_card_is_at_least_44_wide_and_grows_with_its_longest_line() {
        let area = Rect::new(0, 0, 100, 30);
        // The explanation, 38 cells, is the longest: 38 + 8 = 46.
        assert_eq!(CARD.card(area), Rect::new(27, 11, 46, 7));
        let short = Confirm {
            question: Cow::Borrowed("Short?"),
            explanation: Cow::Borrowed(""),
            ..CARD
        };
        assert_eq!(short.card(area), Rect::new(28, 11, 44, 7));
        // Clamped a cell inside a small screen.
        assert_eq!(CARD.card(Rect::new(0, 0, 40, 6)).width, 38);
    }

    #[test]
    fn copy_and_buttons_sit_at_the_spec_offsets() -> anyhow::Result<()> {
        let buffer = draw(0)?;
        let (x, y) = (27, 11);
        assert_eq!(buffer[(x, y)].symbol(), "▀");
        assert!(row(&buffer, y + 1)[usize::from(x + 3)..].starts_with("a.txt has unsaved changes"));
        assert!(buffer[(x + 3, y + 1)].modifier.contains(Modifier::BOLD));
        let theme = Theme::default();
        assert_eq!(buffer[(x + 3, y + 2)].fg, theme.muted);
        let buttons = row(&buffer, y + 4);
        assert!(
            buttons[usize::from(x + 3)..].starts_with(" Save    Discard    Cancel "),
            "{buttons:?}"
        );
        assert_eq!(
            row(&buffer, y + 4).find("esc"),
            Some(usize::from(x + 46 - 6))
        );
        Ok(())
    }

    #[test]
    fn the_focused_button_is_lit_and_the_key_letter_underlined() -> anyhow::Result<()> {
        let theme = Theme::default();
        let buffer = draw(0)?;
        let (x, y) = (30, 15);
        // ` Save ` runs accent → accent2 in acc_ink bold.
        assert_eq!(buffer[(x, y)].bg, theme.accent);
        assert_eq!(buffer[(x + 5, y)].bg, theme.accent2);
        assert_eq!(
            buffer[(x + 2, y)].bg,
            grad(&[theme.accent, theme.accent2], 0.4)
        );
        assert_eq!(buffer[(x + 1, y)].fg, theme.acc_ink);
        assert!(buffer[(x + 1, y)].modifier.contains(Modifier::BOLD));
        assert!(buffer[(x + 1, y)].modifier.contains(Modifier::UNDERLINED));
        // ` Discard ` is plain: strong on raised2, its `D` underlined.
        let discard = x + 8;
        assert_eq!(buffer[(discard, y)].bg, theme.raised2);
        assert_eq!(buffer[(discard + 1, y)].fg, theme.strong);
        assert!(
            buffer[(discard + 1, y)]
                .modifier
                .contains(Modifier::UNDERLINED)
        );
        assert!(
            !buffer[(discard + 2, y)]
                .modifier
                .contains(Modifier::UNDERLINED)
        );
        // Moving the focus moves the light.
        let buffer = draw(1)?;
        assert_eq!(buffer[(x, y)].bg, theme.raised2);
        assert_eq!(buffer[(discard, y)].bg, theme.accent);
        Ok(())
    }

    #[test]
    fn clicks_press_buttons_and_outside_dismisses() {
        let area = Rect::new(0, 0, 100, 30);
        let picked = |c| Some(Reply::Answer(Answer::Picked(c)));
        assert_eq!(CARD.click(area, Position::new(31, 15)), picked('s'));
        assert_eq!(CARD.click(area, Position::new(40, 15)), picked('d'));
        assert_eq!(CARD.click(area, Position::new(49, 15)), picked('c'));
        assert_eq!(CARD.click(area, Position::new(37, 15)), None);
        assert_eq!(CARD.click(area, Position::new(31, 13)), None);
        assert_eq!(
            CARD.click(area, Position::new(0, 0)),
            Some(Reply::Answer(Answer::Dismissed))
        );
    }
}
