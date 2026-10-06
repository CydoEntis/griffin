//! The tab bar on row 0 above the editor: one ` name ` cell run per open buffer,
//! with `●` on the ones holding unsaved changes.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use unicode_width::UnicodeWidthStr;

/// What one tab shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabLabel {
    pub name: String,
    pub dirty: bool,
}

impl TabLabel {
    fn text(&self) -> String {
        if self.dirty {
            format!(" {} ● ", self.name)
        } else {
            format!(" {} ", self.name)
        }
    }
}

/// Where each tab lands in a bar `width` cells wide, as (column offset, width);
/// `None` for tabs that don't fit. When they don't all fit, tabs drop off the left
/// until the active one does, so the tab being edited is always on screen.
pub fn layout(labels: &[TabLabel], active: usize, width: u16) -> Vec<Option<(u16, u16)>> {
    let widths: Vec<usize> = labels.iter().map(|label| label.text().width()).collect();
    let width = usize::from(width);
    let mut first = 0;
    while first < active && widths[first..=active].iter().sum::<usize>() > width {
        first += 1;
    }
    let mut x = 0;
    widths
        .iter()
        .enumerate()
        .map(|(index, &w)| {
            if index < first || x >= width {
                return None;
            }
            let at = x;
            x += w;
            // Both fit in u16: `at` is below `width`, which came from a u16, and the
            // width shown is cut to what's left of it.
            let shown = w.min(width - at);
            Some((
                u16::try_from(at).unwrap_or(u16::MAX),
                u16::try_from(shown).unwrap_or(u16::MAX),
            ))
        })
        .collect()
}

/// The tab under screen column `col` of a bar drawn in `area`.
pub fn tab_at(labels: &[TabLabel], active: usize, area: Rect, col: u16) -> Option<usize> {
    let offset = col.checked_sub(area.x)?;
    layout(labels, active, area.width)
        .iter()
        .position(|place| place.is_some_and(|(x, w)| (x..x + w).contains(&offset)))
}

/// Draws the bar into `area`. The active tab is reversed, standing in for
/// `tab_active_bg`/`tab_active_fg` until themes exist (#17).
pub fn render_tabs(labels: &[TabLabel], active: usize, area: Rect, frame: &mut Frame) {
    let out = frame.buffer_mut();
    for (index, place) in layout(labels, active, area.width).into_iter().enumerate() {
        let Some((x, w)) = place else {
            continue;
        };
        let style = if index == active {
            Style::new().reversed()
        } else {
            Style::new()
        };
        out.set_stringn(
            area.x + x,
            area.y,
            labels[index].text(),
            usize::from(w),
            style,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn label(name: &str, dirty: bool) -> TabLabel {
        TabLabel {
            name: name.into(),
            dirty,
        }
    }

    #[test]
    fn tabs_sit_side_by_side_with_a_dirty_mark() {
        let labels = [label("a.txt", false), label("b.txt", true)];
        assert_eq!(layout(&labels, 0, 40), vec![Some((0, 7)), Some((7, 9))]);
        let area = Rect::new(31, 0, 40, 1);
        assert_eq!(tab_at(&labels, 0, area, 31), Some(0));
        assert_eq!(tab_at(&labels, 0, area, 37), Some(0));
        assert_eq!(tab_at(&labels, 0, area, 38), Some(1));
        assert_eq!(tab_at(&labels, 0, area, 47), None);
        assert_eq!(tab_at(&labels, 0, area, 5), None);
    }

    #[test]
    fn tabs_drop_off_the_left_so_the_active_one_shows() {
        let labels: Vec<TabLabel> = (0..5).map(|n| label(&format!("f{n}"), false)).collect();
        // Each tab is 4 cells; 10 fit two and a half.
        assert_eq!(
            layout(&labels, 0, 10),
            vec![Some((0, 4)), Some((4, 4)), Some((8, 2)), None, None]
        );
        assert_eq!(
            layout(&labels, 4, 10),
            vec![None, None, None, Some((0, 4)), Some((4, 4))]
        );
    }

    #[test]
    fn renders_names_with_the_active_tab_reversed() -> anyhow::Result<()> {
        let labels = [label("a.txt", false), label("b.txt", true)];
        let mut terminal = Terminal::new(TestBackend::new(20, 1))?;
        terminal.draw(|frame| render_tabs(&labels, 1, Rect::new(0, 0, 20, 1), frame))?;
        let buffer = terminal.backend().buffer();
        let row: String = (0..20).map(|x| buffer[(x, 0)].symbol()).collect();
        assert_eq!(row.trim_end(), " a.txt  b.txt ●");
        let reversed: String = (0..20)
            .filter(|&x| {
                buffer[(x, 0)]
                    .modifier
                    .contains(ratatui::style::Modifier::REVERSED)
            })
            .map(|x| buffer[(x, 0)].symbol())
            .collect();
        assert_eq!(reversed, " b.txt ● ");
        Ok(())
    }
}
