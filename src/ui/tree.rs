//! The file tree panel on the left and the `│` divider beside it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;

use crate::theme::Theme;
use crate::workspace::Tree;

/// Columns the tree takes, not counting the divider.
pub const TREE_WIDTH: u16 = 30;

/// Draws the visible rows of `tree` into `area`: two spaces of indent per level, a
/// `▸`/`▾` marker on folders, the name cut at the edge, on `sidebar_bg`. The
/// selected row is filled with `hov` across the whole width, dimmed while the
/// editor has focus. Returns where the selected row is drawn, for the terminal
/// cursor.
pub fn render_tree(
    theme: &Theme,
    tree: &Tree,
    focused: bool,
    area: Rect,
    frame: &mut Frame,
) -> Option<(u16, u16)> {
    let out = frame.buffer_mut();
    out.set_style(area, Style::new().bg(theme.sidebar_bg).fg(theme.text));
    let width = usize::from(area.width);
    let mut selected_at = None;
    for line in 0..area.height {
        let Some(index) = tree.row_at(usize::from(line)) else {
            break;
        };
        let row = &tree.rows()[index];
        let marker = match (row.entry.is_dir, row.expanded) {
            (true, true) => "▾ ",
            (true, false) => "▸ ",
            (false, _) => "  ",
        };
        let text = format!("{}{marker}{}", "  ".repeat(row.depth), row.entry.name);
        let y = area.y + line;
        let style = if index == tree.selected() {
            selected_at = Some((area.x, y));
            let filled = Theme::highlight(theme.hov, theme.strong);
            if focused { filled } else { filled.dim() }
        } else {
            Style::new()
        };
        // Pad to the full width so a selected row is highlighted edge to edge.
        out.set_stringn(area.x, y, format!("{text:<width$}"), width, style);
    }
    selected_at
}

/// A vertical `│` down `area`, in the theme's `line` colour.
pub fn render_divider(theme: &Theme, area: Rect, frame: &mut Frame) {
    let out = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        out.set_string(area.x, y, "│", Style::new().fg(theme.line));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Modifier;
    use std::fs;

    #[test]
    fn rows_are_indented_marked_and_the_selection_reversed() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::create_dir(dir.path().join("src"))?;
        fs::write(dir.path().join("src").join("main.rs"), "")?;
        fs::write(dir.path().join("notes.txt"), "")?;
        let mut tree = Tree::new(dir.path());
        tree.expand();
        tree.move_by(1);

        let mut terminal = Terminal::new(TestBackend::new(20, 4))?;
        let mut at = None;
        terminal.draw(|frame| {
            at = render_tree(
                &Theme::default(),
                &tree,
                true,
                Rect::new(0, 0, 12, 4),
                frame,
            )
        })?;
        let buffer = terminal.backend().buffer();
        let row = |y: u16| -> String { (0..12).map(|x| buffer[(x, y)].symbol()).collect() };
        assert_eq!(row(0), "▾ src       ");
        assert_eq!(row(1), "    main.rs ");
        assert_eq!(row(2), "  notes.txt ");
        assert_eq!(row(3).trim(), "");
        assert_eq!(at, Some((0, 1)));
        for x in 0..12 {
            assert!(buffer[(x, 1)].modifier.contains(Modifier::REVERSED));
            assert!(!buffer[(x, 0)].modifier.contains(Modifier::REVERSED));
        }
        Ok(())
    }
}
