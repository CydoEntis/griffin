//! The project search panel: a centred card with a query line, the case and regex
//! toggles, and every line in the project the last search found, as
//! `path:line: text`. Enter searches; once the hits are in, Enter opens one.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, Clear};
use unicode_width::UnicodeWidthStr;

use crate::buffer::movement::Motion;
use crate::keymap::{Action, Input};
use crate::search::{Hit, Query};
use crate::theme::Theme;
use crate::ui::prompt::{Outcome, PromptBar};

/// Widest and tallest the card gets, borders included.
const MAX_WIDTH: u16 = 120;
const MAX_HEIGHT: u16 = 20;

/// What a key asked the editor to do, when it did more than move or edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Searched {
    /// Search the project for this, replacing the hits listed.
    Start(Query),
    /// Open this hit's file at its line and column.
    Open(Hit),
    /// Esc: close the panel, stopping any search still running.
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSearch {
    query: PromptBar,
    case_sensitive: bool,
    regex: bool,
    /// What the listed hits were found for; `None` until a search starts.
    searched: Option<Query>,
    /// Sorted by path, then line.
    hits: Vec<Hit>,
    /// Index into `hits`.
    selected: usize,
    /// The arrows have moved the selection since the search started, so it
    /// follows its hit as more stream in; until then it stays on the first.
    moved: bool,
    /// Which search the hits belong to; batches from an older one are dropped.
    id: u64,
    /// The background search hasn't said it's finished yet.
    running: bool,
    /// Why the last search found nothing, when its regex is invalid.
    error: Option<String>,
}

impl Default for ProjectSearch {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectSearch {
    pub fn new() -> Self {
        ProjectSearch {
            query: PromptBar::new("Search", ""),
            case_sensitive: false,
            regex: false,
            searched: None,
            hits: Vec::new(),
            selected: 0,
            moved: false,
            id: 0,
            running: false,
            error: None,
        }
    }

    fn query(&self) -> Query {
        Query {
            pattern: self.query.text().to_string(),
            case_sensitive: self.case_sensitive,
            regex: self.regex,
        }
    }

    #[cfg(test)]
    pub fn hits(&self) -> &[Hit] {
        &self.hits
    }

    pub fn selected(&self) -> Option<&Hit> {
        self.hits.get(self.selected)
    }

    /// The editor started search `id` for `query`: the old hits go.
    pub fn start(&mut self, id: u64, query: Query) {
        self.id = id;
        self.searched = Some(query);
        self.hits.clear();
        self.selected = 0;
        self.moved = false;
        self.running = true;
        self.error = None;
    }

    /// The search couldn't start, for the reason given.
    pub fn fail(&mut self, error: String) {
        self.hits.clear();
        self.selected = 0;
        self.running = false;
        self.error = Some(error);
    }

    /// A batch of hits from search `id`. Once the arrows have moved the
    /// selection it stays on its hit, so hits streaming in don't move it out from
    /// under them.
    pub fn add(&mut self, id: u64, hits: Vec<Hit>) {
        if id != self.id || hits.is_empty() {
            return;
        }
        let kept = self
            .selected()
            .filter(|_| self.moved)
            .map(|hit| (hit.path.clone(), hit.line.line));
        self.hits.extend(hits);
        self.hits
            .sort_by(|a, b| a.path.cmp(&b.path).then(a.line.line.cmp(&b.line.line)));
        if let Some((path, line)) = kept {
            self.selected = self
                .hits
                .iter()
                .position(|hit| hit.path == path && hit.line.line == line)
                .unwrap_or(0);
        }
    }

    /// Search `id` has looked at every file.
    pub fn finish(&mut self, id: u64) {
        if id == self.id {
            self.running = false;
        }
    }

    /// Up and Down move through the hits, Alt+C and Alt+R toggle case and regex
    /// (searching again once a search has run), Enter searches or, when the hits
    /// listed are for what's typed, opens the selected one; Esc closes.
    pub fn handle(&mut self, input: Input) -> Option<Searched> {
        match input {
            Input::Action(Action::Move(Motion::Up)) => {
                self.selected = self.selected.saturating_sub(1);
                self.moved = true;
                None
            }
            Input::Action(Action::Move(Motion::Down)) => {
                if self.selected + 1 < self.hits.len() {
                    self.selected += 1;
                }
                self.moved = true;
                None
            }
            Input::Action(Action::FindCase) => {
                self.case_sensitive = !self.case_sensitive;
                self.research()
            }
            Input::Action(Action::FindRegex) => {
                self.regex = !self.regex;
                self.research()
            }
            // The find bar's Enter, as the panel reads keys in its scope.
            Input::Action(Action::FindNext) => self.submit(),
            input => match self.query.handle(input)? {
                Outcome::Submit => self.submit(),
                Outcome::Cancel => Some(Searched::Close),
            },
        }
    }

    /// Pasted text goes into the query as if typed; a line break is Enter.
    pub fn paste(&mut self, text: &str) -> Option<Searched> {
        match self.query.paste(text)? {
            Outcome::Submit => self.submit(),
            Outcome::Cancel => Some(Searched::Close),
        }
    }

    fn submit(&mut self) -> Option<Searched> {
        let query = self.query();
        if query.pattern.is_empty() {
            return None;
        }
        if self.searched.as_ref() == Some(&query) {
            self.selected().cloned().map(Searched::Open)
        } else {
            Some(Searched::Start(query))
        }
    }

    /// A toggle changed what matches: the listed hits are stale, so search again
    /// if there are any to replace, as the find bar does.
    fn research(&mut self) -> Option<Searched> {
        let query = self.query();
        (self.searched.is_some() && !query.pattern.is_empty()).then_some(Searched::Start(query))
    }

    /// What the right end of the query line says: the error, the hit count, or
    /// that the search is still going.
    pub fn status(&self) -> String {
        if let Some(err) = &self.error {
            return err.clone();
        }
        if self.searched.is_none() {
            return String::new();
        }
        let count = match self.hits.len() {
            0 if !self.running => "no hits".to_string(),
            1 => "1 hit".to_string(),
            n => format!("{n} hits"),
        };
        if self.running {
            format!("searching… {count}")
        } else {
            count
        }
    }

    /// Where the card goes in `area`: centred, four fifths of the width and up to
    /// twenty rows tall.
    fn card(area: Rect) -> Rect {
        let width = (area.width * 4 / 5).clamp(40, MAX_WIDTH).min(area.width);
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

    /// Draws the card centred in `area`: a border, the query line with the toggles
    /// (in the accent while on) and the status at its right, then the hits with
    /// their matches in the accent and the selected one filled with `hov`.
    /// Returns where the cursor goes, in the query.
    pub fn render(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        let card = Self::card(area);
        frame.render_widget(Clear, card);
        let block = Block::bordered()
            .border_style(Style::new().fg(theme.border))
            .style(Style::new().bg(theme.card).fg(theme.text));
        let inner = block.inner(card);
        frame.render_widget(block, card);
        if inner.height == 0 || inner.width == 0 {
            return (card.x, card.y);
        }
        let cursor = self.render_query(theme, frame, Rect { height: 1, ..inner });

        let list = Rect {
            y: inner.y + 1,
            height: inner.height - 1,
            ..inner
        };
        let rows = usize::from(list.height);
        // Scrolls just far enough to keep the selection on the last row.
        let first = self.selected.saturating_sub(rows.saturating_sub(1));
        for (line, (index, hit)) in self
            .hits
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .enumerate()
        {
            let y = list.y + u16::try_from(line).unwrap_or(u16::MAX);
            render_hit(theme, frame, list, y, hit, index == self.selected);
        }
        cursor
    }

    fn render_query(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        let status = self.status();
        let base = Style::new().bg(theme.card2);
        let toggle = |on: bool| base.fg(if on { theme.accent } else { theme.muted });
        let status_style = if self.error.is_some() {
            base.fg(theme.err)
        } else {
            base.fg(theme.strong)
        };
        // Right-aligned: `Aa  .*  3 hits `, as in the find bar.
        let gap = base.fg(theme.strong);
        let parts = [
            ("Aa", toggle(self.case_sensitive)),
            ("  ", gap),
            (".*", toggle(self.regex)),
            ("  ", gap),
            (status.as_str(), status_style),
            (" ", gap),
        ];
        let at = self.query.render(theme, frame, area);
        let width: usize = parts.iter().map(|(text, _)| text.width()).sum();
        let mut x = area
            .right()
            .saturating_sub(u16::try_from(width).unwrap_or(u16::MAX));
        for (text, style) in parts {
            if x >= area.x {
                frame.buffer_mut().set_string(x, area.y, text, style);
            }
            x += u16::try_from(text.width()).unwrap_or(0);
        }
        at
    }
}

/// One `path:line: text` row of the list, one space in, its matches in the
/// accent; the selected row filled edge to edge.
fn render_hit(theme: &Theme, frame: &mut Frame, list: Rect, y: u16, hit: &Hit, selected: bool) {
    let width = usize::from(list.width);
    let base = if selected {
        Theme::highlight(theme.hov, theme.strong)
    } else {
        Style::new()
    };
    // On the selected row a match is a block of accent with `acc_ink` on it, since
    // accent text wouldn't read on `hov`.
    let match_style = if selected {
        Theme::highlight(theme.accent, theme.acc_ink)
    } else {
        Style::new().fg(theme.accent)
    };
    let place_style = if selected {
        base
    } else {
        Style::new().fg(theme.muted)
    };
    let out = frame.buffer_mut();
    out.set_stringn(list.x, y, format!("{:width$}", ""), width, base);
    let place = format!("{}:{}: ", hit.path, hit.line.line + 1);
    let cells =
        place
            .chars()
            .map(|ch| (ch, place_style))
            .chain(hit.line.text.chars().enumerate().map(|(at, ch)| {
                let matched = hit.line.matches.iter().any(|m| m.contains(&at));
                (ch, if matched { match_style } else { base })
            }));
    let mut x = list.x + 1;
    for (ch, style) in cells {
        let cell = ch.to_string();
        let cell_width = u16::try_from(cell.width()).unwrap_or(1);
        if x + cell_width > list.right() {
            break;
        }
        out.set_string(x, y, &cell, style);
        x += cell_width;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::LineHit;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[expect(
        clippy::single_range_in_vec_init,
        reason = "a list of one match, not a range of numbers"
    )]
    fn hit(path: &str, line: usize, text: &str) -> Hit {
        Hit {
            path: path.to_string(),
            line: LineHit {
                line,
                col: 0,
                text: text.to_string(),
                matches: vec![0..4],
            },
        }
    }

    fn type_query(search: &mut ProjectSearch, text: &str) {
        for c in text.chars() {
            assert_eq!(search.handle(Input::Text(c)), None);
        }
    }

    fn query(pattern: &str, case_sensitive: bool, regex: bool) -> Query {
        Query {
            pattern: pattern.to_string(),
            case_sensitive,
            regex,
        }
    }

    #[test]
    fn enter_searches_then_opens_the_selected_hit() {
        let mut search = ProjectSearch::new();
        let enter = Input::Action(Action::FindNext);
        // Nothing to search for yet.
        assert_eq!(search.handle(enter), None);
        type_query(&mut search, "TODO");
        let started = search.handle(enter);
        assert_eq!(started, Some(Searched::Start(query("TODO", false, false))));
        search.start(1, query("TODO", false, false));
        assert_eq!(search.status(), "searching… 0 hits");
        // Enter before any hit arrives has nothing to open.
        assert_eq!(search.handle(enter), None);
        search.add(1, vec![hit("b.rs", 0, "TODO b"), hit("a.rs", 4, "TODO a")]);
        search.finish(1);
        assert_eq!(search.status(), "2 hits");
        assert_eq!(search.selected().map(|h| h.path.as_str()), Some("a.rs"));
        search.handle(Input::Action(Action::Move(Motion::Down)));
        search.handle(Input::Action(Action::Move(Motion::Down)));
        assert_eq!(
            search.handle(enter),
            Some(Searched::Open(hit("b.rs", 0, "TODO b")))
        );
        // Editing the query makes Enter search again.
        type_query(&mut search, "!");
        assert_eq!(
            search.handle(enter),
            Some(Searched::Start(query("TODO!", false, false)))
        );
        assert_eq!(
            search.handle(Input::Action(Action::Cancel)),
            Some(Searched::Close)
        );
    }

    #[test]
    fn toggles_search_again_once_a_search_has_run() {
        let mut search = ProjectSearch::new();
        type_query(&mut search, "a.c");
        // Before the first Enter a toggle only changes the next search.
        assert_eq!(search.handle(Input::Action(Action::FindCase)), None);
        search.start(1, query("a.c", true, false));
        assert_eq!(
            search.handle(Input::Action(Action::FindRegex)),
            Some(Searched::Start(query("a.c", true, true)))
        );
        search.fail("invalid regex".into());
        assert_eq!(search.status(), "invalid regex");
    }

    #[test]
    fn late_batches_keep_a_moved_selection_and_stale_ones_are_dropped() {
        let mut search = ProjectSearch::new();
        search.start(2, query("x", false, false));
        search.add(2, vec![hit("m.rs", 0, "x"), hit("z.rs", 0, "x")]);
        // Unmoved, the selection stays on whatever sorts first.
        search.add(2, vec![hit("c.rs", 0, "x")]);
        assert_eq!(search.selected().map(|h| h.path.as_str()), Some("c.rs"));
        search.handle(Input::Action(Action::Move(Motion::Down)));
        search.handle(Input::Action(Action::Move(Motion::Down)));
        search.add(2, vec![hit("a.rs", 0, "x")]);
        assert_eq!(search.selected().map(|h| h.path.as_str()), Some("z.rs"));
        search.add(1, vec![hit("b.rs", 0, "x")]);
        assert_eq!(search.hits().len(), 4);
        search.finish(1);
        assert_eq!(search.status(), "searching… 4 hits");
        search.finish(2);
        assert_eq!(search.status(), "4 hits");
    }

    #[test]
    fn hits_render_as_path_line_text_with_the_match_in_the_accent() -> anyhow::Result<()> {
        let mut search = ProjectSearch::new();
        type_query(&mut search, "TODO");
        search.start(1, query("TODO", false, false));
        search.add(
            1,
            vec![hit("src/lib.rs", 2, "TODO later"), hit("x.rs", 0, "TODO")],
        );
        search.finish(1);
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| {
            search.render(&theme, frame, frame.area());
        })?;
        let buffer = terminal.backend().buffer();
        let card = ProjectSearch::card(Rect::new(0, 0, 100, 30));
        let line = |row: u16| -> String {
            (card.x..card.right())
                .map(|x| buffer[(x, row)].symbol())
                .collect()
        };
        assert!(
            line(card.y + 1).contains("Search: TODO"),
            "{}",
            line(card.y + 1)
        );
        assert!(line(card.y + 1).contains("Aa  .*  2 hits"));
        // The second hit isn't selected, so its match is accent text.
        let row = card.y + 3;
        assert!(line(row).contains(" x.rs:1: TODO"), "{}", line(row));
        let accented: String = (card.x..card.right())
            .filter(|&x| buffer[(x, row)].fg == theme.accent)
            .map(|x| buffer[(x, row)].symbol())
            .collect();
        assert_eq!(accented, "TODO");
        assert!(line(card.y + 2).contains("src/lib.rs:3: TODO later"));

        // A toggle that's on is the only accent on the query line.
        search.handle(Input::Action(Action::FindRegex));
        terminal.draw(|frame| {
            search.render(&theme, frame, frame.area());
        })?;
        let buffer = terminal.backend().buffer();
        let accented: String = (card.x..card.right())
            .filter(|&x| buffer[(x, card.y + 1)].fg == theme.accent)
            .map(|x| buffer[(x, card.y + 1)].symbol())
            .collect();
        assert_eq!(accented, ".*");
        Ok(())
    }
}
