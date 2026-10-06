pub mod completion;
pub mod confirm;
pub mod find;
pub mod hover;
pub mod picker;
pub mod prompt;
pub mod run;
pub mod search;
pub mod status;
pub mod tabs;
pub mod tree;

use ratatui::style::Style;
use ratatui::widgets::Block;

use crate::theme::Theme;

/// The frame every popup card is drawn in: a `border` line around `card`.
pub fn card_block(theme: &Theme) -> Block<'static> {
    Block::bordered()
        .border_style(Style::new().fg(theme.border))
        .style(Style::new().bg(theme.card).fg(theme.text))
}
