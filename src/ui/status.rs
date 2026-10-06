use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Paragraph;

/// Draws the one-row status line at the bottom of the screen, in the spec's order:
/// message, then the buffer's name.
pub fn render_status(frame: &mut Frame, area: Rect, name: &str, message: Option<&str>) {
    let text = match message {
        Some(message) => format!("griffin  {message}  {name}"),
        None => format!("griffin  {name}"),
    };
    let status = Paragraph::new(text).style(Style::new().reversed());
    frame.render_widget(status, area);
}
