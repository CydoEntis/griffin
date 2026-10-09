//! The picker: a query over a list, filtered fuzzily as the query is typed, best
//! match first. The "cast" palette (design README §3) lists the project's files
//! then the commands, and its query's first character switches mode: `>` lists
//! only commands, `:` goes to a line, `/` searches the project's text. F5 lists
//! the `[[run]]` entries by name as a dialog.

use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::UnicodeWidthStr;

use crate::buffer::movement::Motion;
use crate::keymap::{Action, Input};
use crate::theme::Theme;
use crate::ui::prompt::{Outcome, PromptBar};
use crate::ui::{dialog_card, enter_mark, glow_row};

/// Widest the choices card gets, borders included.
const MAX_WIDTH: u16 = 80;

/// One entry of a list of choices, such as a run command: picked by its name,
/// listed with a detail and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub name: String,
    pub detail: String,
    pub source: &'static str,
}

/// A global action as the cast lists it: its name in plain words and the key
/// bound to it, if any.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub action: Action,
    pub title: &'static str,
    pub key: Option<String>,
}

/// Tallest the card for a list of choices gets (SPEC_V1_LAYOUT §9).
const CHOICES_HEIGHT: u16 = 8;
/// Where a choice's detail starts, from the card's left edge.
const DETAIL_X: u16 = 14;

/// The cast card's width, before clamping to the screen, and the row it drops
/// from (design README §3).
const CAST_WIDTH: u16 = 86;
const CAST_TOP: u16 = 4;
/// Most file rows the cast shows; the selection scrolls through the rest.
const CAST_ROWS: usize = 5;
/// Most command rows under the files: the design keeps them to a taste of what
/// `>` would list.
const CAST_MIXED_COMMANDS: usize = 2;
/// Most command rows once `>` lists only commands.
const CAST_COMMAND_ROWS: usize = 8;
/// The cast's rows other than its sections: the lit edge, a blank, the query and
/// the rule above them; a blank, the footer and a blank below.
const CAST_CHROME: u16 = 7;
/// Right of the query: the palette's name.
const CAST_NAME: &str = "cast";
/// The footer's prefixes, each with what it switches to, and the keys. `@`
/// (symbols) is left out until it does something.
const CAST_PREFIXES: [(&str, &str); 3] = [(">", "commands"), (":", "line"), ("/", "text")];
const CAST_KEYS: &str = "↑↓  ⏎  esc";

/// What a key did to the picker, when it did more than move or edit the query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picked {
    /// Enter on an item: for a file, its path relative to the project root,
    /// `/`-separated; for a list of choices, the choice as given.
    Open(String),
    /// Enter on a command: run its action.
    Run(Action),
    /// Enter on `:N`: move the cursor to line N, counting from 1; it's known to
    /// be in the buffer.
    Line(usize),
    /// Enter on `/text`: open project search for the text.
    Search(String),
    /// Esc: close the picker and do nothing.
    Close,
}

/// What the cast lists, from the query's first character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// No prefix: files, then a few commands.
    Files,
    /// `>`: commands only.
    Commands,
    /// `:`: a line to go to.
    Line,
    /// `/`: text to search the project for.
    Text,
}

/// What `:` holds, checked against the buffer's lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LineAsked {
    /// Nothing typed after `:` yet.
    Empty,
    /// A line in the buffer.
    Go(usize),
    /// A number past either end.
    OutOfRange,
    /// Not a number.
    NotANumber,
}

/// One file or command the query matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Match {
    /// Index into `files` or `commands`.
    item: usize,
    score: u32,
}

/// An entry of the list the arrows move through.
#[derive(Debug, Clone, Copy)]
enum Item<'a> {
    File(&'a str),
    Command(&'a Command),
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
    /// `None` for the cast palette.
    choices: Option<Vec<Choice>>,
    /// The files the query matches, best first.
    matches: Vec<Match>,
    /// What the cast lists under COMMANDS, in the order given.
    commands: Vec<Command>,
    /// The commands the query matches, best first.
    command_matches: Vec<Match>,
    /// How many lines the buffer `:` moves in has.
    lines: usize,
    /// Index into the files listed, then the commands listed after them.
    selected: usize,
    matcher: Matcher,
    /// Commands are matched as words, not paths.
    command_matcher: Matcher,
}

impl Picker {
    /// The cast palette over `commands`, for a buffer of `lines` lines, holding
    /// `query` (`:` for go to line), waiting for the file list.
    pub fn cast(commands: Vec<Command>, lines: usize, query: &str) -> Self {
        let mut picker = Picker {
            query: PromptBar::new("Go to file", query),
            files: None,
            no_match: "no matching files",
            choices: None,
            matches: Vec::new(),
            commands,
            command_matches: Vec::new(),
            lines,
            selected: 0,
            matcher: Matcher::new(Config::DEFAULT.match_paths()),
            command_matcher: Matcher::new(Config::DEFAULT),
        };
        picker.refilter();
        picker
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
            commands: Vec::new(),
            command_matches: Vec::new(),
            lines: 0,
            selected: 0,
            matcher: Matcher::new(Config::DEFAULT),
            command_matcher: Matcher::new(Config::DEFAULT),
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
    #[cfg(test)]
    pub fn matches(&self) -> impl Iterator<Item = &str> {
        let files = self.files.as_deref().unwrap_or_default();
        self.matches.iter().map(|m| files[m.item].as_str())
    }

    /// The selected file's path, or the selected command's name.
    #[cfg(test)]
    pub fn selected(&self) -> Option<&str> {
        match self.item(self.selected)? {
            Item::File(path) => Some(path),
            Item::Command(command) => Some(command.title),
        }
    }

    /// What the query's first character asks for; a list of choices has no
    /// modes.
    fn mode(&self) -> Mode {
        if self.choices.is_some() {
            return Mode::Files;
        }
        match self.query.text().chars().next() {
            Some('>') => Mode::Commands,
            Some(':') => Mode::Line,
            Some('/') => Mode::Text,
            _ => Mode::Files,
        }
    }

    /// The query without the prefix that picked the mode.
    fn needle(&self) -> &str {
        let text = self.query.text();
        if self.mode() == Mode::Files {
            return text;
        }
        let mut chars = text.chars();
        chars.next();
        chars.as_str()
    }

    /// How many files the arrows move through: none outside the files mode.
    fn files_listed(&self) -> usize {
        if self.mode() == Mode::Files {
            self.matches.len()
        } else {
            0
        }
    }

    /// How many commands the arrows move through, after the files.
    fn commands_listed(&self) -> usize {
        match self.mode() {
            Mode::Files => self.command_matches.len().min(CAST_MIXED_COMMANDS),
            Mode::Commands => self.command_matches.len(),
            Mode::Line | Mode::Text => 0,
        }
    }

    /// The `index`th entry the arrows move through: the files, then the
    /// commands.
    fn item(&self, index: usize) -> Option<Item<'_>> {
        let files = self.files_listed();
        if index < files {
            let file = self.matches[index].item;
            return self
                .files
                .as_ref()
                .and_then(|all| all.get(file))
                .map(|path| Item::File(path));
        }
        if index - files < self.commands_listed() {
            let command = self.command_matches[index - files].item;
            return self.commands.get(command).map(Item::Command);
        }
        None
    }

    /// The line `:` asks for, checked against the buffer's.
    fn line_asked(&self) -> LineAsked {
        let text = self.needle().trim();
        if text.is_empty() {
            return LineAsked::Empty;
        }
        match text.parse::<usize>() {
            Ok(line) if (1..=self.lines).contains(&line) => LineAsked::Go(line),
            Ok(_) => LineAsked::OutOfRange,
            // Too many digits for a usize is still a number, just too big.
            Err(_) if text.chars().all(|c| c.is_ascii_digit()) => LineAsked::OutOfRange,
            Err(_) => LineAsked::NotANumber,
        }
    }

    /// Up and Down move through the list, Enter acts on the selection or on
    /// what the prefix asks for, and Esc closes; everything else edits the
    /// query.
    pub fn handle(&mut self, input: Input) -> Option<Picked> {
        match input {
            Input::Action(Action::Move(Motion::Up)) => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            Input::Action(Action::Move(Motion::Down)) => {
                if self.selected + 1 < self.files_listed() + self.commands_listed() {
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

    /// Pasted text goes into the query as if typed; a line break is Enter.
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
            Outcome::Submit => self.submit(),
            Outcome::Cancel => Some(Picked::Close),
        }
    }

    /// Enter: the selected file or command, the line asked for when it's in
    /// the buffer, or the text to search for. A line out of range does nothing,
    /// so the cast stays open showing why.
    fn submit(&self) -> Option<Picked> {
        match self.mode() {
            Mode::Files | Mode::Commands => match self.item(self.selected)? {
                Item::File(path) => Some(Picked::Open(path.to_string())),
                Item::Command(command) => Some(Picked::Run(command.action)),
            },
            Mode::Line => match self.line_asked() {
                LineAsked::Go(line) => Some(Picked::Line(line)),
                LineAsked::Empty | LineAsked::OutOfRange | LineAsked::NotANumber => None,
            },
            Mode::Text => Some(Picked::Search(self.needle().to_string())),
        }
    }

    /// Scores every file and command against the query. An empty query lists
    /// them in the order given; otherwise the best score comes first, files
    /// tying going to the shorter path, as it's the more likely one to have been
    /// meant, and commands tying keeping their order.
    fn refilter(&mut self) {
        self.selected = 0;
        let mode = self.mode();
        let needle = self.needle().to_string();
        let pattern = Pattern::parse(&needle, CaseMatching::Smart, Normalization::Smart);
        let sorted = !needle.trim().is_empty();
        let mut chars = Vec::new();

        self.command_matches = if matches!(mode, Mode::Files | Mode::Commands) {
            let mut found: Vec<Match> = self
                .commands
                .iter()
                .enumerate()
                .filter_map(|(item, command)| {
                    let score = pattern.score(
                        Utf32Str::new(command.title, &mut chars),
                        &mut self.command_matcher,
                    )?;
                    Some(Match { item, score })
                })
                .collect();
            if sorted {
                // Stable, so ties keep the commands' order.
                found.sort_by_key(|m| std::cmp::Reverse(m.score));
            }
            found
        } else {
            Vec::new()
        };

        let Some(files) = &self.files else {
            return;
        };
        if mode != Mode::Files {
            self.matches.clear();
            return;
        }
        self.matches = files
            .iter()
            .enumerate()
            .filter_map(|(item, path)| {
                let score = pattern.score(Utf32Str::new(path, &mut chars), &mut self.matcher)?;
                Some(Match { item, score })
            })
            .collect();
        if sorted {
            self.matches.sort_by(|a, b| {
                b.score
                    .cmp(&a.score)
                    .then_with(|| files[a.item].len().cmp(&files[b.item].len()))
                    .then_with(|| files[a.item].cmp(&files[b.item]))
            });
        }
    }

    /// The char indices in `text` of the letters the query matched, sorted.
    fn matched(&self, text: &str, matcher: &mut Matcher, indices: &mut Vec<u32>) {
        let pattern = Pattern::parse(self.needle(), CaseMatching::Smart, Normalization::Smart);
        let mut chars = Vec::new();
        indices.clear();
        pattern.indices(Utf32Str::new(text, &mut chars), matcher, indices);
        indices.sort_unstable();
        indices.dedup();
    }

    /// Where the picker is drawn in `area`, so a click outside it can close it.
    pub fn card(&self, area: Rect) -> Rect {
        if self.choices.is_some() {
            Self::choices_card(area)
        } else {
            self.cast_card(area)
        }
    }

    /// Draws the picker over `area` and returns where the cursor goes, in the
    /// query.
    pub fn render(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        match &self.choices {
            Some(choices) => self.render_choices(theme, frame, area, choices),
            None => self.render_cast(theme, frame, area),
        }
    }

    /// The cast's sections, top to bottom, for the mode the query picks.
    fn cast_sections(&self) -> Vec<Section> {
        // One row for a message when there's nothing to list.
        let rows = |listed: usize, most: usize| listed.clamp(1, most);
        match self.mode() {
            Mode::Files => {
                let mut sections = vec![Section::Files(rows(self.matches.len(), CAST_ROWS))];
                let commands = self.commands_listed();
                if commands > 0 {
                    sections.push(Section::Commands(commands));
                }
                sections
            }
            Mode::Commands => vec![Section::Commands(rows(
                self.command_matches.len(),
                CAST_COMMAND_ROWS,
            ))],
            Mode::Line => vec![Section::Line],
            Mode::Text => vec![Section::Text],
        }
    }

    /// The cast card in `area`: 86 wide or the screen less four, centred, from
    /// row 4, as tall as its sections need (design README §3).
    fn cast_card(&self, area: Rect) -> Rect {
        let width = CAST_WIDTH.min(area.width.saturating_sub(4));
        let y = area.y + CAST_TOP.min(area.height);
        let height = (CAST_CHROME + sections_height(&self.cast_sections())).min(area.bottom() - y);
        Rect {
            x: area.x + (area.width - width) / 2,
            y,
            width,
            height,
        }
    }

    /// Dims `area` and draws the cast card over it: the lit edge, `✦` and the
    /// query with the palette's name and what it lists at the right, a rule,
    /// then each section under its header, the selected row the glow row with
    /// `⏎`; then the footer.
    fn render_cast(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        crate::ui::dim(theme, frame.buffer_mut(), area);
        let card = self.cast_card(area);
        dialog_card(theme, frame, card);
        let sections = self.cast_sections();
        if card.height < CAST_CHROME + sections_height(&sections) || card.width < 40 {
            return (card.x, card.y);
        }
        let out = frame.buffer_mut();
        let left = card.x + 4;
        let right = card.right() - 4;
        let muted = Style::new().fg(theme.muted);
        let mode = self.mode();

        let query_y = card.y + 2;
        let scope = match mode {
            Mode::Files => " · files · commands",
            Mode::Commands => " · commands",
            Mode::Line => " · line",
            Mode::Text => " · text",
        };
        let scope_x = right - width_of(scope);
        let name_x = scope_x - width_of(CAST_NAME);
        out.set_string(name_x, query_y, CAST_NAME, Style::new().fg(theme.accent));
        out.set_string(scope_x, query_y, scope, muted);
        out.set_string(
            left,
            query_y,
            "✦",
            Style::new().fg(theme.accent2).add_modifier(Modifier::BOLD),
        );
        // A line that can't be gone to shows in `err` where it's typed too.
        let bad_line = mode == Mode::Line
            && matches!(
                self.line_asked(),
                LineAsked::OutOfRange | LineAsked::NotANumber
            );
        let query_fg = if bad_line { theme.err } else { theme.strong };
        // Two blanks short of the palette's name, so a long query never runs
        // into it.
        let query = Rect::new(left + 2, query_y, name_x.saturating_sub(left + 4), 1);
        let cursor = self
            .query
            .render_value(out, query, Style::new().fg(query_fg));

        for x in card.x + 2..card.right() - 2 {
            out[(x, card.y + 3)].set_symbol("─").set_fg(theme.line2);
        }

        let row = Row {
            theme,
            card,
            left,
            right,
        };
        let mut y = card.y + 4;
        for (i, section) in sections.iter().enumerate() {
            if i > 0 {
                y += 1;
            }
            out.set_string(
                left,
                y,
                section.header(),
                muted.add_modifier(Modifier::BOLD),
            );
            y += 1;
            match *section {
                Section::Files(rows) => self.render_files(out, &row, y, rows),
                Section::Commands(rows) => self.render_commands(out, &row, y, rows),
                Section::Line => self.render_line(out, &row, y),
                Section::Text => self.render_text(out, &row, y),
            }
            y += section.rows();
        }

        let footer_y = y + 1;
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

    /// Up to `rows` files from row `y`, scrolled just far enough to keep a
    /// selected file on the last row.
    fn render_files(&self, out: &mut Buffer, row: &Row, y: u16, rows: usize) {
        let theme = row.theme;
        let muted = Style::new().fg(theme.muted);
        let Some(files) = &self.files else {
            out.set_string(row.left, y, "listing files…", muted);
            return;
        };
        if self.matches.is_empty() {
            out.set_string(row.left, y, self.no_match, muted);
            return;
        }
        let last = self.selected.min(self.matches.len() - 1);
        let first = last.saturating_sub(rows - 1);
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
            let path = files[m.item].as_str();
            self.matched(path, &mut matcher, &mut indices);
            let y = y + u16::try_from(line).unwrap_or(u16::MAX);
            let selected = index == self.selected;
            row.select(out, y, selected);
            // Two blanks short of `⏎`, so a long path never runs into it.
            let room = Rect::new(row.left, y, row.right.saturating_sub(row.left + 3), 1);
            cast_file(theme, out, room, path, &indices, selected);
        }
    }

    /// Up to `rows` commands from row `y`: the name with its matched letters
    /// lit and the key right-aligned in `muted`, scrolled just far enough to
    /// keep a selected command on the last row.
    fn render_commands(&self, out: &mut Buffer, row: &Row, y: u16, rows: usize) {
        let theme = row.theme;
        if self.command_matches.is_empty() {
            out.set_string(
                row.left,
                y,
                "no matching commands",
                Style::new().fg(theme.muted),
            );
            return;
        }
        let before = self.files_listed();
        let first = self
            .selected
            .checked_sub(before)
            .map_or(0, |at| at.saturating_sub(rows - 1));
        let mut matcher = self.command_matcher.clone();
        let mut indices = Vec::new();
        for (line, (index, m)) in self
            .command_matches
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .enumerate()
        {
            let Some(command) = self.commands.get(m.item) else {
                continue;
            };
            let y = y + u16::try_from(line).unwrap_or(u16::MAX);
            let selected = before + index == self.selected;
            row.select(out, y, selected);
            let plain = selected && !theme.ramps();
            let pick = |style: Style| if plain { Style::new() } else { style };
            // On the selected row the key steps left of `⏎`.
            let key_end = if selected {
                row.right.saturating_sub(3)
            } else {
                row.right
            };
            let mut text_end = key_end;
            if let Some(key) = &command.key {
                let key_x = key_end.saturating_sub(width_of(key));
                out.set_string(key_x, y, key, pick(Style::new().fg(theme.muted)));
                text_end = key_x.saturating_sub(2);
            }
            self.matched(command.title, &mut matcher, &mut indices);
            let hit = pick(Style::new().fg(theme.accent)).add_modifier(Modifier::BOLD);
            let rest = pick(Style::new().fg(if selected { theme.strong } else { theme.fg }));
            let mut x = row.left;
            for (at, ch) in command.title.chars().enumerate() {
                let cell = ch.to_string();
                let cells = width_of(&cell);
                if x + cells > text_end {
                    break;
                }
                let is_hit = u32::try_from(at).is_ok_and(|at| indices.binary_search(&at).is_ok());
                out.set_string(x, y, &cell, if is_hit { hit } else { rest });
                x += cells;
            }
        }
    }

    /// The row under `LINE`: a hint until a number is typed, then the line to go
    /// to as the selected row, or the value in `err` with why it can't be gone
    /// to.
    fn render_line(&self, out: &mut Buffer, row: &Row, y: u16) {
        let theme = row.theme;
        let muted = Style::new().fg(theme.muted);
        let range = format!("1–{}", self.lines);
        let room = usize::from(row.right.saturating_sub(row.left + 3));
        match self.line_asked() {
            LineAsked::Empty => {
                out.set_stringn(
                    row.left,
                    y,
                    format!("type a line number, {range}"),
                    room,
                    muted,
                );
            }
            LineAsked::Go(line) => {
                row.select(out, y, true);
                let plain = !theme.ramps();
                let pick = |style: Style| if plain { Style::new() } else { style };
                let (x, _) = out.set_stringn(
                    row.left,
                    y,
                    format!("Go to line {line}"),
                    room,
                    pick(Style::new().fg(theme.strong)),
                );
                let left = room.saturating_sub(usize::from(x - row.left));
                out.set_stringn(
                    x,
                    y,
                    format!("  of {}", self.lines),
                    left,
                    pick(Style::new().fg(theme.muted)),
                );
            }
            asked @ (LineAsked::OutOfRange | LineAsked::NotANumber) => {
                let value = self.needle().trim();
                let (x, _) = out.set_stringn(row.left, y, value, room, Style::new().fg(theme.err));
                let why = if asked == LineAsked::OutOfRange {
                    format!("  is out of range: lines {range}")
                } else {
                    format!("  is not a line number: lines {range}")
                };
                let left = room.saturating_sub(usize::from(x - row.left));
                out.set_stringn(x, y, why, left, muted);
            }
        }
    }

    /// The row under `TEXT`: a hint until something is typed, then the search
    /// Enter starts as the selected row.
    fn render_text(&self, out: &mut Buffer, row: &Row, y: u16) {
        let theme = row.theme;
        let room = usize::from(row.right.saturating_sub(row.left + 3));
        let text = self.needle();
        if text.is_empty() {
            out.set_stringn(
                row.left,
                y,
                "type text to search the project for",
                room,
                Style::new().fg(theme.muted),
            );
            return;
        }
        row.select(out, y, true);
        let plain = !theme.ramps();
        let pick = |style: Style| if plain { Style::new() } else { style };
        let (x, _) = out.set_stringn(
            row.left,
            y,
            "Search the project for ",
            room,
            pick(Style::new().fg(theme.strong)),
        );
        let left = room.saturating_sub(usize::from(x - row.left));
        out.set_stringn(
            x,
            y,
            text,
            left,
            pick(Style::new().fg(theme.accent)).add_modifier(Modifier::BOLD),
        );
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
            let Some(choice) = choices.get(m.item) else {
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

            self.matched(&choice.name, &mut matcher, &mut indices);
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
}

/// How many cells `text` takes.
fn width_of(text: &str) -> u16 {
    u16::try_from(text.width()).unwrap_or(u16::MAX)
}

/// One part of the cast, under its header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Section {
    /// This many file rows.
    Files(usize),
    /// This many command rows.
    Commands(usize),
    Line,
    Text,
}

impl Section {
    fn header(self) -> &'static str {
        match self {
            Section::Files(_) => "FILES",
            Section::Commands(_) => "COMMANDS",
            Section::Line => "LINE",
            Section::Text => "TEXT",
        }
    }

    fn rows(self) -> u16 {
        match self {
            Section::Files(rows) | Section::Commands(rows) => u16::try_from(rows).unwrap_or(1),
            Section::Line | Section::Text => 1,
        }
    }
}

/// How many rows `sections` take: each its header and its rows, with a blank
/// between two.
fn sections_height(sections: &[Section]) -> u16 {
    let rows: u16 = sections.iter().map(|s| 1 + s.rows()).sum();
    rows + u16::try_from(sections.len().saturating_sub(1)).unwrap_or(0)
}

/// Where a cast row's text goes: from `left` to `right`, inside `card`.
struct Row<'a> {
    theme: &'a Theme,
    card: Rect,
    left: u16,
    right: u16,
}

impl Row<'_> {
    /// When `selected`, lights row `y` as the one Enter acts on: the glow
    /// across the card and `⏎` at the right.
    fn select(&self, out: &mut Buffer, y: u16, selected: bool) {
        if selected {
            glow_row(self.theme, out, self.card.x, self.card.right(), y);
            enter_mark(self.theme, out, self.right, y);
        }
    }
}

/// One file row of the cast in `room`: the file's name, its matched letters
/// `accent` bold and the rest `fg` (`strong` when selected), then two blanks and
/// its folder in plain `muted`: the design lights only the name, so a query that
/// hits the folder (`src`, `util`) doesn't paint the folder of every row.
/// `matched` holds the char indices of the matched letters in `path`, sorted. On
/// the selected row in `mono`, which is reverse video, the only styling is bold.
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
    let dir = pick(Style::new().fg(theme.muted));

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
        if !put(&mut x, at, dir, dir) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::theme::mix;

    fn picker(files: &[&str]) -> Picker {
        let mut picker = Picker::cast(Vec::new(), 1, "");
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
        let mut p = Picker::cast(Vec::new(), 1, "");
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
                source: ".tome.toml",
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
        assert!(first.ends_with(".tome.toml  ⏎    "), "{first:?}");
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
    fn the_cast_card_drops_from_row_four_and_fits_its_rows() {
        let area = Rect::new(0, 0, 160, 45);
        let mut p = Picker::cast(Vec::new(), 1, "");
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
        let p = Picker::cast(Vec::new(), 1, "");
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
    fn letters_matched_in_the_folder_stay_muted() -> anyhow::Result<()> {
        let mut p = picker(FILES);
        type_query(&mut p, "util");
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
        assert!(line.starts_with("    helpers.rs  src/util"), "{line:?}");
        let folder = card.x + 16..card.x + 24;
        for x in folder {
            let cell = &buffer[(x, row)];
            assert_eq!(cell.fg, theme.muted, "{:?} at {x}", cell.symbol());
            assert!(!cell.modifier.contains(Modifier::BOLD));
        }
        Ok(())
    }

    fn commands() -> Vec<Command> {
        [
            (Action::Save, Some("Ctrl+S")),
            (Action::ToggleSplit, Some("Alt+V")),
            (Action::Run, Some("F5")),
            (Action::CycleFocus, None),
        ]
        .map(|(action, key)| Command {
            action,
            title: action.title(),
            key: key.map(str::to_string),
        })
        .to_vec()
    }

    fn cast(query: &str) -> Picker {
        let mut p = Picker::cast(commands(), 200, query);
        p.set_files(FILES.iter().map(|f| f.to_string()).collect());
        p
    }

    fn draw(p: &Picker, theme: &Theme) -> anyhow::Result<Buffer> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| {
            p.render(theme, frame, frame.area());
        })?;
        Ok(terminal.backend().buffer().clone())
    }

    /// The text of row `y` across `card`.
    fn card_row(buffer: &Buffer, card: Rect, y: u16) -> String {
        (card.x..card.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn with_no_prefix_two_commands_follow_the_files_and_enter_runs_one() {
        let mut p = cast("");
        // Five file rows, a blank, the header and two command rows.
        assert_eq!(p.card(Rect::new(0, 0, 160, 45)).height, 17);
        for _ in 0..FILES.len() {
            p.handle(Input::Action(Action::Move(Motion::Down)));
        }
        assert_eq!(p.selected(), Some("Save"));
        p.handle(Input::Action(Action::Move(Motion::Down)));
        assert_eq!(p.selected(), Some("Split right"));
        // Only two commands are listed, so the selection stops on the second.
        p.handle(Input::Action(Action::Move(Motion::Down)));
        assert_eq!(p.selected(), Some("Split right"));
        assert_eq!(
            p.handle(Input::Action(Action::Newline)),
            Some(Picked::Run(Action::ToggleSplit))
        );
    }

    #[test]
    fn a_command_row_shows_its_name_and_its_key_at_the_right() -> anyhow::Result<()> {
        let p = cast(">");
        let theme = Theme::default();
        let buffer = draw(&p, &theme)?;
        let card = p.card(Rect::new(0, 0, 100, 30));
        assert!(card_row(&buffer, card, card.y + 4).starts_with("    COMMANDS"));
        let first = card_row(&buffer, card, card.y + 5);
        assert!(first.starts_with("    Save "), "{first:?}");
        // The selected row's key steps left of `⏎`.
        assert!(first.ends_with("Ctrl+S  ⏎    "), "{first:?}");
        let second = card_row(&buffer, card, card.y + 6);
        assert!(second.starts_with("    Split right "), "{second:?}");
        assert!(second.ends_with(" Alt+V    "), "{second:?}");
        assert_eq!(buffer[(card.right() - 9, card.y + 6)].fg, theme.muted);
        // A command without a key shows none.
        let last = card_row(&buffer, card, card.y + 8);
        assert_eq!(last.trim_end(), "    Cycle focus");
        Ok(())
    }

    #[test]
    fn a_prefix_of_gt_lists_only_commands_matched_by_name() {
        let mut p = cast(">split");
        assert_eq!(p.matches().count(), 0);
        assert_eq!(p.selected(), Some("Split right"));
        assert_eq!(
            p.handle(Input::Action(Action::Newline)),
            Some(Picked::Run(Action::ToggleSplit))
        );
        type_query(&mut p, "zzz");
        assert_eq!(p.selected(), None);
        assert_eq!(p.handle(Input::Action(Action::Newline)), None);
    }

    #[test]
    fn a_colon_goes_to_a_line_in_the_buffer_and_only_there() {
        let mut p = cast(":");
        assert_eq!(p.handle(Input::Action(Action::Newline)), None);
        type_query(&mut p, "120");
        assert_eq!(
            p.handle(Input::Action(Action::Newline)),
            Some(Picked::Line(120))
        );
        for query in [":0", ":201", ":99999999999999999999999", ":abc"] {
            let mut p = cast(query);
            assert_eq!(p.handle(Input::Action(Action::Newline)), None, "{query}");
        }
        assert_eq!(
            cast(":200").handle(Input::Action(Action::Newline)),
            Some(Picked::Line(200))
        );
    }

    #[test]
    fn a_line_out_of_range_shows_its_value_in_err() -> anyhow::Result<()> {
        let p = cast(":999");
        let theme = Theme::default();
        let buffer = draw(&p, &theme)?;
        let card = p.card(Rect::new(0, 0, 100, 30));
        assert!(card_row(&buffer, card, card.y + 4).starts_with("    LINE"));
        let row = card_row(&buffer, card, card.y + 5);
        assert!(
            row.starts_with("    999  is out of range: lines 1–200"),
            "{row:?}"
        );
        assert_eq!(buffer[(card.x + 4, card.y + 5)].fg, theme.err);
        assert_eq!(buffer[(card.x + 9, card.y + 5)].fg, theme.muted);
        // Where it's typed too, after `✦ :`.
        assert_eq!(buffer[(card.x + 7, card.y + 2)].fg, theme.err);
        assert!(card_row(&buffer, card, card.y + 2).contains("cast · line"));

        let buffer = draw(&cast(":120"), &theme)?;
        let row = card_row(&buffer, card, card.y + 5);
        assert!(row.starts_with("    Go to line 120  of 200"), "{row:?}");
        assert_eq!(buffer[(card.x + 7, card.y + 2)].fg, theme.strong);
        Ok(())
    }

    #[test]
    fn a_slash_searches_the_project_for_the_rest() {
        let mut p = cast("/");
        type_query(&mut p, "foo");
        assert_eq!(
            p.handle(Input::Action(Action::Newline)),
            Some(Picked::Search("foo".into()))
        );
        // Deleting the prefix lists files again.
        let mut p = cast("/");
        assert_eq!(p.matches().count(), 0);
        p.handle(Input::Action(Action::Backspace));
        assert_eq!(p.matches().count(), FILES.len());
    }
}
