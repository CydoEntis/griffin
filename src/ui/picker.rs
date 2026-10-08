//! The picker: a query over a list, filtered fuzzily as the query is typed, best
//! match first. Go to file lists the project's files in the "cast" palette (design
//! README §3); F5 lists the `[[run]]` entries by name in a centred card.

use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;
use unicode_width::UnicodeWidthStr;

use crate::buffer::movement::Motion;
use crate::keymap::{Action, Input};
use crate::theme::Theme;
use crate::ui::prompt::{Outcome, PromptBar};
use crate::ui::{dialog_card, enter_mark, glow_row};

/// Widest and tallest the choices card gets, borders included.
const MAX_WIDTH: u16 = 80;
const MAX_HEIGHT: u16 = 20;

/// The cast card's width, before clamping to the screen, and the row it drops
/// from (design README §3).
const CAST_WIDTH: u16 = 86;
const CAST_TOP: u16 = 4;
/// Most file rows the cast shows; the selection scrolls through the rest.
const CAST_ROWS: usize = 5;
/// The cast's rows other than the file rows: the lit edge, a blank, the query,
/// the rule and the `FILES` header above them; a blank, the footer and a blank
/// below.
const CAST_CHROME: u16 = 8;
/// Right of the query: the palette's name, then what it lists.
const CAST_NAME: &str = "cast";
const CAST_SCOPE: &str = " · files · commands";
/// The footer's prefixes, each with what it switches to, and the keys.
const CAST_PREFIXES: [(&str, &str); 3] = [(">", "commands"), (":", "line"), ("/", "text")];
const CAST_KEYS: &str = "↑↓  ⏎  esc";

/// What a key did to the picker, when it did more than move or edit the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    /// Enter on an item: for go to file, its path relative to the project root,
    /// `/`-separated; for a list of choices, the choice as given.
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
    /// Every file in the project, or the choices; `None` until the background
    /// walk reports back.
    files: Option<Vec<String>>,
    /// Shown when nothing matches the query.
    no_match: &'static str,
    /// Drawn as the cast palette rather than the plain card.
    cast: bool,
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
            no_match: "no matching files",
            cast: true,
            matches: Vec::new(),
            selected: 0,
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
        }
    }

    /// A picker titled `title` over `choices`, listed in the order given.
    pub fn choices(title: &'static str, choices: Vec<String>) -> Self {
        let mut picker = Picker {
            query: PromptBar::new(title, ""),
            files: None,
            no_match: "no match",
            cast: false,
            matches: Vec::new(),
            selected: 0,
            matcher: Matcher::new(Config::DEFAULT),
        };
        picker.set_files(choices);
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

    /// The char indices in `path` of the letters the query matched, sorted.
    fn matched(&self, path: &str, matcher: &mut Matcher, indices: &mut Vec<u32>) {
        let pattern = Pattern::parse(self.query.text(), CaseMatching::Smart, Normalization::Smart);
        let mut chars = Vec::new();
        indices.clear();
        pattern.indices(Utf32Str::new(path, &mut chars), matcher, indices);
        indices.sort_unstable();
        indices.dedup();
    }

    /// Where the picker is drawn in `area`, so a click outside it can close it.
    pub fn card(&self, area: Rect) -> Rect {
        if self.cast {
            self.cast_card(area)
        } else {
            Self::list_card(area)
        }
    }

    /// Draws the picker over `area` and returns where the cursor goes, in the
    /// query.
    pub fn render(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        if self.cast {
            self.render_cast(theme, frame, area)
        } else {
            self.render_list(theme, frame, area)
        }
    }

    /// How many file rows the cast shows: one for a message when there's
    /// nothing to list.
    fn cast_rows(&self) -> u16 {
        let rows = self.matches.len().clamp(1, CAST_ROWS);
        u16::try_from(rows).unwrap_or(1)
    }

    /// The cast card in `area`: 86 wide or the screen less four, centred, from
    /// row 4, as tall as its file rows need (design README §3).
    fn cast_card(&self, area: Rect) -> Rect {
        let width = CAST_WIDTH.min(area.width.saturating_sub(4));
        let y = area.y + CAST_TOP.min(area.height);
        let height = (CAST_CHROME + self.cast_rows()).min(area.bottom() - y);
        Rect {
            x: area.x + (area.width - width) / 2,
            y,
            width,
            height,
        }
    }

    /// Dims `area` and draws the cast card over it: the lit edge, `✦` and the
    /// query with the palette's name at the right, a rule, then up to five files
    /// under `FILES`, the selected one the glow row with `⏎`; then the footer.
    fn render_cast(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        crate::ui::dim(theme, frame.buffer_mut(), area);
        let card = self.cast_card(area);
        dialog_card(theme, frame, card);
        let rows = self.cast_rows();
        if card.height < CAST_CHROME + rows || card.width < 40 {
            return (card.x, card.y);
        }
        let out = frame.buffer_mut();
        let left = card.x + 4;
        let right = card.right() - 4;
        let muted = Style::new().fg(theme.muted);

        let query_y = card.y + 2;
        let scope_x = right - width_of(CAST_SCOPE);
        let name_x = scope_x - width_of(CAST_NAME);
        out.set_string(name_x, query_y, CAST_NAME, Style::new().fg(theme.accent));
        out.set_string(scope_x, query_y, CAST_SCOPE, muted);
        out.set_string(
            left,
            query_y,
            "✦",
            Style::new().fg(theme.accent2).add_modifier(Modifier::BOLD),
        );
        // Two blanks short of the palette's name, so a long query never runs
        // into it.
        let query = Rect::new(left + 2, query_y, name_x.saturating_sub(left + 4), 1);
        let cursor = self
            .query
            .render_value(out, query, Style::new().fg(theme.strong));

        for x in card.x + 2..card.right() - 2 {
            out[(x, card.y + 3)].set_symbol("─").set_fg(theme.line2);
        }
        out.set_string(
            left,
            card.y + 4,
            "FILES",
            muted.add_modifier(Modifier::BOLD),
        );

        let first_row = card.y + 5;
        match &self.files {
            None => {
                out.set_string(left, first_row, "listing files…", muted);
            }
            Some(_) if self.matches.is_empty() => {
                out.set_string(left, first_row, self.no_match, muted);
            }
            Some(files) => {
                // Scrolls just far enough to keep the selection on the last row.
                let first = self.selected.saturating_sub(CAST_ROWS - 1);
                let mut matcher = self.matcher.clone();
                let mut indices = Vec::new();
                for (line, (index, m)) in self
                    .matches
                    .iter()
                    .enumerate()
                    .skip(first)
                    .take(CAST_ROWS)
                    .enumerate()
                {
                    let path = files[m.file].as_str();
                    self.matched(path, &mut matcher, &mut indices);
                    let y = first_row + u16::try_from(line).unwrap_or(u16::MAX);
                    let selected = index == self.selected;
                    if selected {
                        glow_row(theme, out, card.x, card.right(), y);
                        enter_mark(theme, out, right, y);
                    }
                    // Two blanks short of `⏎`, so a long path never runs into it.
                    let room = Rect::new(left, y, right.saturating_sub(left + 3), 1);
                    cast_file(theme, out, room, path, &indices, selected);
                }
            }
        }

        let footer_y = first_row + rows + 1;
        let lit = Style::new().fg(theme.accent).add_modifier(Modifier::BOLD);
        let mut x = left;
        for (i, (prefix, label)) in CAST_PREFIXES.iter().enumerate() {
            if i > 0 {
                x += 3;
            }
            x = out.set_stringn(x, footer_y, prefix, usize::MAX, lit).0;
            x = out
                .set_stringn(x, footer_y, format!(" {label}"), usize::MAX, muted)
                .0;
        }
        out.set_string(right - width_of(CAST_KEYS), footer_y, CAST_KEYS, muted);
        cursor
    }

    /// Where the choices card goes in `area`: centred, three fifths of the width
    /// and up to twenty rows tall.
    fn list_card(area: Rect) -> Rect {
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

    /// Draws the choices card centred in `area`: a border, the query line, then
    /// the matches with their matched letters in `accent` and the selected one
    /// filled with `hov`. Returns where the cursor goes, in the query.
    fn render_list(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        let card = Self::list_card(area);
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
                list.x + 1,
                list.y,
                "listing files…",
                width.saturating_sub(1),
                Style::new().fg(theme.muted),
            );
            return cursor;
        };
        if self.matches.is_empty() {
            out.set_stringn(
                list.x + 1,
                list.y,
                self.no_match,
                width.saturating_sub(1),
                Style::new().fg(theme.muted),
            );
            return cursor;
        }

        let rows = usize::from(list.height);
        // Scrolls just far enough to keep the selection on the last row.
        let first = self.selected.saturating_sub(rows.saturating_sub(1));
        let mut matcher = self.matcher.clone();
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
            self.matched(path, &mut matcher, &mut indices);

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

/// How many cells `text` takes.
fn width_of(text: &str) -> u16 {
    u16::try_from(text.width()).unwrap_or(u16::MAX)
}

/// One file row of the cast in `room`: the file's name, its matched letters
/// `accent` bold and the rest `fg` (`strong` when selected), then two blanks and
/// its folder in `muted`, any matched letters there in `accent`. `matched` holds
/// the char indices of the matched letters in `path`, sorted. On the selected
/// row in `mono`, which is reverse video, the only styling is bold.
fn cast_file(
    theme: &Theme,
    out: &mut Buffer,
    room: Rect,
    path: &str,
    matched: &[u32],
    selected: bool,
) {
    let plain = selected && !theme.ramps();
    let pick = |style: Style| if plain { Style::new() } else { style };
    let name_hit = pick(Style::new().fg(theme.accent)).add_modifier(Modifier::BOLD);
    let name_rest = pick(Style::new().fg(if selected { theme.strong } else { theme.fg }));
    let dir_hit = if plain {
        Style::new().add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(theme.accent)
    };
    let dir_rest = pick(Style::new().fg(theme.muted));

    let chars: Vec<char> = path.chars().collect();
    // The name starts after the last `/`; a file at the root has no folder.
    let base = chars.iter().rposition(|&c| c == '/').map_or(0, |at| at + 1);
    // Draws the char at `at` at `x` and moves `x` past it; false once it
    // wouldn't fit.
    let mut put = |x: &mut u16, at: usize, hit: Style, rest: Style| -> bool {
        let cell = chars[at].to_string();
        let cells = width_of(&cell);
        if *x + cells > room.right() {
            return false;
        }
        let is_hit = u32::try_from(at).is_ok_and(|at| matched.binary_search(&at).is_ok());
        out.set_string(*x, room.y, &cell, if is_hit { hit } else { rest });
        *x += cells;
        true
    };
    let mut x = room.x;
    for at in base..chars.len() {
        if !put(&mut x, at, name_hit, name_rest) {
            return;
        }
    }
    // Two blanks, then the folder without the `/` that ends it.
    x += 2;
    for at in 0..base.saturating_sub(1) {
        if !put(&mut x, at, dir_hit, dir_rest) {
            return;
        }
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
    fn choices_are_listed_in_order_and_picked_by_name() {
        let mut p = Picker::choices("Run", vec!["dev".into(), "test".into()]);
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
    fn the_cast_card_drops_from_row_four_and_fits_its_rows() {
        let area = Rect::new(0, 0, 160, 45);
        let mut p = Picker::new();
        // Waiting for the walk: one row for the message.
        assert_eq!(p.card(area), Rect::new(37, 4, 86, 9));
        p.set_files(FILES.iter().map(|f| f.to_string()).collect());
        assert_eq!(p.card(area), Rect::new(37, 4, 86, 13));
        // Clamped to the screen less four at the smallest size.
        assert_eq!(p.card(Rect::new(0, 0, 88, 30)).width, 84);
    }

    #[test]
    fn the_cast_says_so_while_the_walk_is_still_listing_files() -> anyhow::Result<()> {
        // Before the walk reports back, which a PTY test can't wait for.
        let p = Picker::new();
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| {
            p.render(&theme, frame, frame.area());
        })?;
        let buffer = terminal.backend().buffer();
        let card = p.card(Rect::new(0, 0, 100, 30));
        let line: String = (card.x..card.right())
            .map(|x| buffer[(x, card.y + 5)].symbol())
            .collect();
        assert!(line.starts_with("    listing files…"), "{line:?}");
        Ok(())
    }

    #[test]
    fn a_file_row_shows_the_name_lit_then_its_folder() -> anyhow::Result<()> {
        let mut p = picker(FILES);
        type_query(&mut p, "main");
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| {
            p.render(&theme, frame, frame.area());
        })?;
        let buffer = terminal.backend().buffer();
        let card = p.card(Rect::new(0, 0, 100, 30));
        let row = card.y + 5;
        let line: String = (card.x..card.right())
            .map(|x| buffer[(x, row)].symbol())
            .collect();
        assert!(line.starts_with("    main.rs  src"), "{line:?}");
        let lit: String = (card.x..card.right())
            .filter(|&x| {
                let cell = &buffer[(x, row)];
                cell.fg == theme.accent && cell.modifier.contains(Modifier::BOLD)
            })
            .map(|x| buffer[(x, row)].symbol())
            .collect();
        assert_eq!(lit, "main");
        assert_eq!(buffer[(card.x + 13, row)].fg, theme.muted);
        Ok(())
    }

    #[test]
    fn the_choices_card_keeps_its_query_line_and_list() -> anyhow::Result<()> {
        let mut p = Picker::choices("Run", vec!["build".into(), "test".into()]);
        type_query(&mut p, "te");
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| {
            p.render(&theme, frame, frame.area());
        })?;
        let buffer = terminal.backend().buffer();
        let card = p.card(Rect::new(0, 0, 100, 30));
        let row = card.y + 2;
        let line: String = (card.x..card.right())
            .map(|x| buffer[(x, row)].symbol())
            .collect();
        assert!(line.contains("test"), "{line:?}");
        Ok(())
    }
}
