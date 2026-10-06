use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme::Theme;

/// What the status line shows about the active buffer.
#[derive(Debug, Default, Clone, Copy)]
pub struct Status<'a> {
    pub name: &'a str,
    /// Unsaved changes, shown as `●` after the name.
    pub dirty: bool,
    pub message: Option<&'a str>,
    /// The cursor as 0-based (line, char column).
    pub position: (usize, usize),
    /// The language server's warnings and errors for the buffer.
    pub warnings: usize,
    pub errors: usize,
}

/// Draws the one-row status line at the bottom of the screen, in the spec's order:
/// message, the buffer's name (with `●` while it has unsaved changes), the cursor
/// shown 1-based, then the diagnostic counts while there are any (`⚠` in
/// `warning`, `✕` in `err`). Chrome, so on `sidebar_bg`.
pub fn render_status(frame: &mut Frame, theme: &Theme, area: Rect, status: &Status) {
    let (line, col) = status.position;
    let position = format!("Ln {}, Col {}", line + 1, col + 1);
    let name = if status.dirty {
        format!("{} ●", status.name)
    } else {
        status.name.to_string()
    };
    let text = match status.message {
        Some(message) => format!("griffin  {message}  {name}  {position}"),
        None => format!("griffin  {name}  {position}"),
    };
    let mut spans = vec![Span::raw(text)];
    if status.warnings + status.errors > 0 {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("⚠ {}", status.warnings),
            Style::new().fg(theme.warning),
        ));
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!("✕ {}", status.errors),
            Style::new().fg(theme.err),
        ));
    }
    let status =
        Paragraph::new(Line::from(spans)).style(Style::new().bg(theme.sidebar_bg).fg(theme.text));
    frame.render_widget(status, area);
}
