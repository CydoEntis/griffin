//! The run panel: a title row naming the command and how it's doing, over the
//! command's latest output, ANSI colours kept.

use std::collections::VecDeque;

use ansi_to_tui::IntoText;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// Output lines the panel keeps; older ones are dropped.
pub const MAX_LINES: usize = 10_000;

/// Display columns a tab in the output takes.
const TAB: &str = "    ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    /// Shift+F5 killed it.
    Stopped,
    /// `None` when the process ended without a code, e.g. killed by a signal.
    Exited(Option<i32>),
}

/// One command's run, as the panel shows it.
#[derive(Debug, Clone)]
pub struct RunView {
    /// Which run this is, so output from an earlier one is dropped.
    pub id: u64,
    pub name: String,
    pub status: RunStatus,
    lines: VecDeque<Line<'static>>,
}

impl RunView {
    pub fn new(id: u64, name: String) -> Self {
        RunView {
            id,
            name,
            status: RunStatus::Running,
            lines: VecDeque::new(),
        }
    }

    /// Starts the output with a marker saying the command was restarted.
    pub fn mark_restarted(&mut self) {
        self.lines.push_back(Line::from(Span::styled(
            "── restarted ──",
            Style::new().dim(),
        )));
    }

    /// Adds one line of output, keeping at most `MAX_LINES`.
    pub fn push(&mut self, raw: &str) {
        if self.lines.len() == MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(to_line(raw));
    }

    /// `dev · running` or `dev · exited 0`.
    pub fn title(&self) -> String {
        match self.status {
            RunStatus::Running => format!("{} · running", self.name),
            RunStatus::Stopped => format!("{} · stopped", self.name),
            RunStatus::Exited(Some(code)) => format!("{} · exited {code}", self.name),
            RunStatus::Exited(None) => format!("{} · exited", self.name),
        }
    }
}

/// One output line as styled text. Colour codes become styles; other control
/// characters are dropped, since written to the terminal as cells they would
/// move its cursor or ring its bell.
fn to_line(raw: &str) -> Line<'static> {
    let clean: String = raw
        .chars()
        .flat_map(|c| match c {
            '\t' => TAB.chars().collect::<Vec<_>>(),
            '\x1b' => vec![c],
            c if c.is_control() => Vec::new(),
            c => vec![c],
        })
        .collect();
    match clean.as_bytes().into_text() {
        Ok(text) => {
            let spans: Vec<Span<'static>> =
                text.lines.into_iter().flat_map(|line| line.spans).collect();
            Line::from(spans)
        }
        // Unparseable codes: show the text as it came rather than lose the line.
        Err(_) => Line::from(clean),
    }
}

/// Draws the panel in `area`: the title row on `sidebar_bg`, then as many of the
/// latest lines as fit.
pub fn render_run_panel(theme: &Theme, run: Option<&RunView>, area: Rect, frame: &mut Frame) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let chrome = Style::new().bg(theme.sidebar_bg).fg(theme.text);
    let out = frame.buffer_mut();
    out.set_style(area, Style::new().bg(theme.bg).fg(theme.fg));
    let title_row = Rect { height: 1, ..area };
    out.set_style(title_row, chrome);
    let width = usize::from(area.width);
    let title = match run {
        Some(run) => format!(" {}", run.title()),
        None => " run".to_string(),
    };
    out.set_stringn(area.x, area.y, title, width, chrome.fg(theme.strong));

    let body = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };
    let Some(run) = run else {
        if body.height > 0 {
            out.set_stringn(
                body.x + 1,
                body.y,
                "F5 runs a command from .glyph.toml",
                width.saturating_sub(1),
                Style::new().fg(theme.muted),
            );
        }
        return;
    };
    let rows = usize::from(body.height);
    let first = run.lines.len().saturating_sub(rows);
    for (offset, line) in run.lines.iter().skip(first).enumerate() {
        let y = body.y + u16::try_from(offset).unwrap_or(u16::MAX);
        out.set_line(body.x + 1, y, line, area.width.saturating_sub(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;

    #[test]
    fn ansi_colours_become_styles_and_codes_disappear() {
        let line = to_line("\x1b[31mred\x1b[0m plain");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "red plain");
        assert_eq!(line.spans[0].style.fg, Some(Color::Red));
    }

    #[test]
    fn tabs_expand_and_other_control_characters_go() {
        let line = to_line("a\tb\x07c\rd");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "a    bcd");
    }

    #[test]
    fn only_the_last_ten_thousand_lines_are_kept() {
        let mut run = RunView::new(1, "dev".into());
        for n in 0..MAX_LINES + 5 {
            run.push(&n.to_string());
        }
        assert_eq!(run.lines.len(), MAX_LINES);
        assert_eq!(run.lines.front().map(|l| l.to_string()), Some("5".into()));
    }

    #[test]
    fn the_title_says_running_or_the_exit_code() {
        let mut run = RunView::new(1, "dev".into());
        assert_eq!(run.title(), "dev · running");
        run.status = RunStatus::Exited(Some(2));
        assert_eq!(run.title(), "dev · exited 2");
        run.status = RunStatus::Exited(None);
        assert_eq!(run.title(), "dev · exited");
        run.status = RunStatus::Stopped;
        assert_eq!(run.title(), "dev · stopped");
    }

    #[test]
    fn the_latest_lines_fill_the_panel_under_the_title() -> anyhow::Result<()> {
        let mut run = RunView::new(1, "dev".into());
        for n in 1..=5 {
            run.push(&format!("line {n}"));
        }
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(20, 3))?;
        terminal.draw(|frame| render_run_panel(&theme, Some(&run), frame.area(), frame))?;
        let buffer = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..20).map(|x| buffer[(x, y)].symbol()).collect() };
        assert!(row(0).starts_with(" dev · running"), "{:?}", row(0));
        assert!(row(1).starts_with(" line 4"), "{:?}", row(1));
        assert!(row(2).starts_with(" line 5"), "{:?}", row(2));
        Ok(())
    }
}
