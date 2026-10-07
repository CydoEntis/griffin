//! The project search panel: a dialog card over the dimmed screen with a Search
//! field and its case and regex chips, a Replace field under it, and every line
//! in the project the last search found, as `path:line  text`. Enter searches;
//! once the hits are in, Enter opens one. Tab moves between the fields, and
//! Alt+A replaces the hits in every listed file.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::buffer::movement::Motion;
use crate::keymap::{Action, Input};
use crate::search::{Hit, Query};
use crate::theme::{Theme, mix};
use crate::ui::prompt::{Outcome, PromptBar};
use crate::ui::{enter_mark, footer, glow_row, input_row};

/// What a key asked the editor to do, when it did more than move or edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Searched {
    /// Search the project for this, replacing the hits listed.
    Start(Query),
    /// Open this hit's file at its line and column.
    Open(Hit),
    /// Alt+A: replace the listed hits' query with `with` in these files, the
    /// listed ones, by the paths the list shows.
    Replace {
        query: Query,
        with: String,
        files: Vec<String>,
    },
    /// Esc: close the panel, stopping any search still running.
    Close,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSearch {
    query: PromptBar,
    /// What Alt+A puts in place of each match.
    replace: PromptBar,
    /// Keys go to the Replace field instead of the query; Tab moves between them.
    in_replace: bool,
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
            replace: PromptBar::new("Replace", ""),
            in_replace: false,
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
    /// listed are for what's typed, opens the selected one; Tab moves between the
    /// query and the Replace field; Alt+A replaces; Esc closes.
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
            Input::Action(Action::Tab) => {
                self.in_replace = !self.in_replace;
                None
            }
            Input::Action(Action::ProjectReplace) => self.replace_all(),
            input => match self.field().handle(input)? {
                Outcome::Submit => self.submit(),
                Outcome::Cancel => Some(Searched::Close),
            },
        }
    }

    /// The field keys type into.
    fn field(&mut self) -> &mut PromptBar {
        if self.in_replace {
            &mut self.replace
        } else {
            &mut self.query
        }
    }

    /// Pasted text goes into the field as if typed; a line break is Enter. A tab
    /// moves between the fields as the key does: on Windows Tab followed quickly
    /// by typing arrives as one paste.
    pub fn paste(&mut self, text: &str) -> Option<Searched> {
        for (i, part) in text.split('\t').enumerate() {
            if i > 0 {
                self.in_replace = !self.in_replace;
            }
            match self.field().paste(part) {
                None => {}
                Some(Outcome::Submit) => return self.submit(),
                Some(Outcome::Cancel) => return Some(Searched::Close),
            }
        }
        None
    }

    /// Alt+A: once a search has finished with hits, replace them in every
    /// file listed, with what the Replace field holds.
    fn replace_all(&self) -> Option<Searched> {
        let query = self.searched.clone()?;
        if self.running || self.hits.is_empty() {
            return None;
        }
        let mut files: Vec<String> = self.hits.iter().map(|hit| hit.path.clone()).collect();
        // Sorted by path already, so each file's hits sit together.
        files.dedup();
        Some(Searched::Replace {
            query,
            with: self.replace.text().to_string(),
            files,
        })
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

    /// What the row under the fields says: the error, how many matches in how
    /// many files, or that the search is still going (SPEC_V1_LAYOUT §7.3).
    pub fn status(&self) -> String {
        if let Some(err) = &self.error {
            return err.clone();
        }
        if self.searched.is_none() {
            return "type to search the project (.gitignore respected)".to_string();
        }
        let matches: usize = self.hits.iter().map(|hit| hit.line.matches.len()).sum();
        // Hits are sorted by path, so each file's sit together.
        let files = self
            .hits
            .iter()
            .enumerate()
            .filter(|&(i, hit)| i == 0 || self.hits[i - 1].path != hit.path)
            .count();
        let count = if matches == 0 && !self.running {
            "no matches".to_string()
        } else {
            format!(
                "{} in {}",
                plural(matches, "match", "matches"),
                plural(files, "file", "files")
            )
        };
        if self.running {
            format!("searching… {count}")
        } else {
            count
        }
    }

    /// Where the card goes in `area` (SPEC_V1_LAYOUT §7.3): centred, 20 cells
    /// narrower than the screen and 8 rows shorter, within 60..=140 by 14..=40.
    pub fn card(area: Rect) -> Rect {
        let width = area.width.saturating_sub(20).clamp(60, 140).min(area.width);
        let height = area.height.saturating_sub(8).clamp(14, 40).min(area.height);
        Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height - height) / 2,
            width,
            height,
        }
    }

    /// Dims `area` and draws the card centred in it: the lit top edge, the Search
    /// and Replace fields, the match count, the hits with the selected one as the
    /// glow row, and the key hints. Returns where the cursor goes, in the field
    /// that takes the keys.
    pub fn render(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        crate::ui::dim(theme, frame.buffer_mut(), area);
        let card = Self::card(area);
        crate::ui::dialog_card(theme, frame, card);
        // Edge, two fields, the count and the footer, plus a hit.
        if card.height < 6 || card.width < 30 {
            return (card.x, card.y);
        }
        let row = |n: u16| Rect {
            y: card.y + n,
            height: 1,
            ..card
        };
        let chips = Rect {
            x: card.right() - CHIPS_WIDTH,
            width: CHIPS_WIDTH,
            ..row(1)
        };
        let query_at = input_row(
            theme,
            frame,
            Rect {
                width: card.width - CHIPS_WIDTH,
                ..row(1)
            },
            "Search",
            LABEL_WIDTH,
            &self.query,
            !self.in_replace,
        );
        self.render_chips(theme, frame.buffer_mut(), chips);
        let replace_at = input_row(
            theme,
            frame,
            row(2),
            "Replace",
            LABEL_WIDTH,
            &self.replace,
            self.in_replace,
        );
        let out = frame.buffer_mut();
        if !self.in_replace && self.replace.text().is_empty() {
            out.set_string(
                replace_at.0,
                replace_at.1,
                "tab to replace",
                Style::new().fg(theme.muted),
            );
        }
        let status_style = Style::new().fg(if self.error.is_some() {
            theme.err
        } else {
            theme.muted
        });
        out.set_stringn(
            card.x + 2,
            card.y + 3,
            self.status(),
            usize::from(card.width - 4),
            status_style,
        );

        let rows = usize::from(card.height - 5);
        // The `path:line` column is as wide as the widest, up to 34, so the
        // lines' text starts in one column.
        let place_width = self
            .hits
            .iter()
            .map(|hit| place(hit).width())
            .max()
            .unwrap_or(0)
            .min(34);
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
            let y = card.y + 4 + u16::try_from(line).unwrap_or(u16::MAX);
            let selected = index == self.selected;
            render_hit(theme, out, card, y, hit, place_width, selected);
        }
        let tab = if self.in_replace { "search" } else { "replace" };
        footer(
            theme,
            out,
            card,
            &format!("↑↓ select   ⏎ open   tab {tab} field   alt+a replace all   esc close"),
        );
        if self.in_replace {
            replace_at
        } else {
            query_at
        }
    }

    /// The case and regex chips at the right of the Search field: ` Aa ` and
    /// ` .* `, `acc_ink` on the accent and bold while on, `muted` while off.
    fn render_chips(&self, theme: &Theme, out: &mut Buffer, area: Rect) {
        out.set_style(area, Style::new().bg(theme.raised2));
        let chip = |on: bool| {
            if on {
                Style::new()
                    .fg(theme.acc_ink)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(theme.muted)
            }
        };
        out.set_string(area.x, area.y, " Aa ", chip(self.case_sensitive));
        out.set_string(area.x + 5, area.y, " .* ", chip(self.regex));
    }
}

/// ` Aa ` ` .* ` and three blanks to the card's right edge.
const CHIPS_WIDTH: u16 = 12;
/// The fields' labels, `Search` and `Replace`, padded to one width so the values
/// line up.
const LABEL_WIDTH: u16 = 7;

/// `count` and the noun that goes with it, e.g. `1 file` or `2 files`.
fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

/// A hit's `path:line`, the line counting from 1.
fn place(hit: &Hit) -> String {
    format!("{}:{}", hit.path, hit.line.line + 1)
}

/// One hit across the card (SPEC_V1_LAYOUT §7.3): `path:line` two cells in,
/// padded to `place_width` (directory `muted`, file name `strong`), then two
/// blanks and the line, scrolled so its first match shows, each match on
/// `find_match_bg`. The selected row is the glow row with `⏎` at its right and
/// its matches in `find_current` colours.
fn render_hit(
    theme: &Theme,
    out: &mut Buffer,
    card: Rect,
    y: u16,
    hit: &Hit,
    place_width: usize,
    selected: bool,
) {
    let ramps = theme.ramps();
    if selected {
        glow_row(theme, out, card.x, card.right(), y);
        enter_mark(theme, out, card.right() - 4, y);
    }
    // In mono the selected row is reverse video, which colours of its own would
    // break up.
    let plain = selected && !ramps;
    let style = |style: Style| if plain { Style::new() } else { style };
    let muted = style(Style::new().fg(if selected { theme.text } else { theme.muted }));
    let matched = if !ramps {
        Style::new().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else if selected {
        Style::new().fg(theme.bg).bg(theme.warn)
    } else {
        Style::new().fg(theme.fg).bg(mix(theme.bg, theme.warn, 0.3))
    };

    let (dir, name) = match hit.path.rfind('/') {
        Some(at) => hit.path.split_at(at + 1),
        None => ("", hit.path.as_str()),
    };
    let line = format!(":{}", hit.line.line + 1);
    let mut cells: Vec<(char, Style)> = dir
        .chars()
        .map(|c| (c, muted))
        .chain(
            name.chars()
                .map(|c| (c, style(Style::new().fg(theme.strong)))),
        )
        .chain(line.chars().map(|c| (c, muted)))
        .collect();
    // Too long for the column: keep the end, where the file name and line are.
    if cells.len() > place_width {
        let cut = cells.len() + 1 - place_width;
        cells.splice(..cut, [('…', muted)]);
    }
    let place_end = card.x + 2 + u16::try_from(place_width).unwrap_or(u16::MAX);
    put_cells(out, card.x + 2, y, place_end, &cells);

    let text_x = place_end + 2;
    // One blank before `⏎`, which sits four cells in from the right.
    let end = card.right().saturating_sub(6);
    let room = usize::from(end.saturating_sub(text_x));
    let first_match = hit.line.matches.first().map_or(0, |m| m.start);
    // Scrolled so the match lands 20 cells in when it would be past the room.
    let start = if first_match + 20 > room {
        first_match.saturating_sub(20)
    } else {
        0
    };
    let mut cells: Vec<(char, Style)> = Vec::new();
    if start > 0 {
        cells.push(('…', style(Style::new().fg(theme.muted))));
    }
    cells.extend(
        hit.line
            .text
            .chars()
            .enumerate()
            .skip(start)
            .map(|(at, c)| {
                let in_match = hit.line.matches.iter().any(|m| m.contains(&at));
                let text = style(Style::new().fg(theme.fg));
                (c, if in_match { matched } else { text })
            }),
    );
    put_cells(out, text_x, y, end, &cells);
}

/// Draws `cells` from `x` on row `y`, stopping before a cell would cross `end`.
fn put_cells(out: &mut Buffer, mut x: u16, y: u16, end: u16, cells: &[(char, Style)]) {
    for &(c, style) in cells {
        let width = u16::try_from(c.width().unwrap_or(0)).unwrap_or(1);
        if x + width > end {
            break;
        }
        out.set_string(x, y, c.to_string(), style);
        x += width;
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
        assert_eq!(search.status(), "searching… 0 matches in 0 files");
        // Enter before any hit arrives has nothing to open.
        assert_eq!(search.handle(enter), None);
        search.add(1, vec![hit("b.rs", 0, "TODO b"), hit("a.rs", 4, "TODO a")]);
        search.finish(1);
        assert_eq!(search.status(), "2 matches in 2 files");
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
    fn tab_moves_to_replace_and_alt_a_asks_for_the_listed_files() {
        let mut search = ProjectSearch::new();
        let replace = Input::Action(Action::ProjectReplace);
        type_query(&mut search, "TODO");
        // Nothing listed yet, so nothing to replace.
        assert_eq!(search.handle(replace), None);
        assert_eq!(search.handle(Input::Action(Action::Tab)), None);
        type_query(&mut search, "DONE");
        search.start(1, query("TODO", false, false));
        search.add(
            1,
            vec![
                hit("b.rs", 0, "TODO"),
                hit("a.rs", 4, "TODO"),
                hit("a.rs", 1, "TODO"),
            ],
        );
        // Still searching: the list isn't complete.
        assert_eq!(search.handle(replace), None);
        search.finish(1);
        assert_eq!(
            search.handle(replace),
            Some(Searched::Replace {
                query: query("TODO", false, false),
                with: "DONE".into(),
                files: vec!["a.rs".into(), "b.rs".into()],
            })
        );
        // Tab goes back to the query, which typing then edits.
        search.handle(Input::Action(Action::Tab));
        type_query(&mut search, "!");
        assert_eq!(search.query().pattern, "TODO!");
        assert_eq!(search.replace.text(), "DONE");
        // A pasted tab switches fields too.
        assert_eq!(search.paste("?\tX"), None);
        assert_eq!(search.query().pattern, "TODO!?");
        assert_eq!(search.replace.text(), "DONEX");
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
        assert_eq!(search.status(), "searching… 4 matches in 4 files");
        search.finish(2);
        assert_eq!(search.status(), "4 matches in 4 files");
    }

    #[test]
    fn renders_a_dimmed_dialog_with_fields_and_aligned_hits() -> anyhow::Result<()> {
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
        let draw = |terminal: &mut Terminal<TestBackend>, search: &ProjectSearch| {
            terminal
                .draw(|frame| {
                    let area = frame.area();
                    frame
                        .buffer_mut()
                        .set_style(area, Style::new().bg(theme.bg).fg(theme.fg));
                    search.render(&theme, frame, area);
                })
                .map(|_| ())
        };
        draw(&mut terminal, &search)?;
        let buffer = terminal.backend().buffer();
        // SPEC_V1_LAYOUT §7.3 at 100x30: 80 wide, 22 tall, centred.
        let card = ProjectSearch::card(Rect::new(0, 0, 100, 30));
        assert_eq!(card, Rect::new(10, 4, 80, 22));
        let line = |row: u16| -> String {
            (card.x..card.right())
                .map(|x| buffer[(x, row)].symbol())
                .collect()
        };
        // Outside the card everything is dimmed towards the scrim.
        assert_eq!(buffer[(0, 0)].bg, mix(theme.bg, theme.scrim, 0.6));
        assert_eq!(buffer[(0, 0)].fg, mix(theme.fg, theme.scrim, 0.6));
        assert_eq!(line(card.y), "▀".repeat(80));
        assert!(
            line(card.y + 1).starts_with("  ✦ Search   TODO"),
            "{}",
            line(card.y + 1)
        );
        assert!(line(card.y + 1).ends_with(" Aa   .*    "));
        assert_eq!(buffer[(card.x + 1, card.y + 1)].bg, theme.raised2);
        assert!(line(card.y + 2).starts_with("  ✦ Replace  tab to replace"));
        assert!(line(card.y + 3).starts_with("  2 matches in 2 files"));
        // `src/lib.rs:3` sets the path column's width; the text lines up after it.
        assert!(line(card.y + 4).starts_with("  src/lib.rs:3  TODO later"));
        assert!(line(card.y + 5).starts_with("  x.rs:1        TODO"));
        assert!(line(card.y + 21).starts_with("  ↑↓ select   ⏎ open   tab replace field"));
        // The first hit is selected: the glow row, with `⏎` at its right.
        let selected = card.y + 4;
        assert_eq!(
            buffer[(card.x, selected)].bg,
            mix(theme.raised, theme.accent, 0.3)
        );
        assert_eq!(buffer[(card.right() - 5, selected)].symbol(), "⏎");
        // The other hit's match sits on `find_match_bg`.
        let match_bg = mix(theme.bg, theme.warn, 0.3);
        let matched: String = (card.x..card.right())
            .filter(|&x| buffer[(x, card.y + 5)].bg == match_bg)
            .map(|x| buffer[(x, card.y + 5)].symbol())
            .collect();
        assert_eq!(matched, "TODO");

        // A chip that's on is `acc_ink` on the accent.
        search.handle(Input::Action(Action::FindRegex));
        draw(&mut terminal, &search)?;
        let buffer = terminal.backend().buffer();
        let on: String = (card.x..card.right())
            .filter(|&x| buffer[(x, card.y + 1)].bg == theme.accent)
            .map(|x| buffer[(x, card.y + 1)].symbol())
            .collect();
        assert_eq!(on, " .* ");
        Ok(())
    }

    #[test]
    fn mono_dims_with_the_modifier_and_reverses_the_selected_row() -> anyhow::Result<()> {
        let mut search = ProjectSearch::new();
        search.start(1, query("x", false, false));
        search.add(1, vec![hit("a.rs", 0, "xxxx")]);
        let mono = Theme::named("mono").expect("mono exists");
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| {
            search.render(&mono, frame, frame.area());
        })?;
        let buffer = terminal.backend().buffer();
        let card = ProjectSearch::card(Rect::new(0, 0, 100, 30));
        assert!(buffer[(0, 0)].modifier.contains(Modifier::DIM));
        assert!(
            !buffer[(card.x, card.y + 4)]
                .modifier
                .contains(Modifier::DIM)
        );
        assert!(
            buffer[(card.x + 2, card.y + 4)]
                .modifier
                .contains(Modifier::REVERSED)
        );
        Ok(())
    }
}
