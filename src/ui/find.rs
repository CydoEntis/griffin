//! The find bar: the one-row prompt bar with case and regex toggles, the match
//! count and, for a bad regex, why it found nothing.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ropey::Rope;
use unicode_width::UnicodeWidthStr;

use crate::keymap::{Action, Input};
use crate::search::{self, Query};
use crate::theme::Theme;
use crate::ui::prompt::{Outcome, PromptBar};

/// What a key did to the find bar, for the editor to follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Nothing the editor needs to react to.
    Stay,
    /// The current match may have changed: move the cursor to it.
    Jump,
    /// Esc: close the bar, leaving the cursor where it is.
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindBar {
    bar: PromptBar,
    case_sensitive: bool,
    regex: bool,
    /// Every match of the bar's text in the buffer, in order.
    matches: Vec<Range<usize>>,
    /// Index into `matches` of the one the cursor is on.
    current: Option<usize>,
    /// Why the pattern found nothing, when it's an invalid regex.
    error: Option<String>,
    /// Where the cursor was when the bar opened: typing looks for the first match
    /// from here, so refining the text doesn't walk the cursor down the file.
    origin: usize,
}

impl FindBar {
    /// A bar holding `text`, already searched in `rope` from `origin`.
    pub fn new(text: &str, origin: usize, rope: &Rope) -> Self {
        let mut find = FindBar {
            bar: PromptBar::new("Find", text),
            case_sensitive: false,
            regex: false,
            matches: Vec::new(),
            current: None,
            error: None,
            origin,
        };
        find.search(rope);
        find
    }

    /// The match the cursor should be on, if any.
    pub fn current_match(&self) -> Option<Range<usize>> {
        self.matches.get(self.current?).cloned()
    }

    pub fn matches(&self) -> &[Range<usize>] {
        &self.matches
    }

    /// One key: edits re-run the search, the find keys move or toggle. The buffer
    /// can't change while the bar is open, since the bar takes every key.
    pub fn handle(&mut self, input: Input, rope: &Rope) -> Step {
        match input {
            Input::Action(Action::FindNext) => self.step(1),
            Input::Action(Action::FindPrev) => self.step(-1),
            Input::Action(Action::FindCase) => {
                self.case_sensitive = !self.case_sensitive;
                self.search(rope)
            }
            Input::Action(Action::FindRegex) => {
                self.regex = !self.regex;
                self.search(rope)
            }
            input => {
                let before = self.bar.text().to_string();
                match self.bar.handle(input) {
                    Some(Outcome::Cancel) => Step::Close,
                    // Enter only gets here when `find_next` was moved off it.
                    Some(Outcome::Submit) => self.step(1),
                    None if self.bar.text() != before => self.search(rope),
                    None => Step::Stay,
                }
            }
        }
    }

    /// Pasted text goes in as typed; a line break in it is dropped, since the bar
    /// holds one line.
    pub fn paste(&mut self, text: &str, rope: &Rope) -> Step {
        let line = text.lines().next().unwrap_or_default();
        let before = self.bar.text().to_string();
        let _ = self.bar.paste(line);
        if self.bar.text() == before {
            Step::Stay
        } else {
            self.search(rope)
        }
    }

    fn search(&mut self, rope: &Rope) -> Step {
        let query = Query {
            pattern: self.bar.text().to_string(),
            case_sensitive: self.case_sensitive,
            regex: self.regex,
        };
        (self.matches, self.error) = match search::find_all(rope, &query) {
            Ok(matches) => (matches, None),
            Err(err) => (Vec::new(), Some(err)),
        };
        self.current = search::next_from(&self.matches, self.origin);
        Step::Jump
    }

    /// Moves `delta` matches along, wrapping at either end.
    fn step(&mut self, delta: isize) -> Step {
        let len = self.matches.len();
        let Some(current) = self.current else {
            return Step::Stay;
        };
        let next = (current.cast_signed() + delta).rem_euclid(len.cast_signed());
        self.current = Some(next.cast_unsigned());
        // Retyping should now search on from here, not from where the bar opened.
        self.origin = self.matches[next.cast_unsigned()].start;
        Step::Jump
    }

    /// What the right end of the bar says about the matches: `n/m`, or the error.
    fn status(&self) -> String {
        if let Some(err) = &self.error {
            return err.clone();
        }
        if self.bar.text().is_empty() {
            return String::new();
        }
        match self.current {
            Some(i) => format!("{}/{}", i + 1, self.matches.len()),
            None => "0/0".to_string(),
        }
    }

    /// Draws `Find: text` with, at the right, the toggles (in the accent while on)
    /// and the match count, and returns where the cursor goes.
    pub fn render(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        let at = self.bar.render(theme, frame, area);
        let status = self.status();
        let base = Style::new().bg(theme.card2);
        let toggle = |on: bool| base.fg(if on { theme.accent } else { theme.muted });
        let status_style = if self.error.is_some() {
            base.fg(theme.err)
        } else {
            base.fg(theme.strong)
        };
        // Right-aligned: `Aa  .*  1/3 `.
        let parts = [
            ("Aa", toggle(self.case_sensitive)),
            ("  ", base),
            (".*", toggle(self.regex)),
            ("  ", base),
            (status.as_str(), status_style),
            (" ", base),
        ];
        let width: usize = parts.iter().map(|(text, _)| text.width()).sum();
        let mut x = area
            .right()
            .saturating_sub(u16::try_from(width).unwrap_or(0));
        for (text, style) in parts {
            let w = u16::try_from(text.width()).unwrap_or(0);
            if x >= area.x {
                frame.buffer_mut().set_string(x, area.y, text, style);
            }
            x += w;
        }
        at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn rope() -> Rope {
        Rope::from_str("foo bar\nFoo foo\n")
    }

    fn type_text(find: &mut FindBar, rope: &Rope, text: &str) {
        for c in text.chars() {
            find.handle(Input::Text(c), rope);
        }
    }

    #[test]
    fn typing_finds_the_first_match_from_where_the_bar_opened() {
        let rope = rope();
        let mut find = FindBar::new("", 5, &rope);
        assert_eq!(find.current_match(), None);
        type_text(&mut find, &rope, "foo");
        assert_eq!(find.matches(), [0..3, 8..11, 12..15]);
        assert_eq!(find.current_match(), Some(8..11));
        assert_eq!(find.status(), "2/3");
    }

    #[test]
    fn next_and_previous_wrap() {
        let rope = rope();
        let mut find = FindBar::new("foo", 0, &rope);
        assert_eq!(find.current_match(), Some(0..3));
        assert_eq!(
            find.handle(Input::Action(Action::FindPrev), &rope),
            Step::Jump
        );
        assert_eq!(find.current_match(), Some(12..15));
        find.handle(Input::Action(Action::FindNext), &rope);
        assert_eq!(find.current_match(), Some(0..3));
        find.handle(Input::Action(Action::FindNext), &rope);
        assert_eq!(find.status(), "2/3");
    }

    #[test]
    fn toggles_research_and_a_bad_regex_says_so_with_no_matches() {
        let rope = rope();
        let mut find = FindBar::new("foo", 0, &rope);
        find.handle(Input::Action(Action::FindCase), &rope);
        assert_eq!(find.matches(), [0..3, 12..15]);
        find.handle(Input::Action(Action::FindRegex), &rope);
        type_text(&mut find, &rope, "(");
        assert!(find.matches().is_empty());
        assert_eq!(find.current_match(), None);
        assert_eq!(find.status(), "invalid regex");
        find.handle(Input::Action(Action::FindRegex), &rope);
        assert_eq!(find.status(), "0/0");
        assert_eq!(
            find.handle(Input::Action(Action::Cancel), &rope),
            Step::Close
        );
    }

    #[test]
    fn renders_text_toggles_and_count() -> anyhow::Result<()> {
        let rope = rope();
        let find = FindBar::new("foo", 0, &rope);
        let mut terminal = Terminal::new(TestBackend::new(40, 1))?;
        let mut at = (0, 0);
        terminal.draw(|frame| at = find.render(&Theme::default(), frame, frame.area()))?;
        let buffer = terminal.backend().buffer();
        let row: String = (0..40).map(|x| buffer[(x, 0)].symbol()).collect();
        assert_eq!(row, format!("{:<28}{}", "Find: foo", "Aa  .*  1/3 "));
        assert_eq!(at, (9, 0));
        Ok(())
    }
}
