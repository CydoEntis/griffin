//! The folder browser `>open directory` opens (glyph-splash spec S6): a card in
//! the cast's style listing one folder's sub-folders, to walk the disk and pick
//! a folder to open as the project.

use std::io;
use std::path::{Component, Path, PathBuf};

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
use crate::ui::{dialog_card, dim, enter_mark, glow_row};

/// The card's width before clamping to the screen, and the row it drops from:
/// the cast palette's, so the browser reads as the palette turning into it.
const WIDTH: u16 = 86;
const TOP: u16 = 4;
/// Most folder rows shown; the selection scrolls through the rest.
const FOLDER_ROWS: usize = 10;
/// The rows other than the folders: the lit edge, a blank, the header, the
/// rule and the open row above them; a blank, the footer and a blank below.
const CHROME: u16 = 8;
/// Right of the header: what Enter does here and what's listed.
const SCOPE: &str = "open · folders";
const FOOTER: &str = "⏎ into  ← up  ctrl+⏎ open here  esc cancel";

/// What a key did to the browser, when it did more than move or filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Browsed {
    /// Open this folder as the project.
    Open(PathBuf),
    /// A folder couldn't be read or a typed path names none; the browser stays
    /// where it was and the status line says this.
    Failed(String),
    /// Esc: close and change nothing.
    Close,
}

#[derive(Debug, Clone)]
pub struct DirPicker {
    /// The folder shown, absolute.
    dir: PathBuf,
    /// Its sub-folders' names, in `list_folders` order.
    folders: Vec<String>,
    query: PromptBar,
    /// Indices into `folders` the query matches, best first.
    matches: Vec<usize>,
    /// 0 is the open row; `n` is `matches[n - 1]`.
    selected: usize,
    matcher: Matcher,
}

impl DirPicker {
    /// The browser on `dir`, or why `dir` can't be listed.
    pub fn new(dir: &Path) -> Result<Self, String> {
        let dir = clean(dir);
        let folders = list_folders(&dir).map_err(|err| cannot_open(&dir, &err))?;
        let mut picker = DirPicker {
            dir,
            folders,
            query: PromptBar::new("Open directory", ""),
            matches: Vec::new(),
            selected: 0,
            matcher: Matcher::new(Config::DEFAULT),
        };
        picker.refilter();
        Ok(picker)
    }

    /// The folder shown.
    #[cfg(test)]
    fn dir(&self) -> &Path {
        &self.dir
    }

    /// The folder names listed, best match first.
    #[cfg(test)]
    fn listed(&self) -> Vec<&str> {
        self.matches
            .iter()
            .map(|&i| self.folders[i].as_str())
            .collect()
    }

    /// Up and Down move through the open row and the folders; Enter acts on
    /// the selection or jumps to a typed path; ← or Backspace with nothing typed
    /// go up; Esc closes; everything else edits the query.
    pub fn handle(&mut self, input: Input) -> Option<Browsed> {
        let empty = self.query.text().is_empty();
        match input {
            Input::Action(Action::Move(Motion::Up)) => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            Input::Action(Action::Move(Motion::Down)) => {
                self.selected = (self.selected + 1).min(self.matches.len());
                None
            }
            Input::Action(Action::Move(Motion::Left) | Action::Backspace) if empty => self.up(),
            Input::Action(Action::OpenFolderHere) => Some(Browsed::Open(self.dir.clone())),
            _ => {
                let before = self.query.text().to_string();
                let outcome = self.query.handle(input);
                self.after_edit(&before, outcome)
            }
        }
    }

    /// Pasted text goes into the query as if typed; a line break is Enter.
    pub fn paste(&mut self, text: &str) -> Option<Browsed> {
        let before = self.query.text().to_string();
        let outcome = self.query.paste(text);
        self.after_edit(&before, outcome)
    }

    fn after_edit(&mut self, before: &str, outcome: Option<Outcome>) -> Option<Browsed> {
        if self.query.text() != before {
            self.refilter();
        }
        match outcome? {
            Outcome::Submit => self.submit(),
            Outcome::Cancel => Some(Browsed::Close),
        }
    }

    /// Enter: a typed path jumps there; otherwise the open row opens the folder
    /// shown and a folder row goes into it.
    fn submit(&mut self) -> Option<Browsed> {
        let typed = self.query.text().trim().to_string();
        if looks_like_path(&typed) {
            let target = clean(&self.dir.join(&typed));
            if !target.is_dir() {
                return Some(Browsed::Failed(format!("no such folder: {typed}")));
            }
            return self.go(target, None);
        }
        match self.selected {
            0 => Some(Browsed::Open(self.dir.clone())),
            n => {
                let name = self.folders.get(*self.matches.get(n - 1)?)?.clone();
                let target = self.dir.join(name);
                self.go(target, None)
            }
        }
    }

    /// Moves to the parent folder, selecting the one just left; nothing above
    /// a drive or `/`.
    fn up(&mut self) -> Option<Browsed> {
        let parent = self.dir.parent()?.to_path_buf();
        let came_from = self
            .dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned());
        self.go(parent, came_from)
    }

    /// Shows `dir` with an empty query and `select` (a folder name) selected,
    /// or the open row; a folder that can't be read leaves everything as it was.
    fn go(&mut self, dir: PathBuf, select: Option<String>) -> Option<Browsed> {
        match list_folders(&dir) {
            Ok(folders) => {
                self.dir = dir;
                self.folders = folders;
                self.query = PromptBar::new(self.query.label, "");
                self.refilter();
                if let Some(name) = select
                    && let Some(at) = self.matches.iter().position(|&i| self.folders[i] == name)
                {
                    self.selected = at + 1;
                }
                None
            }
            Err(err) => Some(Browsed::Failed(cannot_open(&dir, &err))),
        }
    }

    /// Scores every folder against the query. Nothing typed lists them all in
    /// order with the open row selected; otherwise the best match comes first,
    /// ties keeping their order, and is selected, as Enter is most likely meant
    /// to go into it.
    fn refilter(&mut self) {
        let needle = self.query.text().to_string();
        if needle.trim().is_empty() {
            self.matches = (0..self.folders.len()).collect();
            self.selected = 0;
            return;
        }
        let pattern = Pattern::parse(&needle, CaseMatching::Smart, Normalization::Smart);
        let mut chars = Vec::new();
        let mut found: Vec<(usize, u32)> = self
            .folders
            .iter()
            .enumerate()
            .filter_map(|(i, name)| {
                let score = pattern.score(Utf32Str::new(name, &mut chars), &mut self.matcher)?;
                Some((i, score))
            })
            .collect();
        // Stable, so ties keep the listing's order.
        found.sort_by_key(|&(_, score)| std::cmp::Reverse(score));
        self.matches = found.into_iter().map(|(i, _)| i).collect();
        self.selected = usize::from(!self.matches.is_empty());
    }

    /// The char indices in `name` of the letters the query matched, sorted.
    fn matched(&self, name: &str, matcher: &mut Matcher, indices: &mut Vec<u32>) {
        indices.clear();
        let needle = self.query.text();
        if needle.trim().is_empty() {
            return;
        }
        let pattern = Pattern::parse(needle, CaseMatching::Smart, Normalization::Smart);
        let mut chars = Vec::new();
        pattern.indices(Utf32Str::new(name, &mut chars), matcher, indices);
        indices.sort_unstable();
        indices.dedup();
    }

    /// How many folder rows the card has room for: one for a message when
    /// there's nothing to list.
    fn rows(&self) -> usize {
        self.matches.len().clamp(1, FOLDER_ROWS)
    }

    /// Where the card goes in `area`: the cast's width, centred, from row 4, as
    /// tall as its rows need.
    pub fn card(&self, area: Rect) -> Rect {
        let width = WIDTH.min(area.width.saturating_sub(4));
        let y = area.y + TOP.min(area.height);
        let rows = u16::try_from(self.rows()).unwrap_or(1);
        let height = (CHROME + rows).min(area.bottom() - y);
        Rect {
            x: area.x + (area.width - width) / 2,
            y,
            width,
            height,
        }
    }

    /// Dims `area` and draws the card over it: the lit edge; `✦`, the folder
    /// and the query, with `open · folders` at the right; a rule; the open row;
    /// the folders, the selected one on the glow row with `⏎`; the footer.
    /// Returns where the cursor goes.
    pub fn render(&self, theme: &Theme, frame: &mut Frame, area: Rect) -> (u16, u16) {
        dim(theme, frame.buffer_mut(), area);
        let card = self.card(area);
        dialog_card(theme, frame, card);
        let rows = self.rows();
        let full = CHROME + u16::try_from(rows).unwrap_or(1);
        if card.height < full || card.width < 40 {
            return (card.x, card.y);
        }
        let out = frame.buffer_mut();
        let left = card.x + 4;
        let right = card.right() - 4;
        let muted = Style::new().fg(theme.muted);

        let header_y = card.y + 2;
        let scope_x = right - width_of(SCOPE);
        out.set_string(scope_x, header_y, SCOPE, muted);
        out.set_string(
            left,
            header_y,
            "✦",
            Style::new().fg(theme.accent2).add_modifier(Modifier::BOLD),
        );
        // Two blanks short of the scope, so a long path never runs into it.
        let field = Rect::new(left + 2, header_y, scope_x.saturating_sub(left + 4), 1);
        let cursor = self.render_header(theme, out, field);

        for x in card.x + 2..card.right() - 2 {
            out[(x, card.y + 3)].set_symbol("─").set_fg(theme.line2);
        }

        let open_y = card.y + 4;
        self.render_open_row(theme, out, card, open_y);
        self.render_folders(theme, out, card, open_y + 1, rows);

        let footer_y = open_y + 1 + u16::try_from(rows).unwrap_or(1) + 1;
        out.set_stringn(
            left,
            footer_y,
            FOOTER,
            usize::from(right.saturating_sub(left)),
            muted,
        );
        cursor
    }

    /// The folder shown in `text`, then the query in `strong`; the folder is cut
    /// from the front with `…` when both don't fit. A typed absolute path stands
    /// alone, as the folder in front of it would read as part of it.
    fn render_header(&self, theme: &Theme, out: &mut Buffer, field: Rect) -> (u16, u16) {
        if field.width == 0 {
            return (field.x, field.y);
        }
        let typed = self.query.text();
        let mut x = field.x;
        if !Path::new(typed).is_absolute() {
            let mut shown = self.dir.display().to_string();
            if !shown.ends_with(std::path::MAIN_SEPARATOR) {
                shown.push(std::path::MAIN_SEPARATOR);
            }
            // The query keeps room for a few letters and the cursor.
            let query_room = typed.width() + 1;
            let room = usize::from(field.width).saturating_sub(query_room.max(8));
            let shown = cut_front(&shown, room);
            x = out
                .set_stringn(x, field.y, &shown, room, Style::new().fg(theme.text))
                .0;
        }
        let rest = Rect::new(x, field.y, field.right().saturating_sub(x), 1);
        self.query
            .render_value(out, rest, Style::new().fg(theme.strong))
    }

    /// `⏎ open <folder>`: what Enter does with nothing else picked.
    fn render_open_row(&self, theme: &Theme, out: &mut Buffer, card: Rect, y: u16) {
        let left = card.x + 4;
        let right = card.right() - 4;
        let selected = self.selected == 0;
        if selected {
            glow_row(theme, out, card.x, card.right(), y);
            enter_mark(theme, out, right, y);
        }
        let plain = selected && !theme.ramps();
        let pick = |style: Style| if plain { Style::new() } else { style };
        let end = usize::from(right.saturating_sub(left + 3));
        let (x, _) = out.set_stringn(left, y, "⏎ ", end, pick(Style::new().fg(theme.accent)));
        let used = usize::from(x - left);
        let label = pick(Style::new().fg(if selected { theme.strong } else { theme.fg }));
        let (x, _) = out.set_stringn(x, y, "open ", end.saturating_sub(used), label);
        let used = usize::from(x - left);
        let path = self.dir.display().to_string();
        let room = end.saturating_sub(used);
        out.set_stringn(
            x,
            y,
            cut_front(&path, room),
            room,
            pick(Style::new().fg(theme.muted)),
        );
    }

    /// Up to `rows` folders from row `y`, scrolled just far enough to keep a
    /// selected folder on the last row: `▸` then the name, its matched letters
    /// in the accent.
    fn render_folders(&self, theme: &Theme, out: &mut Buffer, card: Rect, y: u16, rows: usize) {
        let left = card.x + 4;
        let right = card.right() - 4;
        let muted = Style::new().fg(theme.muted);
        if self.matches.is_empty() {
            let why = if self.folders.is_empty() {
                "no folders here"
            } else {
                "no matching folders"
            };
            out.set_string(left, y, why, muted);
            return;
        }
        let first = self.selected.saturating_sub(1).saturating_sub(rows - 1);
        let mut matcher = self.matcher.clone();
        let mut indices = Vec::new();
        for (line, (index, &folder)) in self
            .matches
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .enumerate()
        {
            let row_y = y + u16::try_from(line).unwrap_or(u16::MAX);
            let selected = index + 1 == self.selected;
            if selected {
                glow_row(theme, out, card.x, card.right(), row_y);
                enter_mark(theme, out, right, row_y);
            }
            let plain = selected && !theme.ramps();
            let pick = |style: Style| if plain { Style::new() } else { style };
            out.set_string(left, row_y, "▸", pick(muted));
            let name = &self.folders[folder];
            self.matched(name, &mut matcher, &mut indices);
            let hit = pick(Style::new().fg(theme.accent)).add_modifier(Modifier::BOLD);
            let rest = pick(Style::new().fg(if selected { theme.strong } else { theme.fg }));
            // Two blanks short of `⏎`, so a long name never runs into it.
            let end = right.saturating_sub(3);
            let mut x = left + 2;
            for (at, ch) in name.chars().enumerate() {
                let cell = ch.to_string();
                let cells = width_of(&cell);
                if x + cells > end {
                    break;
                }
                let is_hit = u32::try_from(at).is_ok_and(|at| indices.binary_search(&at).is_ok());
                out.set_string(x, row_y, &cell, if is_hit { hit } else { rest });
                x += cells;
            }
        }
    }
}

/// The folders directly inside `dir`, by name: sorted ignoring case, those
/// starting with `.` last, as they're rarely the one wanted. Links to folders
/// count as folders. An entry that can't be read is left out; `dir` itself
/// failing is the error.
pub fn list_folders(dir: &Path) -> io::Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort_by(|a, b| {
        a.starts_with('.')
            .cmp(&b.starts_with('.'))
            .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
            .then_with(|| a.cmp(b))
    });
    Ok(names)
}

/// `path` made absolute with `.` and `..` worked out from the text alone, so
/// the header shows `C:\dev` rather than `C:\dev\glyph\..` and going up from it
/// goes where it says.
pub fn clean(path: &Path) -> PathBuf {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                // A root has no parent; `..` there stays at the root.
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Whether the query names a path rather than filtering: absolute, or holding
/// a separator.
fn looks_like_path(text: &str) -> bool {
    !text.is_empty()
        && (Path::new(text).is_absolute()
            || text.contains('/')
            || text.contains(std::path::MAIN_SEPARATOR))
}

fn cannot_open(dir: &Path, err: &io::Error) -> String {
    format!("cannot open {}: {err}", dir.display())
}

/// How many cells `text` takes.
fn width_of(text: &str) -> u16 {
    u16::try_from(text.width()).unwrap_or(u16::MAX)
}

/// `text` cut to `room` cells by dropping its start for `…`, as the end of a
/// path says most about where it is.
fn cut_front(text: &str, room: usize) -> String {
    if text.width() <= room {
        return text.to_string();
    }
    if room == 0 {
        return String::new();
    }
    let mut kept: Vec<char> = Vec::new();
    let mut used = 1;
    for c in text.chars().rev() {
        let w = c.to_string().width();
        if used + w > room {
            break;
        }
        used += w;
        kept.push(c);
    }
    std::iter::once('…').chain(kept.into_iter().rev()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::theme::mix;

    fn folder(names: &[&str]) -> anyhow::Result<tempfile::TempDir> {
        let dir = tempfile::tempdir()?;
        for name in names {
            fs::create_dir_all(dir.path().join(name))?;
        }
        fs::write(dir.path().join("file.txt"), "not a folder")?;
        Ok(dir)
    }

    fn browse(dir: &Path) -> anyhow::Result<DirPicker> {
        DirPicker::new(dir).map_err(anyhow::Error::msg)
    }

    fn type_query(p: &mut DirPicker, text: &str) {
        for c in text.chars() {
            p.handle(Input::Text(c));
        }
    }

    fn press(p: &mut DirPicker, action: Action) -> Option<Browsed> {
        p.handle(Input::Action(action))
    }

    #[test]
    fn folders_are_listed_ignoring_case_with_dot_folders_last_and_no_files() -> anyhow::Result<()> {
        let dir = folder(&["beta", "Alpha", ".git", "gamma", ".cache", "_build"])?;
        assert_eq!(
            list_folders(dir.path())?,
            ["_build", "Alpha", "beta", "gamma", ".cache", ".git"]
        );
        Ok(())
    }

    #[test]
    fn a_missing_folder_is_an_error() -> anyhow::Result<()> {
        let dir = folder(&[])?;
        assert!(list_folders(&dir.path().join("gone")).is_err());
        assert!(DirPicker::new(&dir.path().join("gone")).is_err());
        Ok(())
    }

    #[test]
    fn clean_works_out_dots_from_the_text() -> anyhow::Result<()> {
        let dir = folder(&["a/b"])?;
        let root = clean(dir.path());
        assert_eq!(
            clean(&root.join("a").join("..").join(".").join("a")),
            root.join("a")
        );
        assert!(clean(Path::new(".")).is_absolute());
        Ok(())
    }

    #[test]
    fn typing_filters_fuzzily_keeping_the_open_row() -> anyhow::Result<()> {
        let dir = folder(&["docs", "src", "scripts", "target"])?;
        let mut p = browse(dir.path())?;
        assert_eq!(p.listed(), ["docs", "scripts", "src", "target"]);
        assert_eq!(p.selected, 0);
        type_query(&mut p, "src");
        assert_eq!(p.listed().first(), Some(&"src"));
        assert!(!p.listed().contains(&"docs"));
        // The best match is selected; Up reaches the open row.
        assert_eq!(p.selected, 1);
        press(&mut p, Action::Move(Motion::Up));
        assert_eq!(p.selected, 0);
        assert_eq!(
            press(&mut p, Action::Newline),
            Some(Browsed::Open(clean(dir.path())))
        );
        type_query(&mut p, "zzz");
        assert!(p.listed().is_empty());
        assert_eq!(p.selected, 0);
        Ok(())
    }

    #[test]
    fn enter_goes_in_and_left_or_backspace_go_up() -> anyhow::Result<()> {
        let dir = folder(&["a/inner", "b"])?;
        let root = clean(dir.path());
        let mut p = browse(&root)?;
        press(&mut p, Action::Move(Motion::Down));
        assert_eq!(press(&mut p, Action::Newline), None);
        assert_eq!(p.dir(), root.join("a"));
        assert_eq!(p.listed(), ["inner"]);
        assert_eq!(p.selected, 0);
        press(&mut p, Action::Move(Motion::Left));
        assert_eq!(p.dir(), root);
        // Back up, the folder just left is selected.
        assert_eq!(p.selected, 1);
        press(&mut p, Action::Move(Motion::Down));
        press(&mut p, Action::Newline);
        assert_eq!(p.dir(), root.join("b"));
        press(&mut p, Action::Backspace);
        assert_eq!(p.dir(), root);
        // With a query, ← and Backspace edit it instead.
        type_query(&mut p, "ab");
        press(&mut p, Action::Move(Motion::Left));
        press(&mut p, Action::Backspace);
        assert_eq!(p.query.text(), "b");
        assert_eq!(p.dir(), root);
        Ok(())
    }

    #[test]
    fn going_up_stops_at_the_root() -> anyhow::Result<()> {
        let dir = folder(&[])?;
        let root = clean(dir.path())
            .ancestors()
            .last()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        let mut p = browse(&root)?;
        assert_eq!(press(&mut p, Action::Move(Motion::Left)), None);
        assert_eq!(p.dir(), root);
        Ok(())
    }

    #[test]
    fn a_typed_path_jumps_there_and_a_wrong_one_says_so() -> anyhow::Result<()> {
        let dir = folder(&["a/deep", "b"])?;
        let root = clean(dir.path());
        let mut p = browse(&root.join("b"))?;
        let sep = std::path::MAIN_SEPARATOR;
        type_query(&mut p, &format!("..{sep}a{sep}deep"));
        assert_eq!(press(&mut p, Action::Newline), None);
        assert_eq!(p.dir(), root.join("a").join("deep"));
        assert_eq!(p.query.text(), "");

        type_query(&mut p, &root.join("b").display().to_string());
        assert_eq!(press(&mut p, Action::Newline), None);
        assert_eq!(p.dir(), root.join("b"));

        type_query(&mut p, &format!("nope{sep}x"));
        assert_eq!(
            press(&mut p, Action::Newline),
            Some(Browsed::Failed(format!("no such folder: nope{sep}x")))
        );
        assert_eq!(p.dir(), root.join("b"));
        Ok(())
    }

    #[test]
    fn a_folder_that_cannot_be_read_says_why_and_stays() -> anyhow::Result<()> {
        let dir = folder(&["gone", "kept"])?;
        let root = clean(dir.path());
        let mut p = browse(&root)?;
        fs::remove_dir(root.join("gone"))?;
        press(&mut p, Action::Move(Motion::Down));
        let Some(Browsed::Failed(why)) = press(&mut p, Action::Newline) else {
            panic!("entering a vanished folder should fail");
        };
        assert!(why.starts_with("cannot open "), "{why}");
        assert!(why.contains("gone"), "{why}");
        assert_eq!(p.dir(), root);
        assert_eq!(p.selected, 1);
        Ok(())
    }

    #[test]
    fn ctrl_enter_opens_the_folder_shown_and_esc_closes() -> anyhow::Result<()> {
        let dir = folder(&["a", "b"])?;
        let mut p = browse(dir.path())?;
        press(&mut p, Action::Move(Motion::Down));
        assert_eq!(
            press(&mut p, Action::OpenFolderHere),
            Some(Browsed::Open(clean(dir.path())))
        );
        assert_eq!(press(&mut p, Action::Cancel), Some(Browsed::Close));
        Ok(())
    }

    fn draw(p: &DirPicker, theme: &Theme) -> anyhow::Result<Buffer> {
        let mut terminal = Terminal::new(TestBackend::new(100, 30))?;
        terminal.draw(|frame| {
            p.render(theme, frame, frame.area());
        })?;
        Ok(terminal.backend().buffer().clone())
    }

    fn card_row(buffer: &Buffer, card: Rect, y: u16) -> String {
        (card.x..card.right())
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn the_card_is_the_casts_with_header_rule_open_row_folders_and_footer() -> anyhow::Result<()> {
        let names: Vec<String> = (0..12).map(|i| format!("dir{i:02}")).collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        let dir = folder(&refs)?;
        let mut p = browse(dir.path())?;
        let theme = Theme::default();
        let area = Rect::new(0, 0, 100, 30);
        let card = p.card(area);
        assert_eq!(card, Rect::new(7, 4, 86, 18));
        let buffer = draw(&p, &theme)?;
        assert_eq!(buffer[(card.x, card.y)].symbol(), "▀");
        let header = card_row(&buffer, card, card.y + 2);
        assert!(header.starts_with("    ✦ "), "{header:?}");
        assert!(header.ends_with("open · folders    "), "{header:?}");
        assert_eq!(buffer[(card.x + 4, card.y + 2)].fg, theme.accent2);
        assert_eq!(buffer[(card.x + 5, card.y + 3)].symbol(), "─");
        let open = card_row(&buffer, card, card.y + 4);
        assert!(open.starts_with("    ⏎ open "), "{open:?}");
        assert!(open.ends_with("⏎    "), "{open:?}");
        assert_eq!(
            buffer[(card.x, card.y + 4)].bg,
            mix(theme.raised, theme.accent, 0.3)
        );
        let first = card_row(&buffer, card, card.y + 5);
        assert!(first.starts_with("    ▸ dir00 "), "{first:?}");
        let last = card_row(&buffer, card, card.y + 14);
        assert!(last.starts_with("    ▸ dir09 "), "{last:?}");
        let footer = card_row(&buffer, card, card.y + 16);
        assert!(footer.starts_with(&format!("    {FOOTER}")), "{footer:?}");

        // The selection scrolls the list past the card.
        for _ in 0..12 {
            press(&mut p, Action::Move(Motion::Down));
        }
        let buffer = draw(&p, &theme)?;
        let last = card_row(&buffer, card, card.y + 14);
        assert!(last.starts_with("    ▸ dir11 "), "{last:?}");
        assert!(last.ends_with("⏎    "), "{last:?}");
        Ok(())
    }

    #[test]
    fn mono_reverses_the_selected_row() -> anyhow::Result<()> {
        let dir = folder(&["a"])?;
        let p = browse(dir.path())?;
        let mono = Theme::named("mono").expect("mono exists");
        let buffer = draw(&p, &mono)?;
        let card = p.card(Rect::new(0, 0, 100, 30));
        assert!(
            buffer[(card.x + 6, card.y + 4)]
                .modifier
                .contains(Modifier::REVERSED)
        );
        assert!(
            !buffer[(card.x + 6, card.y + 5)]
                .modifier
                .contains(Modifier::REVERSED)
        );
        Ok(())
    }

    #[test]
    fn a_long_folder_is_cut_from_the_front() {
        assert_eq!(cut_front("abcdef", 4), "…def");
        assert_eq!(cut_front("abc", 4), "abc");
    }
}
