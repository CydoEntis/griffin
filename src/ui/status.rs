use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Paragraph;

/// Draws the one-row status line at the bottom of the screen, in the spec's order:
/// message, the buffer's name, then the cursor as 0-based (line, char column),
/// shown 1-based.
pub fn render_status(
    frame: &mut Frame,
    area: Rect,
    name: &str,
    message: Option<&str>,
    (line, col): (usize, usize),
) {
    let position = format!("Ln {}, Col {}", line + 1, col + 1);
    let text = match message {
        Some(message) => format!("griffin  {message}  {name}  {position}"),
        None => format!("griffin  {name}  {position}"),
    };
    let status = Paragraph::new(text).style(Style::new().reversed());
    frame.render_widget(status, area);
}
