//! The picker: a centred card with a query line over a list, filtered fuzzily as
//! the query is typed, best match first. Go to file lists the project's files; F5
//! lists the `[[run]]` entries by name.

use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;
use unicode_width::UnicodeWidthStr;

use crate::buffer::movement::Motion;
use crate::keymap::{Action, Input};
use crate::theme::Theme;
use crate::ui::prompt::{Outcome, PromptBar};

/// Widest and tallest the card gets, borders included.
const MAX_WIDTH: u16 = 80;
const MAX_HEIGHT: u16 = 20;

/// What a key did to the picker, when it did more than move or edit the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    /// Enter on an item: for go to file, its path relative to the project root,
    /// `/`-separated; for a list of choices, the choice as given.
    Open(String),
    /// Esc: close the picker and do nothing.
    Close,
}

/// One entry of a list of choices, such as a run command: picked by its name,
/// listed with a detail and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub name: String,
    pub detail: String,
    pub source: &'static str,
}

/// Tallest the card for a list of choices gets (SPEC_V1_LAYOUT §9).
const CHOICES_HEIGHT: u16 = 8;
/// Where a choice's detail starts, from the card's left edge.
const DETAIL_X: u16 = 14;

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
    /// Every file in the project, or the choices; `None` until the background
    /// walk reports back.
    files: Option<Vec<String>>,
    /// Shown when nothing matches the query.
    no_match: &'static str,
    /// For a list of choices, each one's detail and source, in `files` order;
    /// `None` for go to file.
    choices: Option<Vec<Choice>>,
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
            no_match: " no matching files",
            choices: None,
            matches: Vec::new(),
            selected: 0,
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
        }
    }

    /// A picker titled `title` over `choices`, listed in the order given and
    /// matched by name. It's drawn as a dialog over the dimmed screen.
    pub fn choices(title: &'static str, choices: Vec<Choice>) -> Self {
        let names = choices.iter().map(|c| c.name.clone()).collect();
        let mut picker = Picker {
            query: PromptBar::new(title, ""),
            files: None,
            no_match: "no match",
            choices: Some(choices),
            matches: Vec::new(),
            selected: 0,
            matcher: Matcher::new(Config::DEFAULT),
        };
        picker.set_files(names);
        picker
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

    /// Where the card for a list of choices goes in `area`: as wide as go to
    /// file's, up to eight rows tall (SPEC_V1_LAYOUT §9), centred.
    fn choices_card(area: Rect) -> Rect {
        let width = (area.width * 3 / 5).clamp(40, MAX_WIDTH).min(area.width);
        let height = CHOICES_HEIGHT.min(area.height.saturating_sub(4));
        Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        }
    }

    /// The positions of `text`'s letters the query matches, sorted.
    fn matched(&self, text: &str) -> Vec<u32> {
        let pattern = Pattern::parse(self.query.text(), CaseMatching::Smart, Normalization::Smart);
        let mut matcher = self.matcher.clone();
        let mut chars = Vec::new();
        let mut indices = Vec::new();
        pattern.indices(Utf32Str::new(text, &mut chars), &mut matcher, &mut indices);
        indices.sort_unstable();
        indices.dedup();
        indices
    }

    /// Dims `area` and draws a list of choices as a dialog over it (README §5.1,
    /// SPEC_V1_LAYOUT §9): the lit top edge, the query on an input row, then one
    /// row per match (its name with the matched letters in the accent, the
    /// detail in `muted` and the source right-aligned), the selected one as the
    /// glow row, and the key hints with the count. Returns where the cursor goes.
    fn render_choices(
        &self,
        theme: &Theme,
        frame: &mut Frame,
        area: Rect,
        choices: &[Choice],
    ) -> (u16, u16) {
        crate::ui::dim(theme, frame.buffer_mut(), area);
        let card = Self::choices_card(area);
        crate::ui::dialog_card(theme, frame, card);
        // The edge, the query, one choice and the footer.
        if card.height < 4 || card.width < 20 {
            return (card.x, card.y);
        }
        let label_width = u16::try_from(self.query.label.width()).unwrap_or(0);
        let cursor = crate::ui::input_row(
            theme,
            frame,
            Rect {
                y: card.y + 1,
                height: 1,
                ..card
            },
            self.query.label,
            label_width,
            &self.query,
            true,
        );
        let out = frame.buffer_mut();
        let muted = Style::new().fg(theme.muted);
        let total = self.files.as_ref().map_or(0, Vec::len);
        crate::ui::footer(theme, out, card, "↑↓ select   ⏎ run   esc close");
        let count = format!("{} of {total}", self.matches.len());
        let count_x = card
            .right()
            .saturating_sub(2 + u16::try_from(count.width()).unwrap_or(0));
        out.set_string(count_x, card.bottom() - 1, count, muted);

        let list = Rect {
            y: card.y + 2,
            height: card.height - 3,
            ..card
        };
        if self.matches.is_empty() {
            out.set_stringn(
                list.x + 2,
                list.y,
                self.no_match,
                usize::from(list.width.saturating_sub(4)),
                muted,
            );
            return cursor;
        }
        let rows = usize::from(list.height);
        // Scrolls just far enough to keep the selection on the last row.
        let first = self.selected.saturating_sub(rows.saturating_sub(1));
        let ramps = theme.ramps();
        for (line, (index, m)) in self
            .matches
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .enumerate()
        {
            let Some(choice) = choices.get(m.file) else {
                continue;
            };
            let y = list.y + u16::try_from(line).unwrap_or(u16::MAX);
            let selected = index == self.selected;
            if selected {
                crate::ui::glow_row(theme, out, card.x, card.right(), y);
                crate::ui::enter_mark(theme, out, card.right() - 4, y);
            }
            // In mono the selected row is reverse video, which colours of its
            // own would break up.
            let plain = selected && !ramps;
            let style = |style: Style| if plain { Style::new() } else { style };
            let name_style = style(Style::new().fg(if selected { theme.strong } else { theme.fg }));
            let hit_style = style(Style::new().fg(theme.accent)).add_modifier(Modifier::BOLD);
            let dim_style = style(Style::new().fg(if selected { theme.text } else { theme.muted }));

            // The source ends two cells short of where `⏎` goes.
            let source_end = card.right().saturating_sub(7);
            let source_width = u16::try_from(choice.source.width()).unwrap_or(0);
            let source_x = source_end.saturating_sub(source_width);
            out.set_string(source_x, y, choice.source, dim_style);
            let text_end = source_x.saturating_sub(2);

            let indices = self.matched(&choice.name);
            let mut x = card.x + 2;
            for (at, ch) in choice.name.chars().enumerate() {
                let cell = ch.to_string();
                let cells = u16::try_from(cell.width()).unwrap_or(1);
                if x + cells > text_end {
                    break;
                }
                let hit = u32::try_from(at).is_ok_and(|at| indices.binary_search(&at).is_ok());
                out.set_string(x, y, &cell, if hit { hit_style } else { name_style });
                x += cells;
            }
            // A detected command is named after itself; saying it twice tells
            // nothing.
            if choice.detail != choice.name {
                let detail_x = (card.x + DETAIL_X).max(x + 2);
                let room = usize::from(text_end.saturating_sub(detail_x));
                out.set_stringn(detail_x, y, &choice.detail, room, dim_style);
            }
        }
        cursor
    }

    /// Draws the card centred in `area` on `card`: a border, the query line, then
    /// the matches with their matched letters in `accent` and the selected one
    /// filled with `hov`. Returns where the cursor goes, in the query.
    pub fn render(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        if let Some(choices) = &self.choices {
            return self.render_choices(theme, frame, area, choices);
        }
        let card = Self::card(area);
        frame.render_widget(Clear, card);
        let block = crate::ui::card_block(theme);
        let inner = block.inner(card);
        frame.render_widget(block, card);
        if inner.height == 0 || inner.width == 0 {
            return (card.x, card.y);
        }
        let cursor = self.query.render(theme, frame, Rect { height: 1, ..inner });

        let list = Rect {
            y: inner.y + 1,
            height: inner.height - 1,
            ..inner
        };
        let width = usize::from(list.width);
        let out = frame.buffer_mut();
        let Some(files) = &self.files else {
            out.set_stringn(
                list.x,
                list.y,
                " listing files…",
                width,
                Style::new().fg(theme.muted),
            );
            return cursor;
        };
        if self.matches.is_empty() {
            out.set_stringn(
                list.x,
                list.y,
                self.no_match,
                width,
                Style::new().fg(theme.muted),
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
            let selected = index == self.selected;
            let base = if selected {
                Theme::highlight(theme.hov, theme.strong)
            } else {
                Style::new()
            };
            // On the selected row a matched letter is a block of accent with
            // `acc_ink` on it, since accent text wouldn't read on `hov`.
            let hit_style = if selected {
                Theme::highlight(theme.accent, theme.acc_ink)
            } else {
                Style::new().fg(theme.accent)
            };
            // Padded so the selected row is filled edge to edge.
            out.set_stringn(list.x, y, format!("{:width$}", ""), width, base);
            let mut x = list.x + 1;
            for (at, ch) in path.chars().enumerate() {
                let cell = ch.to_string();
                let cells = u16::try_from(cell.width()).unwrap_or(1);
                if x + cells > list.right() {
                    break;
                }
                let hit = u32::try_from(at).is_ok_and(|at| indices.binary_search(&at).is_ok());
                let style = if hit { hit_style } else { base };
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

    use crate::theme::mix;

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

    fn run_choices() -> Vec<Choice> {
        [("dev", "npm run dev"), ("test", "cargo test")]
            .map(|(name, detail)| Choice {
                name: name.into(),
                detail: detail.into(),
                source: ".glyph.toml",
            })
            .to_vec()
    }

    #[test]
    fn choices_are_a_dialog_with_details_sources_and_the_glow() -> anyhow::Result<()> {
        let p = Picker::choices("Run", run_choices());
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| {
            p.render(&theme, frame, frame.area());
        })?;
        let buffer = terminal.backend().buffer();
        // 60 wide, 8 tall, centred.
        let card = Rect::new(20, 11, 60, 8);
        assert_eq!(Picker::choices_card(Rect::new(0, 0, 100, 30)), card);
        let row = |y: u16| -> String {
            (card.x..card.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        };
        assert_eq!(buffer[(card.x, card.y)].symbol(), "▀");
        assert!(
            row(card.y + 1).starts_with("  ✦ Run  "),
            "{:?}",
            row(card.y + 1)
        );
        let first = row(card.y + 2);
        assert!(first.starts_with("  dev         npm run dev"), "{first:?}");
        assert!(first.ends_with(".glyph.toml  ⏎    "), "{first:?}");
        assert_eq!(
            buffer[(card.x, card.y + 2)].bg,
            mix(theme.raised, theme.accent, 0.3)
        );
        assert_eq!(buffer[(card.x + 14, card.y + 3)].fg, theme.muted);
        let footer = row(card.bottom() - 1);
        assert!(
            footer.starts_with("  ↑↓ select   ⏎ run   esc close"),
            "{footer:?}"
        );
        assert!(footer.ends_with("2 of 2  "), "{footer:?}");
        Ok(())
    }

    #[test]
    fn choices_are_listed_in_order_and_picked_by_name() {
        let mut p = Picker::choices("Run", run_choices());
        assert_eq!(p.matches().collect::<Vec<_>>(), ["dev", "test"]);
        type_query(&mut p, "te");
        assert_eq!(
            p.handle(Input::Action(Action::Newline)),
            Some(Picked::Open("test".into()))
        );
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
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| {
            p.render(&theme, frame, frame.area());
        })?;
        let buffer = terminal.backend().buffer();
        let card = Picker::card(Rect::new(0, 0, 100, 30));
        let row = card.y + 2;
        let line: String = (card.x..card.right())
            .map(|x| buffer[(x, row)].symbol())
            .collect();
        assert!(line.contains("src/main.rs"), "{line:?}");
        let accented: String = (card.x..card.right())
            .filter(|&x| buffer[(x, row)].fg == theme.accent)
            .map(|x| buffer[(x, row)].symbol())
            .collect();
        assert_eq!(accented, "main");
        Ok(())
    }
}
