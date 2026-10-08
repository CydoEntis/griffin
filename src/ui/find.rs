//! The find bar: the one-row prompt bar with case and regex toggles, the match
//! count and, for a bad regex, why it found nothing. Opened with Ctrl+R it has a
//! Replace field beside the Find field, and Tab moves between them.

use std::ops::Range;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
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
    /// Enter in the Replace field: replace the current match, then go to the next.
    Replace,
    /// Replace every match as one undo step.
    ReplaceAll,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindBar {
    bar: PromptBar,
    /// The Replace field, once Ctrl+R has asked for it.
    replace: Option<PromptBar>,
    /// Keys go to the Replace field instead of the Find field.
    in_replace: bool,
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
            replace: None,
            in_replace: false,
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

    /// Adds the Replace field if it isn't there yet; `focus` moves typing into it.
    pub fn show_replace(&mut self, focus: bool) {
        self.replace
            .get_or_insert_with(|| PromptBar::new("Replace", ""));
        if focus {
            self.in_replace = true;
        }
    }

    /// Every match with the text that replaces it, `$1` groups expanded in regex
    /// mode. Empty while the bar has no Replace field.
    pub fn replacements(&self, rope: &Rope) -> Vec<(Range<usize>, String)> {
        let Some(replace) = &self.replace else {
            return Vec::new();
        };
        search::replacements(rope, &self.query(), replace.text()).unwrap_or_default()
    }

    /// The current match with the text that replaces it.
    pub fn current_replacement(&self, rope: &Rope) -> Option<(Range<usize>, String)> {
        let current = self.current_match()?;
        self.replacements(rope)
            .into_iter()
            .find(|(range, _)| *range == current)
    }

    /// Searches again after the buffer changed, taking the first match from
    /// `origin` as the current one.
    pub fn research_from(&mut self, rope: &Rope, origin: usize) {
        self.origin = origin;
        self.search(rope);
    }

    fn query(&self) -> Query {
        Query {
            pattern: self.bar.text().to_string(),
            case_sensitive: self.case_sensitive,
            regex: self.regex,
        }
    }

    /// One key: edits re-run the search, the find keys move or toggle. The buffer
    /// only changes while the bar is open through the replace steps it returns.
    pub fn handle(&mut self, input: Input, rope: &Rope) -> Step {
        match input {
            Input::Action(Action::FindNext) if self.in_replace => Step::Replace,
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
            Input::Action(Action::ReplaceAll) if self.replace.is_some() => Step::ReplaceAll,
            Input::Action(Action::Replace) => {
                self.show_replace(true);
                Step::Stay
            }
            Input::Action(Action::Tab) => {
                self.in_replace = self.replace.is_some() && !self.in_replace;
                Step::Stay
            }
            input if self.in_replace => match self.replace.as_mut().map(|bar| bar.handle(input)) {
                Some(Some(Outcome::Cancel)) => Step::Close,
                // Enter only gets here when `find_next` was moved off it.
                Some(Some(Outcome::Submit)) => Step::Replace,
                _ => Step::Stay,
            },
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
    /// holds one line. A tab moves between the fields as the key does: on Windows
    /// Tab followed quickly by typing arrives as one paste.
    pub fn paste(&mut self, text: &str, rope: &Rope) -> Step {
        let line = text.lines().next().unwrap_or_default();
        let mut step = Step::Stay;
        for (i, part) in line.split('\t').enumerate() {
            if i > 0 {
                self.in_replace = self.replace.is_some() && !self.in_replace;
            }
            if self.paste_into_field(part, rope) == Step::Jump {
                step = Step::Jump;
            }
        }
        step
    }

    fn paste_into_field(&mut self, text: &str, rope: &Rope) -> Step {
        if self.in_replace {
            if let Some(replace) = &mut self.replace {
                let _ = replace.paste(text);
            }
            return Step::Stay;
        }
        let before = self.bar.text().to_string();
        let _ = self.bar.paste(text);
        if self.bar.text() == before {
            Step::Stay
        } else {
            self.search(rope)
        }
    }

    fn search(&mut self, rope: &Rope) -> Step {
        (self.matches, self.error) = match search::find_all(rope, &self.query()) {
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

    /// Draws ` Find  text` (and, when it's there, a `│` rule at the middle and
    /// ` Replace  text` after it) on `raised` with, at the right, the ` Aa ` and
    /// ` .* ` chips (filled with the accent while on) and the match count, and
    /// returns where the cursor goes.
    pub fn render(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        let status = self.status();
        let base = Style::new().bg(theme.raised);
        let chip = |on: bool| {
            if on {
                base.bg(theme.accent)
                    .fg(theme.acc_ink)
                    .add_modifier(Modifier::BOLD)
            } else {
                base.fg(theme.muted)
            }
        };
        // A bad pattern is drawn in `err` along with the count saying why.
        let (pattern, status_style) = if self.error.is_some() {
            (Style::new().fg(theme.err), base.fg(theme.err))
        } else {
            (Style::new(), base.fg(theme.strong))
        };
        // Right-aligned: ` Aa   .*   1/3 `.
        let parts = [
            (" Aa ", chip(self.case_sensitive)),
            (" ", base),
            (" .* ", chip(self.regex)),
            ("  ", base),
            (status.as_str(), status_style),
            (" ", base),
        ];
        let width: usize = parts.iter().map(|(text, _)| text.width()).sum();
        let width = u16::try_from(width).unwrap_or(u16::MAX);
        let buf = frame.buffer_mut();
        buf.set_style(area, base);
        let at = match &self.replace {
            None => {
                let find_area = Rect {
                    width: area.width.saturating_sub(width),
                    ..area
                };
                self.bar.render_field(theme, buf, find_area, true, pattern)
            }
            Some(replace) => {
                // Halves of the whole row, not of what the count leaves, so the
                // Replace field stays put while the count changes width; the
                // Replace field shrinks first as the row narrows.
                let half = area.width / 2;
                let find_area = Rect {
                    width: half,
                    ..area
                };
                let replace_area = Rect {
                    x: area.x + half + 1,
                    width: area.width.saturating_sub(half + 1).saturating_sub(width),
                    ..area
                };
                let find_at =
                    self.bar
                        .render_field(theme, buf, find_area, !self.in_replace, pattern);
                if half < area.width {
                    buf.set_string(area.x + half, area.y, "│", base.fg(theme.guide));
                }
                let replace_at =
                    replace.render_field(theme, buf, replace_area, self.in_replace, Style::new());
                if self.in_replace { replace_at } else { find_at }
            }
        };
        let mut x = area.right().saturating_sub(width);
        for (text, style) in parts {
            let w = u16::try_from(text.width()).unwrap_or(0);
            if x >= area.x {
                buf.set_string(x, area.y, text, style);
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
    fn tab_moves_between_fields_and_enter_in_replace_asks_to_replace() {
        let rope = rope();
        let mut find = FindBar::new("foo", 0, &rope);
        let act = Input::Action;
        // Without the Replace field, Tab and Alt+A do nothing.
        assert_eq!(find.handle(act(Action::Tab), &rope), Step::Stay);
        assert_eq!(find.handle(act(Action::ReplaceAll), &rope), Step::Stay);
        assert!(find.replacements(&rope).is_empty());

        find.show_replace(false);
        find.handle(act(Action::Tab), &rope);
        type_text(&mut find, &rope, "x");
        // Typing in Replace doesn't touch the pattern.
        assert_eq!(find.matches(), [0..3, 8..11, 12..15]);
        assert_eq!(
            find.current_replacement(&rope),
            Some((0..3, "x".to_string()))
        );
        assert_eq!(find.handle(act(Action::FindNext), &rope), Step::Replace);
        assert_eq!(
            find.handle(act(Action::ReplaceAll), &rope),
            Step::ReplaceAll
        );
        // Shift+Enter still steps back through matches from the Replace field.
        assert_eq!(find.handle(act(Action::FindPrev), &rope), Step::Jump);
        assert_eq!(find.current_match(), Some(12..15));
        find.handle(act(Action::Tab), &rope);
        type_text(&mut find, &rope, "x");
        assert!(find.matches().is_empty());
        assert_eq!(find.handle(act(Action::Cancel), &rope), Step::Close);
    }

    #[test]
    fn a_pasted_tab_moves_to_the_other_field() {
        let rope = rope();
        let mut find = FindBar::new("", 0, &rope);
        find.show_replace(false);
        assert_eq!(find.paste("foo\tbar", &rope), Step::Jump);
        assert_eq!(find.matches(), [0..3, 8..11, 12..15]);
        assert_eq!(
            find.current_replacement(&rope),
            Some((0..3, "bar".to_string()))
        );
        // Without the Replace field a tab is just dropped.
        let mut find = FindBar::new("", 0, &rope);
        find.paste("fo\to", &rope);
        assert_eq!(find.matches(), [0..3, 8..11, 12..15]);
    }

    #[test]
    fn research_from_picks_the_next_match_after_an_edit() {
        let mut find = FindBar::new("foo", 0, &rope());
        find.show_replace(true);
        let edited = Rope::from_str("x bar\nFoo foo\n");
        find.research_from(&edited, 1);
        assert_eq!(find.matches(), [6..9, 10..13]);
        assert_eq!(find.current_match(), Some(6..9));
    }

    #[test]
    fn renders_find_and_replace_fields_side_by_side() -> anyhow::Result<()> {
        let theme = Theme::default();
        let rope = rope();
        let mut find = FindBar::new("foo", 0, &rope);
        find.show_replace(true);
        find.paste("bar", &rope);
        let mut terminal = Terminal::new(TestBackend::new(60, 1))?;
        let mut at = (0, 0);
        terminal.draw(|frame| at = find.render(&theme, frame, frame.area()))?;
        let buffer = terminal.backend().buffer();
        let row: String = (0..60).map(|x| buffer[(x, 0)].symbol()).collect();
        // Find takes half the row, the rule sits at the middle, and Replace runs
        // up to the chips.
        assert_eq!(
            row,
            format!(
                "{:<30}│{:<14}{}",
                " Find  foo", " Replace  bar", " Aa   .*   1/3 "
            )
        );
        assert_eq!(at, (44, 0));
        assert_eq!(buffer[(30, 0)].fg, theme.guide);
        // The focused field's label is `text`, the other one's `muted`.
        assert_eq!(buffer[(1, 0)].fg, theme.muted);
        assert_eq!(buffer[(32, 0)].fg, theme.text);
        for x in 0..60 {
            assert_eq!(buffer[(x, 0)].bg, theme.raised, "{x}");
        }
        Ok(())
    }

    #[test]
    fn renders_text_chips_and_count() -> anyhow::Result<()> {
        let theme = Theme::default();
        let rope = rope();
        let mut find = FindBar::new("foo", 0, &rope);
        find.handle(Input::Action(Action::FindCase), &rope);
        let mut terminal = Terminal::new(TestBackend::new(40, 1))?;
        let mut at = (0, 0);
        terminal.draw(|frame| at = find.render(&theme, frame, frame.area()))?;
        let buffer = terminal.backend().buffer();
        let row: String = (0..40).map(|x| buffer[(x, 0)].symbol()).collect();
        assert_eq!(row, format!("{:<25}{}", " Find  foo", " Aa   .*   1/2 "));
        assert_eq!(at, (10, 0));
        // An `on` chip is `acc_ink` on `accent`, bold; an `off` one `muted`.
        let on = &buffer[(25, 0)];
        assert_eq!((on.fg, on.bg), (theme.acc_ink, theme.accent));
        assert!(on.modifier.contains(Modifier::BOLD));
        let off = &buffer[(30, 0)];
        assert_eq!((off.fg, off.bg), (theme.muted, theme.raised));
        assert_eq!(buffer[(36, 0)].fg, theme.strong);
        assert_eq!(buffer[(7, 0)].fg, theme.strong);
        Ok(())
    }

    #[test]
    fn a_bad_regex_draws_the_pattern_and_the_count_in_err() -> anyhow::Result<()> {
        let theme = Theme::default();
        let rope = rope();
        let mut find = FindBar::new("(", 0, &rope);
        find.handle(Input::Action(Action::FindRegex), &rope);
        let mut terminal = Terminal::new(TestBackend::new(40, 1))?;
        terminal.draw(|frame| {
            find.render(&theme, frame, frame.area());
        })?;
        let buffer = terminal.backend().buffer();
        let row: String = (0..40).map(|x| buffer[(x, 0)].symbol()).collect();
        assert!(row.ends_with("  invalid regex "), "{row:?}");
        assert_eq!(buffer[(7, 0)].fg, theme.err);
        assert_eq!(buffer[(26, 0)].fg, theme.err);
        Ok(())
    }
}
