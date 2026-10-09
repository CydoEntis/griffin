//! The card `>keybindings` opens: every command with its keys and its name in
//! `[keys]`, searchable, in the catalog card's style (glyph-catalog spec C2).
//! Commands that only work in one place (the tree, the splash…) come after the
//! rest, under a heading naming that place.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::buffer::movement::Motion;
use crate::keymap::{Action, Input, Keymap, Scope};
use crate::theme::Theme;
use crate::ui::prompt::PromptBar;
use crate::ui::{dialog_card, dim, footer, glow_row};

/// The catalog's width and the row it drops from, so both cards read alike.
const WIDTH: u16 = 86;
const TOP: u16 = 4;
/// The rows other than the list: the lit edge, a blank, the header and the
/// rule above it; a blank and the footer below.
const CHROME: u16 = 6;
const HEADER: &str = "keybindings";
const FOOTER: &str = "⏎ rebind  ⌫ reset  esc close";
/// What a command with no key shows in place of its keys.
const NO_KEY: &str = "—";
/// Cells for the title, so the config names line up: the longest title and two
/// blanks.
const TITLE_WIDTH: u16 = 35;
/// The fewest cells a title keeps when the keys are wide (all of a shorter
/// title), so a row still says what it's for on a narrow screen.
const TITLE_MIN: u16 = 12;

/// The places with keys of their own, in the order the card lists them, and
/// the heading each is listed under.
const SCOPES: [(Scope, &str); 7] = [
    (Scope::Tree, "file tree"),
    (Scope::Find, "find bar"),
    (Scope::Search, "project search"),
    (Scope::Folders, "folder browser"),
    (Scope::Catalog, "language servers"),
    (Scope::Splash, "splash"),
    (Scope::Debug, "debug panel"),
];

/// One command's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub action: Action,
    /// What the cast palette calls it.
    pub title: &'static str,
    /// Its keys as the palette labels them, the palette's pick first; empty
    /// when it has none.
    pub keys: Vec<String>,
    /// Its name on the left of `[keys]`.
    pub name: &'static str,
    pub scope: Scope,
}

impl Row {
    /// The keys as the row shows them.
    pub fn keys_label(&self) -> String {
        if self.keys.is_empty() {
            NO_KEY.to_string()
        } else {
            self.keys.join(", ")
        }
    }

    /// Whether every word of `query` is in the title, a key or the config
    /// name, ignoring case.
    fn matches(&self, query: &str) -> bool {
        let hay = format!("{} {} {}", self.title, self.keys.join(" "), self.name).to_lowercase();
        query
            .to_lowercase()
            .split_whitespace()
            .all(|word| hay.contains(word))
    }
}

/// One line of the card's list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// The row at this index into `shown`.
    Row(usize),
    /// A scope's heading, as the catalog labels its debuggers.
    Heading(&'static str),
    Blank,
}

/// What a key did to the card, when it did more than move or filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Esc: close it.
    Close,
}

#[derive(Debug, Clone)]
pub struct Keybindings {
    /// Every command: those that work anywhere, then each scope's.
    rows: Vec<Row>,
    /// What's typed, filtering the rows.
    query: PromptBar,
    /// The indices into `rows` the query matches, in order.
    shown: Vec<usize>,
    /// Index into `shown`.
    selected: usize,
    /// The first line of the list in view. It moves only when the selection
    /// would leave the view, so ↑ moves the glow rather than the list.
    top: usize,
}

impl Keybindings {
    /// The card over every action, with its keys in `keymap`.
    pub fn new(keymap: &Keymap) -> Self {
        let row = |action: Action| Row {
            action,
            title: action.title(),
            keys: keymap.key_labels(action),
            name: action.name(),
            scope: action.scope(),
        };
        let mut rows: Vec<Row> = Action::all()
            .filter(|a| a.scope() == Scope::Global)
            .map(row)
            .collect();
        for (scope, _) in SCOPES {
            rows.extend(Action::all().filter(|a| a.scope() == scope).map(row));
        }
        let shown = (0..rows.len()).collect();
        Keybindings {
            rows,
            query: PromptBar::new("Keybindings", ""),
            shown,
            selected: 0,
            top: 0,
        }
    }

    #[cfg(test)]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// The command the selected row is for, if any row is shown: what #170
    /// rebinds or resets.
    #[cfg(test)]
    pub fn selected_action(&self) -> Option<Action> {
        let &row = self.shown.get(self.selected)?;
        Some(self.rows[row].action)
    }

    /// ↑ and ↓ move the selection, stopping at either end, and scroll the
    /// list as the card drawn in `area` needs; Esc closes; Enter does nothing
    /// yet; anything else edits the query.
    pub fn handle(&mut self, input: Input, area: Rect) -> Option<Step> {
        match input {
            Input::Action(Action::Move(Motion::Up)) => {
                self.selected = self.selected.saturating_sub(1);
                self.top = self.first_line(self.room(area));
            }
            Input::Action(Action::Move(Motion::Down)) => {
                self.selected = (self.selected + 1).min(self.shown.len().saturating_sub(1));
                self.top = self.first_line(self.room(area));
            }
            Input::Action(Action::Cancel) => return Some(Step::Close),
            // Rebinding the selected row (#170) goes here.
            Input::Action(Action::Newline) => {}
            // With nothing typed, Backspace is the selected row's reset
            // (#170); with a query it has to stay able to correct a typo.
            Input::Action(Action::Backspace) if self.query.text().is_empty() => {}
            _ => {
                let before = self.query.text().to_string();
                self.query.handle(input);
                if self.query.text() != before {
                    self.refilter();
                }
            }
        }
        None
    }

    /// Pasted text goes into the query as if typed; a line break does nothing.
    pub fn paste(&mut self, text: &str) {
        let line: String = text.chars().filter(|c| !matches!(c, '\r' | '\n')).collect();
        let before = self.query.text().to_string();
        self.query.paste(&line);
        if self.query.text() != before {
            self.refilter();
        }
    }

    fn refilter(&mut self) {
        let query = self.query.text();
        self.shown = (0..self.rows.len())
            .filter(|&i| self.rows[i].matches(query))
            .collect();
        self.selected = 0;
        self.top = 0;
    }

    /// How many lines of the list the card drawn in `area` shows.
    fn room(&self, area: Rect) -> usize {
        usize::from(self.card(area).height.saturating_sub(CHROME))
    }

    /// The first line to draw in a list `room` lines tall: `top`, moved just
    /// far enough to bring the selected row into view, along with the heading
    /// and blank above it when scrolling up to it.
    fn first_line(&self, room: usize) -> usize {
        let lines = self.lines();
        let at = lines
            .iter()
            .position(|&line| line == Line::Row(self.selected))
            .unwrap_or(0);
        let lead = lines[..at]
            .iter()
            .rposition(|line| matches!(line, Line::Row(_)))
            .map_or(0, |row| row + 1);
        let top = if at < self.top { lead } else { self.top };
        top.max((at + 1).saturating_sub(room.max(1)))
    }

    /// The list as drawn: the rows shown, each scope's after a blank and its
    /// heading.
    fn lines(&self) -> Vec<Line> {
        let mut lines = Vec::new();
        let mut scope = Scope::Global;
        for (at, &row) in self.shown.iter().enumerate() {
            let row_scope = self.rows[row].scope;
            if row_scope != scope {
                scope = row_scope;
                if let Some(&(_, heading)) = SCOPES.iter().find(|(s, _)| *s == scope) {
                    if !lines.is_empty() {
                        lines.push(Line::Blank);
                    }
                    lines.push(Line::Heading(heading));
                }
            }
            lines.push(Line::Row(at));
        }
        lines
    }

    /// Where the card goes in `area`: the catalog's width, centred, from row
    /// 4, as tall as every row needs or down to a row short of the bottom. Its
    /// height doesn't follow the query, so the card holds still while typing.
    pub fn card(&self, area: Rect) -> Rect {
        let width = WIDTH.min(area.width.saturating_sub(4));
        let y = area.y + TOP.min(area.height);
        // Every row, a heading for each scope and a blank before it.
        let all = self.rows.len() + 2 * SCOPES.len();
        let height = CHROME
            .saturating_add(u16::try_from(all).unwrap_or(u16::MAX))
            .min(area.bottom().saturating_sub(y + 1));
        Rect {
            x: area.x + (area.width - width) / 2,
            y,
            width,
            height,
        }
    }

    /// Dims `area` and draws the card over it: the lit edge; `✦ keybindings`
    /// and the query; a rule; the list, scrolled just far enough to keep the
    /// selected row, on the glow row, in view; the footer. Returns where the
    /// cursor goes, in the query.
    pub fn render(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        dim(theme, frame.buffer_mut(), area);
        let card = self.card(area);
        dialog_card(theme, frame, card);
        if card.height <= CHROME || card.width < 40 {
            return (card.x, card.y);
        }
        let out = frame.buffer_mut();
        let left = card.x + 4;
        let right = card.right() - 4;

        let header_y = card.y + 2;
        out.set_string(
            left,
            header_y,
            "✦",
            Style::new().fg(theme.accent2).add_modifier(Modifier::BOLD),
        );
        out.set_string(left + 2, header_y, HEADER, Style::new().fg(theme.strong));
        let query_x = left + 2 + width_of(HEADER) + 2;
        let query = Rect::new(query_x, header_y, right.saturating_sub(query_x), 1);
        let cursor = self
            .query
            .render_value(out, query, Style::new().fg(theme.strong));

        for x in card.x + 2..card.right() - 2 {
            out[(x, card.y + 3)].set_symbol("─").set_fg(theme.line2);
        }

        let room = usize::from(card.height - CHROME);
        let top = card.y + 4;
        let lines = self.lines();
        if lines.is_empty() {
            out.set_string(
                left,
                top,
                "no matching commands",
                Style::new().fg(theme.muted),
            );
        }
        // `top` was set for this screen; clamping again keeps the selection
        // in view after a resize.
        let first = self.first_line(room);
        for (offset, line) in lines.iter().skip(first).take(room).enumerate() {
            let y = top + u16::try_from(offset).unwrap_or(u16::MAX);
            match *line {
                Line::Row(i) => {
                    let row = &self.rows[self.shown[i]];
                    render_row(theme, out, card, y, row, i == self.selected);
                }
                Line::Heading(heading) => {
                    out.set_string(
                        left,
                        y,
                        heading,
                        Style::new().fg(theme.muted).add_modifier(Modifier::BOLD),
                    );
                }
                Line::Blank => {}
            }
        }

        footer(theme, out, card, FOOTER);
        cursor
    }
}

/// The title, the config name in `muted`, and the keys at the right as the
/// palette shows them. A selected row is on the glow row, its title in
/// `strong`; in `mono` it carries no colours, so the reverse video reads.
fn render_row(theme: &Theme, out: &mut Buffer, card: Rect, y: u16, row: &Row, selected: bool) {
    let left = card.x + 4;
    let right = card.right() - 4;
    if selected {
        glow_row(theme, out, card.x, card.right(), y);
    }
    let plain = selected && !theme.ramps();
    let pick = |style: Style| if plain { Style::new() } else { style };

    // The keys get what the title's minimum and two blanks leave, cut with
    // `…` beyond that.
    let title_min = width_of(row.title).min(TITLE_MIN);
    let keys_room = right.saturating_sub(left + title_min + 2);
    let keys = cut(&row.keys_label(), keys_room);
    let keys_x = right.saturating_sub(width_of(&keys));
    out.set_string(keys_x, y, &keys, pick(Style::new().fg(theme.text)));

    let title = pick(Style::new().fg(if selected { theme.strong } else { theme.fg }));
    out.set_stringn(
        left,
        y,
        row.title,
        usize::from(keys_x.saturating_sub(left + 2)),
        title,
    );
    // Two blanks short of the keys, so a long name never runs into them.
    let name_x = left + TITLE_WIDTH;
    if name_x + 2 < keys_x {
        out.set_stringn(
            name_x,
            y,
            row.name,
            usize::from(keys_x - name_x - 2),
            pick(Style::new().fg(theme.muted)),
        );
    }
}

/// How many cells `text` takes.
fn width_of(text: &str) -> u16 {
    u16::try_from(text.width()).unwrap_or(u16::MAX)
}

/// `text` if it fits in `room` cells, else as much as fits followed by `…`.
fn cut(text: &str, room: u16) -> String {
    if width_of(text) <= room {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = u16::try_from(c.width().unwrap_or(0)).unwrap_or(u16::MAX);
        if used + w + 1 > room {
            break;
        }
        out.push(c);
        used += w;
    }
    if room > 0 {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::config::{KeyBinding, KeysConfig};
    use crate::theme::mix;

    fn fresh() -> Keybindings {
        Keybindings::new(&Keymap::default())
    }

    fn row(k: &Keybindings, action: Action) -> &Row {
        k.rows()
            .iter()
            .find(|r| r.action == action)
            .unwrap_or_else(|| panic!("no row for {action:?}"))
    }

    /// The screen the unit tests draw on.
    const SCREEN: Rect = Rect::new(0, 0, 100, 30);

    fn type_text(k: &mut Keybindings, text: &str) {
        for c in text.chars() {
            k.handle(Input::Text(c), SCREEN);
        }
    }

    fn shown(k: &Keybindings) -> Vec<&'static str> {
        k.shown.iter().map(|&i| k.rows[i].name).collect()
    }

    #[test]
    fn every_action_has_a_row_with_title_keys_and_config_name() {
        let k = fresh();
        assert_eq!(k.rows().len(), Action::all().count());
        let save = row(&k, Action::Save);
        assert_eq!(
            (save.title, save.keys_label().as_str(), save.name),
            ("Save", "Ctrl+S", "save")
        );
        let keys = row(&k, Action::Keybindings);
        assert_eq!(keys.keys_label(), "—");
        assert_eq!(keys.name, "keybindings");
        let tree = row(&k, Action::TreeNewFolder);
        assert_eq!(
            (tree.keys_label().as_str(), tree.scope),
            ("Shift+A", Scope::Tree)
        );
    }

    #[test]
    fn a_rebound_command_shows_every_key_it_has() -> anyhow::Result<()> {
        let keys: KeysConfig = [(
            "save".to_string(),
            KeyBinding::Many(vec!["ctrl+s".into(), "alt+w".into()]),
        )]
        .into_iter()
        .collect();
        let k = Keybindings::new(&Keymap::new(&keys)?);
        assert_eq!(row(&k, Action::Save).keys_label(), "Alt+W, Ctrl+S");
        Ok(())
    }

    #[test]
    fn global_commands_come_first_then_each_scope_under_its_heading() {
        let k = fresh();
        let first_scoped = k
            .rows()
            .iter()
            .position(|r| r.scope != Scope::Global)
            .unwrap_or(0);
        assert!(
            k.rows()[first_scoped..]
                .iter()
                .all(|r| r.scope != Scope::Global)
        );
        assert_eq!(k.rows()[0].action, Action::Quit);
        let lines = k.lines();
        let headings: Vec<&str> = lines
            .iter()
            .filter_map(|l| match l {
                Line::Heading(h) => Some(*h),
                _ => None,
            })
            .collect();
        assert_eq!(
            headings,
            [
                "file tree",
                "find bar",
                "project search",
                "folder browser",
                "language servers",
                "splash",
                "debug panel"
            ]
        );
        // A blank before each heading, and each scope's rows under it.
        let tree = lines
            .iter()
            .position(|l| *l == Line::Heading("file tree"))
            .unwrap_or(0);
        assert_eq!(lines[tree - 1], Line::Blank);
        let Line::Row(i) = lines[tree + 1] else {
            panic!("no row under the tree heading");
        };
        assert_eq!(k.rows[k.shown[i]].action, Action::TreeNewFile);
    }

    #[test]
    fn typing_filters_on_title_key_or_config_name() {
        let mut k = fresh();
        type_text(&mut k, "save as");
        assert_eq!(shown(&k), ["save_as"]);
        let mut k = fresh();
        type_text(&mut k, "ctrl+shift+left");
        assert_eq!(shown(&k), ["select_word_left"]);
        let mut k = fresh();
        type_text(&mut k, "SPLASH_Q");
        assert_eq!(shown(&k), ["splash_quit"]);
        // Backspace corrects the query; the selection starts over.
        k.handle(Input::Action(Action::Backspace), SCREEN);
        k.handle(Input::Action(Action::Backspace), SCREEN);
        assert!(shown(&k).len() > 1);
        assert_eq!(k.selected, 0);
        let mut k = fresh();
        type_text(&mut k, "zzz");
        assert!(k.lines().is_empty());
        assert_eq!(k.selected_action(), None);
    }

    #[test]
    fn arrows_stop_at_the_ends_enter_and_backspace_do_nothing_and_esc_closes() {
        let mut k = fresh();
        let press = |k: &mut Keybindings, a: Action| k.handle(Input::Action(a), SCREEN);
        assert_eq!(press(&mut k, Action::Move(Motion::Up)), None);
        assert_eq!(k.selected_action(), Some(Action::Quit));
        press(&mut k, Action::Move(Motion::Down));
        assert_eq!(k.selected_action(), Some(Action::Save));
        for _ in 0..500 {
            press(&mut k, Action::Move(Motion::Down));
        }
        assert_eq!(k.selected_action(), Some(Action::DebugSwitchPane));
        assert_eq!(press(&mut k, Action::Newline), None);
        assert_eq!(press(&mut k, Action::Backspace), None);
        assert_eq!(k.selected_action(), Some(Action::DebugSwitchPane));
        assert_eq!(k.query.text(), "");
        assert_eq!(press(&mut k, Action::Cancel), Some(Step::Close));
    }

    #[test]
    fn a_paste_goes_into_the_query_without_its_line_break() {
        let mut k = fresh();
        k.paste("toggle tree\n");
        assert_eq!(k.query.text(), "toggle tree");
        assert_eq!(shown(&k), ["toggle_tree"]);
    }

    fn draw(k: &Keybindings, theme: &Theme) -> anyhow::Result<Buffer> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| {
            k.render(theme, frame, frame.area());
        })?;
        Ok(terminal.backend().buffer().clone())
    }

    fn card_row(buffer: &Buffer, card: Rect, y: u16) -> String {
        (card.x..card.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn the_card_is_the_catalogs_with_header_rule_rows_and_footer() -> anyhow::Result<()> {
        let k = fresh();
        let theme = Theme::default();
        let card = k.card(Rect::new(0, 0, 100, 30));
        assert_eq!(card, Rect::new(7, 4, 86, 25));
        let buffer = draw(&k, &theme)?;
        assert_eq!(buffer[(card.x, card.y)].symbol(), "▀");
        let header = card_row(&buffer, card, card.y + 2);
        assert!(header.starts_with("    ✦ keybindings "), "{header:?}");
        assert_eq!(buffer[(card.x + 4, card.y + 2)].fg, theme.accent2);
        assert_eq!(buffer[(card.x + 5, card.y + 3)].symbol(), "─");

        let quit = card_row(&buffer, card, card.y + 4);
        assert_eq!(quit, format!("    {:<35}{:<37}Ctrl+Q    ", "Quit", "quit"));
        // The first row is selected: on the glow, its title in `strong`.
        assert_eq!(
            buffer[(card.x, card.y + 4)].bg,
            mix(theme.raised, theme.accent, 0.3)
        );
        assert_eq!(buffer[(card.x + 4, card.y + 4)].fg, theme.strong);
        assert_eq!(buffer[(card.x + 39, card.y + 4)].fg, theme.muted);
        let save = card_row(&buffer, card, card.y + 5);
        assert!(save.starts_with("    Save "), "{save:?}");
        assert_eq!(buffer[(card.x, card.y + 5)].bg, theme.raised);
        assert_eq!(buffer[(card.x + 4, card.y + 5)].fg, theme.fg);

        assert_eq!(card_row(&buffer, card, card.bottom() - 2).trim(), "");
        let foot = card_row(&buffer, card, card.bottom() - 1);
        assert!(foot.starts_with(&format!("  {FOOTER}")), "{foot:?}");
        Ok(())
    }

    #[test]
    fn a_scope_heading_is_muted_and_bold_and_a_keyless_row_shows_a_dash() -> anyhow::Result<()> {
        let mut k = fresh();
        type_text(&mut k, "tree");
        let theme = Theme::default();
        let card = k.card(Rect::new(0, 0, 100, 30));
        let buffer = draw(&k, &theme)?;
        // Toggle and focus first, then the tree's own under its heading.
        assert!(card_row(&buffer, card, card.y + 4).starts_with("    Toggle file tree"));
        assert!(card_row(&buffer, card, card.y + 5).starts_with("    Focus file tree"));
        assert_eq!(card_row(&buffer, card, card.y + 6).trim(), "");
        let heading = card_row(&buffer, card, card.y + 7);
        assert_eq!(heading.trim_end(), "    file tree", "{heading:?}");
        assert_eq!(buffer[(card.x + 4, card.y + 7)].fg, theme.muted);
        assert!(
            buffer[(card.x + 4, card.y + 7)]
                .modifier
                .contains(Modifier::BOLD)
        );
        assert!(card_row(&buffer, card, card.y + 8).starts_with("    New file in tree"));

        let mut k = fresh();
        type_text(&mut k, "keybindings");
        let buffer = draw(&k, &theme)?;
        let row = card_row(&buffer, card, card.y + 4);
        assert_eq!(
            row,
            format!("    {:<35}{:<42}—    ", "Keybindings", "keybindings")
        );
        Ok(())
    }

    #[test]
    fn the_list_scrolls_to_keep_the_selected_row_in_view() -> anyhow::Result<()> {
        let mut k = fresh();
        for _ in 0..500 {
            k.handle(Input::Action(Action::Move(Motion::Down)), SCREEN);
        }
        let theme = Theme::default();
        let card = k.card(Rect::new(0, 0, 100, 30));
        let buffer = draw(&k, &theme)?;
        let last = card_row(&buffer, card, card.bottom() - 3);
        assert!(
            last.starts_with("    Debug panel: stack or variables"),
            "{last:?}"
        );
        assert_eq!(
            buffer[(card.x, card.bottom() - 3)].bg,
            mix(theme.raised, theme.accent, 0.3)
        );
        Ok(())
    }

    #[test]
    fn up_moves_the_selection_within_the_view_without_scrolling() -> anyhow::Result<()> {
        let mut k = fresh();
        let theme = Theme::default();
        let card = k.card(SCREEN);
        for _ in 0..25 {
            k.handle(Input::Action(Action::Move(Motion::Down)), SCREEN);
        }
        let scrolled = card_row(&draw(&k, &theme)?, card, card.y + 4);
        let bottom = card.bottom() - 3;
        let lit = mix(theme.raised, theme.accent, 0.3);
        assert_eq!(draw(&k, &theme)?[(card.x, bottom)].bg, lit);
        for _ in 0..3 {
            k.handle(Input::Action(Action::Move(Motion::Up)), SCREEN);
        }
        let buffer = draw(&k, &theme)?;
        assert_eq!(card_row(&buffer, card, card.y + 4), scrolled);
        assert_eq!(buffer[(card.x, bottom - 3)].bg, lit);
        assert_eq!(buffer[(card.x, bottom)].bg, theme.raised);
        // Going on up past the top scrolls back a line at a time.
        for _ in 0..22 {
            k.handle(Input::Action(Action::Move(Motion::Up)), SCREEN);
        }
        let buffer = draw(&k, &theme)?;
        assert!(card_row(&buffer, card, card.y + 4).starts_with("    Quit"));
        Ok(())
    }

    #[test]
    fn wide_keys_are_cut_with_an_ellipsis_and_the_title_keeps_its_room() -> anyhow::Result<()> {
        let keys: KeysConfig = [(
            "doc_start".to_string(),
            KeyBinding::Many(vec![
                "ctrl+shift+alt+pageup".into(),
                "ctrl+alt+shift+home".into(),
                "ctrl+alt+f12".into(),
            ]),
        )]
        .into_iter()
        .collect();
        let mut k = Keybindings::new(&Keymap::new(&keys)?);
        type_text(&mut k, "doc_start");
        let screen = Rect::new(0, 0, 45, 30);
        let card = k.card(screen);
        assert_eq!(card.width, 41);
        let mut terminal = Terminal::new(TestBackend::new(45, 30))?;
        terminal.draw(|frame| {
            k.render(&Theme::default(), frame, frame.area());
        })?;
        let row = card_row(terminal.backend().buffer(), card, card.y + 4);
        // Twelve cells of the title, two blanks, then the keys cut to fit.
        assert_eq!(row, "    Go to start   Ctrl+Alt+F12, Ctrl…    ");
        assert!(row.ends_with("…    "), "{row:?}");
        assert_eq!(cut("abcdef", 4), "abc…");
        assert_eq!(cut("abc", 4), "abc");
        Ok(())
    }

    #[test]
    fn mono_reverses_the_selected_row() -> anyhow::Result<()> {
        let mut k = fresh();
        let mono = Theme::named("mono").expect("mono exists");
        let card = k.card(Rect::new(0, 0, 100, 30));
        let buffer = draw(&k, &mono)?;
        for x in [card.x, card.x + 4, card.right() - 6, card.right() - 1] {
            assert!(
                buffer[(x, card.y + 4)]
                    .modifier
                    .contains(Modifier::REVERSED),
                "{x}"
            );
        }
        k.handle(Input::Action(Action::Move(Motion::Down)), SCREEN);
        let buffer = draw(&k, &mono)?;
        assert!(
            buffer[(card.x + 4, card.y + 5)]
                .modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            !buffer[(card.x + 4, card.y + 4)]
                .modifier
                .contains(Modifier::REVERSED)
        );
        Ok(())
    }
}
