use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::Paragraph;

/// Draws the one-row status line at the bottom of the screen.
pub fn render_status(frame: &mut Frame, area: Rect) {
    let status = Paragraph::new("griffin").style(Style::new().reversed());
    frame.render_widget(status, area);
}
