//! The run panel: a title row naming the command and how it's doing, over the
//! command's latest output, its ANSI colours mapped onto the theme's roles.

use std::collections::VecDeque;

use ansi_to_tui::IntoText;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

/// Output lines the panel keeps; older ones are dropped.
pub const MAX_LINES: usize = 10_000;

/// Display columns a tab in the output takes.
const TAB: &str = "    ";

/// The keys the title row offers while the command runs, and once it's done
/// (SPEC_V1_LAYOUT §9).
const RUNNING_HINTS: &str = "shift+F5 stop   ctrl+F5 restart   F4 hide";
const DONE_HINTS: &str = "F5 run again   F4 hide";
/// The same for a debug session's output: Alt+F6 is what stops it.
const DEBUG_RUNNING_HINTS: &str = "alt+F6 stop   F4 hide";
const DEBUG_DONE_HINTS: &str = "alt+F5 debug again   F4 hide";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Running,
    /// Shift+F5 killed it.
    Stopped,
    /// `None` when the process ended without a code, e.g. killed by a signal.
    Exited(Option<i32>),
    /// A debugged program that ended without the adapter giving its exit code
    /// (a lone DAP `terminated`). Nothing says it failed, so it isn't drawn
    /// as a failure.
    Ended,
}

/// One row of the panel's body.
#[derive(Debug, Clone)]
enum Row {
    Output(Line<'static>),
    /// The rule a restart starts with, stamped with the local time it happened.
    /// Kept apart from output since it spans the panel's width, known only when
    /// drawn.
    Restarted(String),
}

/// One command's run, as the panel shows it.
#[derive(Debug, Clone)]
pub struct RunView {
    /// Which run this is, so output from an earlier one is dropped.
    pub id: u64,
    pub name: String,
    /// The command line, shown after the state in the title.
    pub command: String,
    pub status: RunStatus,
    /// The output is a debugged program's, which the debugger's keys control
    /// rather than the run keys.
    pub debug: bool,
    lines: VecDeque<Row>,
}

impl RunView {
    pub fn new(id: u64, name: String, command: String) -> Self {
        RunView {
            id,
            name,
            command,
            status: RunStatus::Running,
            debug: false,
            lines: VecDeque::new(),
        }
    }

    /// A panel for a debugged program's output.
    pub fn debug(id: u64, name: String, command: String) -> Self {
        RunView {
            debug: true,
            ..RunView::new(id, name, command)
        }
    }

    /// Starts the output with a rule saying the command was restarted at `at`,
    /// an `HH:MM:SS` local time.
    pub fn mark_restarted(&mut self, at: String) {
        self.lines.push_back(Row::Restarted(at));
    }

    /// Adds one line of output, keeping at most `MAX_LINES`.
    pub fn push(&mut self, raw: &str) {
        if self.lines.len() == MAX_LINES {
            self.lines.pop_front();
        }
        self.lines.push_back(Row::Output(to_line(raw)));
    }

    /// The status glyph, the state word, and the colour both are drawn in
    /// (SPEC_V1_LAYOUT §9, README §5.7).
    fn state(&self, theme: &Theme) -> (&'static str, String, Color) {
        match self.status {
            RunStatus::Running => ("●", "running".into(), theme.warn),
            RunStatus::Exited(Some(0)) => ("✓", "exited 0".into(), theme.ok),
            RunStatus::Exited(Some(code)) => ("✕", format!("exited {code}"), theme.err),
            RunStatus::Exited(None) => ("✕", "exited".into(), theme.err),
            RunStatus::Stopped => ("■", "stopped".into(), theme.muted),
            RunStatus::Ended => ("■", "ended".into(), theme.muted),
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

/// The theme's colour for one of the sixteen ANSI colours, so program output
/// matches the theme (SPEC_V1_LAYOUT §9); a bright colour shares its base's
/// role. 256-colour and RGB codes pass through, and a reset goes back to
/// `default`, the panel's own colour, rather than the terminal's.
fn ansi_role(theme: &Theme, color: Color, default: Color) -> Color {
    let syntax = |style: Style| style.fg.unwrap_or(default);
    match color {
        Color::Red | Color::LightRed => theme.err,
        Color::Green | Color::LightGreen => theme.ok,
        Color::Yellow | Color::LightYellow => theme.warn,
        Color::Blue | Color::LightBlue => syntax(theme.syntax.function),
        Color::Magenta | Color::LightMagenta => syntax(theme.syntax.keyword),
        Color::Cyan | Color::LightCyan => syntax(theme.syntax.r#type),
        Color::Gray | Color::White => theme.strong,
        Color::Black | Color::DarkGray => theme.muted,
        Color::Reset => default,
        other => other,
    }
}

/// `line` with its ANSI colours swapped for the theme's roles.
fn themed(theme: &Theme, line: &Line<'static>) -> Line<'static> {
    let spans = line.spans.iter().map(|span| {
        let mut style = span.style;
        style.fg = style.fg.map(|c| ansi_role(theme, c, theme.fg));
        style.bg = style.bg.map(|c| ansi_role(theme, c, theme.bg));
        Span::styled(span.content.clone(), style)
    });
    Line::from(spans.collect::<Vec<_>>())
}

/// Draws the panel in `area`: the title row on `surface`, then as many of the
/// latest lines as fit, on `bg`.
pub fn render_run_panel(theme: &Theme, run: Option<&RunView>, area: Rect, frame: &mut Frame) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let out = frame.buffer_mut();
    out.set_style(area, Style::new().bg(theme.bg).fg(theme.fg));
    let title_row = Rect { height: 1, ..area };
    out.set_style(title_row, Style::new().bg(theme.surface).fg(theme.text));
    let width = usize::from(area.width);
    let body = Rect {
        y: area.y + 1,
        height: area.height - 1,
        ..area
    };
    let Some(run) = run else {
        out.set_stringn(
            area.x,
            area.y,
            " run",
            width,
            Style::new().fg(theme.strong).add_modifier(Modifier::BOLD),
        );
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
    render_title(theme, run, title_row, frame);

    let out = frame.buffer_mut();
    let room = area.width.saturating_sub(1);
    let rows = usize::from(body.height);
    let first = run.lines.len().saturating_sub(rows);
    for (offset, row) in run.lines.iter().skip(first).enumerate() {
        let y = body.y + u16::try_from(offset).unwrap_or(u16::MAX);
        match row {
            Row::Output(line) => {
                out.set_line(body.x + 1, y, &themed(theme, line), room);
            }
            Row::Restarted(at) => {
                let rule = format!("── restarted {at} ");
                let rest = usize::from(room).saturating_sub(rule.width());
                let rule = format!("{rule}{}", "─".repeat(rest));
                out.set_stringn(
                    body.x + 1,
                    y,
                    rule,
                    usize::from(room),
                    Style::new().fg(theme.muted),
                );
            }
        }
    }
}

/// The title row (SPEC_V1_LAYOUT §9): one cell in, the status glyph and a
/// space, the name in `strong` bold, two spaces, the state word in the state's
/// colour, three spaces, then the command in `muted`, cut short of the hints,
/// which end one cell from the right edge. The hints are left out when the
/// command wouldn't have even a couple of cells beside them.
fn render_title(theme: &Theme, run: &RunView, row: Rect, frame: &mut Frame) {
    let out = frame.buffer_mut();
    let (glyph, word, color) = run.state(theme);
    let state = Style::new().fg(color);
    let muted = Style::new().fg(theme.muted);
    let right = row.right();
    let mut x = row.x + 1;
    let mut put = |text: &str, style: Style| {
        let room = usize::from(right.saturating_sub(x));
        x = out.set_stringn(x, row.y, text, room, style).0;
    };
    put(glyph, state);
    put(" ", Style::new());
    put(
        &run.name,
        Style::new().fg(theme.strong).add_modifier(Modifier::BOLD),
    );
    put("  ", Style::new());
    put(&word, state);
    put("   ", Style::new());

    let hints = match (run.debug, run.status == RunStatus::Running) {
        (false, true) => RUNNING_HINTS,
        (false, false) => DONE_HINTS,
        (true, true) => DEBUG_RUNNING_HINTS,
        (true, false) => DEBUG_DONE_HINTS,
    };
    let hints_width = u16::try_from(hints.width()).unwrap_or(u16::MAX);
    let hints_x = right.saturating_sub(hints_width.saturating_add(1));
    let fits = hints_x >= x + 4;
    // Two blanks between the command and the hints.
    let command_end = if fits { hints_x - 2 } else { right };
    let room = usize::from(command_end.saturating_sub(x));
    out.set_stringn(x, row.y, &run.command, room, muted);
    if fits {
        out.set_string(hints_x, row.y, hints, muted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn view() -> RunView {
        RunView::new(1, "dev".into(), "npm run dev".into())
    }

    fn draw(run: &RunView, width: u16, height: u16) -> anyhow::Result<Terminal<TestBackend>> {
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(width, height))?;
        terminal.draw(|frame| render_run_panel(&theme, Some(run), frame.area(), frame))?;
        Ok(terminal)
    }

    fn row(terminal: &Terminal<TestBackend>, y: u16) -> String {
        let buffer = terminal.backend().buffer();
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect()
    }

    #[test]
    fn ansi_colours_become_styles_and_codes_disappear() {
        let line = to_line("\x1b[31mred\x1b[0m plain");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "red plain");
        assert_eq!(line.spans[0].style.fg, Some(Color::Red));
    }

    #[test]
    fn ansi_colours_map_onto_the_theme_roles() {
        let theme = Theme::default();
        let fg = |code: &str| {
            let line = themed(&theme, &to_line(&format!("\x1b[{code}mx")));
            line.spans[0].style.fg
        };
        assert_eq!(fg("31"), Some(theme.err));
        assert_eq!(fg("92"), Some(theme.ok));
        assert_eq!(fg("33"), Some(theme.warn));
        assert_eq!(fg("34"), theme.syntax.function.fg);
        assert_eq!(fg("35"), theme.syntax.keyword.fg);
        assert_eq!(fg("36"), theme.syntax.r#type.fg);
        assert_eq!(fg("37"), Some(theme.strong));
        assert_eq!(fg("30"), Some(theme.muted));
        assert_eq!(fg("90"), Some(theme.muted));
        assert_eq!(fg("38;5;141"), Some(Color::Indexed(141)));
        assert_eq!(fg("38;2;1;2;3"), Some(Color::Rgb(1, 2, 3)));
        assert_eq!(fg("39"), Some(theme.fg));
    }

    #[test]
    fn tabs_expand_and_other_control_characters_go() {
        let line = to_line("a\tb\x07c\rd");
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "a    bcd");
    }

    #[test]
    fn only_the_last_ten_thousand_lines_are_kept() {
        let mut run = view();
        for n in 0..MAX_LINES + 5 {
            run.push(&n.to_string());
        }
        assert_eq!(run.lines.len(), MAX_LINES);
        let first = match run.lines.front() {
            Some(Row::Output(line)) => line.to_string(),
            other => panic!("{other:?}"),
        };
        assert_eq!(first, "5");
    }

    #[test]
    fn the_title_has_a_glyph_and_word_per_state() -> anyhow::Result<()> {
        let theme = Theme::default();
        let mut run = view();
        let cases = [
            (
                RunStatus::Running,
                " ● dev  running   npm run dev",
                theme.warn,
            ),
            (
                RunStatus::Exited(Some(0)),
                " ✓ dev  exited 0   npm run dev",
                theme.ok,
            ),
            (
                RunStatus::Exited(Some(2)),
                " ✕ dev  exited 2   npm run dev",
                theme.err,
            ),
            (
                RunStatus::Exited(None),
                " ✕ dev  exited   npm run dev",
                theme.err,
            ),
            (
                RunStatus::Stopped,
                " ■ dev  stopped   npm run dev",
                theme.muted,
            ),
            (RunStatus::Ended, " ■ dev  ended   npm run dev", theme.muted),
        ];
        for (status, title, color) in cases {
            run.status = status;
            let terminal = draw(&run, 80, 2)?;
            let buffer = terminal.backend().buffer();
            assert!(
                row(&terminal, 0).starts_with(title),
                "{:?}",
                row(&terminal, 0)
            );
            assert_eq!(buffer[(1, 0)].fg, color);
            assert_eq!(buffer[(3, 0)].fg, theme.strong);
            assert!(buffer[(3, 0)].modifier.contains(Modifier::BOLD));
            assert_eq!(buffer[(8, 0)].fg, color);
            assert_eq!(buffer[(0, 0)].bg, theme.surface);
        }
        Ok(())
    }

    #[test]
    fn the_hints_end_at_the_right_and_go_when_there_is_no_room() -> anyhow::Result<()> {
        let mut run = view();
        let terminal = draw(&run, 80, 2)?;
        assert!(row(&terminal, 0).ends_with(&format!("{RUNNING_HINTS} ")));
        run.status = RunStatus::Exited(Some(0));
        let terminal = draw(&run, 80, 2)?;
        assert!(row(&terminal, 0).ends_with(&format!("{DONE_HINTS} ")));
        let terminal = draw(&run, 30, 2)?;
        assert!(!row(&terminal, 0).contains("F4"), "{:?}", row(&terminal, 0));
        Ok(())
    }

    #[test]
    fn a_restart_is_a_muted_rule_across_the_panel() -> anyhow::Result<()> {
        let theme = Theme::default();
        let mut run = view();
        run.mark_restarted("12:34:56".into());
        let terminal = draw(&run, 30, 2)?;
        let line = row(&terminal, 1);
        assert_eq!(line, format!(" ── restarted 12:34:56 {}", "─".repeat(7)));
        assert_eq!(terminal.backend().buffer()[(5, 1)].fg, theme.muted);
        Ok(())
    }

    #[test]
    fn the_latest_lines_fill_the_panel_under_the_title() -> anyhow::Result<()> {
        let theme = Theme::default();
        let mut run = view();
        for n in 1..=5 {
            run.push(&format!("line {n}"));
        }
        let terminal = draw(&run, 20, 3)?;
        assert!(row(&terminal, 0).starts_with(" ● dev  running"));
        assert!(row(&terminal, 1).starts_with(" line 4"));
        assert!(row(&terminal, 2).starts_with(" line 5"));
        assert_eq!(terminal.backend().buffer()[(1, 2)].bg, theme.bg);
        Ok(())
    }
}
