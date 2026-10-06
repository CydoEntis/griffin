//! The go-to-file picker: a centred card with a query line over the project's
//! files, filtered fuzzily as the query is typed, best match first.

use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Clear};
use unicode_width::UnicodeWidthStr;

use crate::buffer::movement::Motion;
use crate::keymap::{Action, Input};
use crate::ui::prompt::{Outcome, PromptBar};

/// Matched letters are drawn in this colour. Fixed until themes exist (R19).
pub const ACCENT: Color = Color::Cyan;

/// Widest and tallest the card gets, borders included.
const MAX_WIDTH: u16 = 80;
const MAX_HEIGHT: u16 = 20;

/// What a key did to the picker, when it did more than move or edit the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    /// Enter on a file: its path relative to the project root, `/`-separated.
    Open(String),
    /// Esc: close the picker and do nothing.
    Close,
}

/// One file the query matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Match {
    /// Index into `files`.
    file: usize,
    score: u32,
}

#[derive(Debug, Clone)]
pub struct Picker {
    query: PromptBar,
    /// Every file in the project; `None` until the background walk reports back.
    files: Option<Vec<String>>,
    /// The files the query matches, best first.
    matches: Vec<Match>,
    /// Index into `matches`.
    selected: usize,
    matcher: Matcher,
}

impl Default for Picker {
    fn default() -> Self {
        Self::new()
    }
}

impl Picker {
    /// An empty picker, waiting for the file list.
    pub fn new() -> Self {
        Picker {
            query: PromptBar::new("Go to file", ""),
            files: None,
            matches: Vec::new(),
            selected: 0,
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
        }
    }

    /// The walk's result: the files to pick from, relative to the root.
    pub fn set_files(&mut self, files: Vec<String>) {
        self.files = Some(files);
        self.refilter();
    }

    /// The matching paths, best first.
    pub fn matches(&self) -> impl Iterator<Item = &str> {
        let files = self.files.as_deref().unwrap_or_default();
        self.matches.iter().map(|m| files[m.file].as_str())
    }

    pub fn selected(&self) -> Option<&str> {
        self.matches().nth(self.selected)
    }

    /// Up and Down move through the list, Enter opens the selected file and Esc
    /// closes; everything else edits the query.
    pub fn handle(&mut self, input: Input) -> Option<Picked> {
        match input {
            Input::Action(Action::Move(Motion::Up)) => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            Input::Action(Action::Move(Motion::Down)) => {
                if self.selected + 1 < self.matches.len() {
                    self.selected += 1;
                }
                None
            }
            _ => {
                let before = self.query.text().to_string();
                let outcome = self.query.handle(input);
                self.after_edit(&before, outcome)
            }
        }
    }

    /// Pasted text goes into the query as if typed; a line break opens the
    /// selected file, as Enter would.
    pub fn paste(&mut self, text: &str) -> Option<Picked> {
        let before = self.query.text().to_string();
        let outcome = self.query.paste(text);
        self.after_edit(&before, outcome)
    }

    fn after_edit(&mut self, before: &str, outcome: Option<Outcome>) -> Option<Picked> {
        if self.query.text() != before {
            self.refilter();
        }
        match outcome? {
            Outcome::Submit => self.selected().map(|path| Picked::Open(path.to_string())),
            Outcome::Cancel => Some(Picked::Close),
        }
    }

    /// Scores every file against the query. An empty query lists every file in
    /// path order; otherwise the best score comes first, ties going to the shorter
    /// path, as it's the more likely one to have been meant.
    fn refilter(&mut self) {
        self.selected = 0;
        let Some(files) = &self.files else {
            return;
        };
        let pattern = Pattern::parse(self.query.text(), CaseMatching::Smart, Normalization::Smart);
        let mut chars = Vec::new();
        self.matches = files
            .iter()
            .enumerate()
            .filter_map(|(file, path)| {
                let score = pattern.score(Utf32Str::new(path, &mut chars), &mut self.matcher)?;
                Some(Match { file, score })
            })
            .collect();
        if !self.query.text().trim().is_empty() {
            self.matches.sort_by(|a, b| {
                b.score
                    .cmp(&a.score)
                    .then_with(|| files[a.file].len().cmp(&files[b.file].len()))
                    .then_with(|| files[a.file].cmp(&files[b.file]))
            });
        }
    }

    /// Where the card goes in `area`: centred, three fifths of the width and up to
    /// twenty rows tall.
    fn card(area: Rect) -> Rect {
        let width = (area.width * 3 / 5).clamp(40, MAX_WIDTH).min(area.width);
        let height = MAX_HEIGHT
            .min(area.height.saturating_sub(4))
            .max(3.min(area.height));
        Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        }
    }

    /// Draws the card centred in `area`: a border, the query line, then the
    /// matches with their matched letters in `ACCENT` and the selected one
    /// reversed. Returns where the cursor goes, in the query.
    pub fn render(&self, frame: &mut Frame, area: Rect) -> (u16, u16) {
        let card = Self::card(area);
        frame.render_widget(Clear, card);
        let block = Block::bordered();
        let inner = block.inner(card);
        frame.render_widget(block, card);
        if inner.height == 0 || inner.width == 0 {
            return (card.x, card.y);
        }
        let cursor = self.query.render(frame, Rect { height: 1, ..inner });

        let list = Rect {
            y: inner.y + 1,
            height: inner.height - 1,
            ..inner
        };
        let width = usize::from(list.width);
        let out = frame.buffer_mut();
        let Some(files) = &self.files else {
            out.set_stringn(list.x, list.y, " listing files…", width, Style::new().dim());
            return cursor;
        };
        if self.matches.is_empty() {
            out.set_stringn(
                list.x,
                list.y,
                " no matching files",
                width,
                Style::new().dim(),
            );
            return cursor;
        }

        let rows = usize::from(list.height);
        // Scrolls just far enough to keep the selection on the last row.
        let first = self.selected.saturating_sub(rows.saturating_sub(1));
        let pattern = Pattern::parse(self.query.text(), CaseMatching::Smart, Normalization::Smart);
        let mut matcher = self.matcher.clone();
        let mut chars = Vec::new();
        let mut indices = Vec::new();
        for (line, (index, m)) in self
            .matches
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .enumerate()
        {
            let path = files[m.file].as_str();
            indices.clear();
            pattern.indices(Utf32Str::new(path, &mut chars), &mut matcher, &mut indices);
            indices.sort_unstable();
            indices.dedup();

            let y = list.y + u16::try_from(line).unwrap_or(u16::MAX);
            let base = if index == self.selected {
                Style::new().reversed()
            } else {
                Style::new()
            };
            // Padded so the selected row is reversed edge to edge.
            out.set_stringn(list.x, y, format!("{:width$}", ""), width, base);
            let mut x = list.x + 1;
            for (at, ch) in path.chars().enumerate() {
                let cell = ch.to_string();
                let cells = u16::try_from(cell.width()).unwrap_or(1);
                if x + cells > list.right() {
                    break;
                }
                let hit = u32::try_from(at).is_ok_and(|at| indices.binary_search(&at).is_ok());
                let style = if hit { base.fg(ACCENT) } else { base };
                out.set_string(x, y, &cell, style);
                x += cells;
            }
        }
        cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn picker(files: &[&str]) -> Picker {
        let mut picker = Picker::new();
        picker.set_files(files.iter().map(|f| f.to_string()).collect());
        picker
    }

    fn type_query(picker: &mut Picker, text: &str) {
        for c in text.chars() {
            picker.handle(Input::Text(c));
        }
    }

    const FILES: &[&str] = &[
        "README.md",
        "docs/guide.md",
        "src/domain.rs",
        "src/main.rs",
        "src/util/helpers.rs",
    ];

    #[test]
    fn an_empty_query_lists_every_file_in_order() {
        let p = picker(FILES);
        assert_eq!(p.matches().collect::<Vec<_>>(), FILES);
        assert_eq!(p.selected(), Some("README.md"));
    }

    #[test]
    fn typing_filters_fuzzily_with_the_best_match_first() {
        let mut p = picker(FILES);
        type_query(&mut p, "main");
        let matches: Vec<_> = p.matches().collect();
        assert_eq!(matches.first(), Some(&"src/main.rs"));
        assert!(!matches.contains(&"README.md"));
        type_query(&mut p, "x");
        assert_eq!(p.matches().count(), 0);
        p.handle(Input::Action(Action::Backspace));
        assert_eq!(p.query.text(), "main");
        assert_eq!(p.matches().next(), Some("src/main.rs"));
    }

    #[test]
    fn arrows_move_within_the_list_and_enter_opens_the_selection() {
        let mut p = picker(FILES);
        p.handle(Input::Action(Action::Move(Motion::Up)));
        assert_eq!(p.selected(), Some("README.md"));
        for _ in 0..10 {
            p.handle(Input::Action(Action::Move(Motion::Down)));
        }
        assert_eq!(p.selected(), Some("src/util/helpers.rs"));
        p.handle(Input::Action(Action::Move(Motion::Up)));
        assert_eq!(
            p.handle(Input::Action(Action::Newline)),
            Some(Picked::Open("src/main.rs".into()))
        );
        assert_eq!(p.handle(Input::Action(Action::Cancel)), Some(Picked::Close));
    }

    #[test]
    fn enter_with_nothing_matched_or_listed_does_nothing() {
        let mut p = Picker::new();
        assert_eq!(p.handle(Input::Action(Action::Newline)), None);
        let mut p = picker(FILES);
        type_query(&mut p, "zzz");
        assert_eq!(p.handle(Input::Action(Action::Newline)), None);
    }

    #[test]
    fn a_pasted_line_break_opens_the_best_match() {
        let mut p = picker(FILES);
        assert_eq!(
            p.paste("helpers\n"),
            Some(Picked::Open("src/util/helpers.rs".into()))
        );
    }

    #[test]
    fn matched_letters_are_drawn_in_the_accent() -> anyhow::Result<()> {
        let mut p = picker(FILES);
        type_query(&mut p, "main");
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| {
            p.render(frame, frame.area());
        })?;
        let buffer = terminal.backend().buffer();
        let card = Picker::card(Rect::new(0, 0, 100, 30));
        let row = card.y + 2;
        let line: String = (card.x..card.right())
            .map(|x| buffer[(x, row)].symbol())
            .collect();
        assert!(line.contains("src/main.rs"), "{line:?}");
        let accented: String = (card.x..card.right())
            .filter(|&x| buffer[(x, row)].fg == ACCENT)
            .map(|x| buffer[(x, row)].symbol())
            .collect();
        assert_eq!(accented, "main");
        Ok(())
    }
}
