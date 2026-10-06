use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Paragraph;

use crate::theme::Theme;

/// Draws the one-row status line at the bottom of the screen, in the spec's order:
/// message, the buffer's name (with `●` while it has unsaved changes), then the
/// cursor as 0-based (line, char column), shown 1-based. Chrome, so on
/// `sidebar_bg`.
pub fn render_status(
    frame: &mut Frame,
    theme: &Theme,
    area: Rect,
    name: &str,
    dirty: bool,
    message: Option<&str>,
    (line, col): (usize, usize),
) {
    let position = format!("Ln {}, Col {}", line + 1, col + 1);
    let name = if dirty {
        format!("{name} ●")
    } else {
        name.to_string()
    };
    let text = match message {
        Some(message) => format!("griffin  {message}  {name}  {position}"),
        None => format!("griffin  {name}  {position}"),
    };
    let status = Paragraph::new(text).style(Style::new().bg(theme.sidebar_bg).fg(theme.text));
    frame.render_widget(status, area);
}
