//! The file tree panel on the left (design README §2.1): the `✦ tome` brand and
//! project folder, then one row per visible node. It's set off from the editor by
//! its `surface` ground alone; the `│` divider here is only drawn between splits.

use std::path::{Path, PathBuf};

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::theme::{Theme, grad, mix};
use crate::workspace::Tree;

/// Columns the tree takes. The editor starts one column further right.
pub const TREE_WIDTH: u16 = 28;

/// Rows above the first node: a blank, the brand row and another blank.
const NODES_TOP: u16 = 3;

/// Where the project folder starts on the brand row.
const PROJECT_X: u16 = 12;

/// What the tree marks besides its own rows. Paths are absolute.
pub struct TreeMarks<'a> {
    /// The project folder as the brand row shows it (see `project_label`).
    pub project: &'a str,
    /// The focused split's active file, whose row glows.
    pub active: Option<&'a Path>,
    /// Open files with unsaved changes, which get a `•`.
    pub dirty: &'a [PathBuf],
}

/// The part of the tree's `area` the nodes are listed in, below the brand. Mouse
/// clicks, scrolling and paging all count rows from its top.
pub fn nodes_area(area: Rect) -> Rect {
    let top = NODES_TOP.min(area.height);
    Rect {
        y: area.y + top,
        height: area.height - top,
        ..area
    }
}

/// `root` as the brand row names it: under the home folder it reads `~/a/b`.
pub fn project_label(root: &Path, home: Option<&Path>) -> String {
    let root = std::path::absolute(root).unwrap_or_else(|_| root.to_path_buf());
    match home.and_then(|home| root.strip_prefix(home).ok()) {
        Some(rest) => rest.components().fold(String::from("~"), |label, part| {
            format!("{label}/{}", part.as_os_str().to_string_lossy())
        }),
        None => root.display().to_string(),
    }
}

/// Draws the tree into `area` (README §2.1). Nodes are indented two columns per
/// level with `│` guides under their ancestors, folders carry `▾`/`▸`, names are
/// cut with `…`. The active file's row, and the selected row while the tree has
/// focus, get the glow ramp and a bold `strong` name; `mono` has no ramps, so its
/// selected row is reversed instead. Returns where the selected row's marker is,
/// for the terminal cursor.
pub fn render_tree(
    theme: &Theme,
    tree: &Tree,
    marks: &TreeMarks,
    focused: bool,
    area: Rect,
    frame: &mut Frame,
) -> Option<(u16, u16)> {
    let out = frame.buffer_mut();
    out.set_style(area, Style::new().bg(theme.surface).fg(theme.text));
    let width = area.width;
    // Names and the dirty mark stop three columns short of the edge.
    let end = width.saturating_sub(3);

    if area.height > 1 {
        let y = area.y + 1;
        put(out, area, 2, y, "✦", Style::new().fg(theme.accent2));
        let brand = ['t', 'o', 'm', 'e'];
        for (i, c) in (0u16..).zip(brand) {
            let fg = grad(&[theme.accent, theme.accent2], f64::from(i) / 3.0);
            let style = Style::new().fg(fg).add_modifier(Modifier::BOLD);
            put(out, area, 4 + i, y, c.encode_utf8(&mut [0; 4]), style);
        }
        let room = usize::from(end.saturating_sub(PROJECT_X));
        let project = cut_front(marks.project, room);
        put(
            out,
            area,
            PROJECT_X,
            y,
            &project,
            Style::new().fg(theme.muted),
        );
    }

    let nodes = nodes_area(area);
    let glow = [
        mix(theme.surface, theme.accent, 0.26),
        mix(theme.surface, theme.accent, 0.08),
        theme.surface,
    ];
    let mut selected_at = None;
    for line in 0..nodes.height {
        let Some(index) = tree.row_at(usize::from(line)) else {
            break;
        };
        let row = &tree.rows()[index];
        let y = nodes.y + line;
        let path = std::path::absolute(&row.entry.path).unwrap_or_else(|_| row.entry.path.clone());
        let selected = index == tree.selected();
        let lit = marks.active == Some(path.as_path()) || (selected && focused);
        if lit && !theme.flat() {
            for i in 0..width {
                let bg = grad(&glow, f64::from(i) / f64::from(width));
                out[(area.x + i, y)].set_bg(bg);
            }
        } else if selected && focused {
            out.set_style(
                Rect::new(area.x, y, width, 1),
                Style::new().add_modifier(Modifier::REVERSED),
            );
        }
        let depth = u16::try_from(row.depth).unwrap_or(u16::MAX);
        for d in 0..depth {
            put(
                out,
                area,
                d.saturating_mul(2).saturating_add(3),
                y,
                "│",
                Style::new().fg(theme.guide),
            );
        }
        let marker_x = depth.saturating_mul(2).saturating_add(2);
        if selected {
            selected_at = Some((area.x + marker_x.min(width.saturating_sub(1)), y));
        }
        let marker = match (row.entry.is_dir, row.expanded) {
            (true, true) => "▾",
            (true, false) => "▸",
            (false, _) => " ",
        };
        put(out, area, marker_x, y, marker, Style::new().fg(theme.muted));
        let name_x = marker_x.saturating_add(2);
        let name_style = if lit {
            Style::new().fg(theme.strong).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(theme.text)
        };
        let name = cut(&row.entry.name, usize::from(end.saturating_sub(name_x)));
        put(out, area, name_x, y, &name, name_style);
        if marks.dirty.contains(&path) {
            put(out, area, end, y, "•", Style::new().fg(theme.warn));
        }
    }
    selected_at
}

/// A vertical `│` down `area` in `guide`: the rule between two splits, the only
/// vertical one on screen (README §5.6).
pub fn render_divider(theme: &Theme, area: Rect, frame: &mut Frame) {
    let out = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        out.set_string(area.x, y, "│", Style::new().fg(theme.guide));
    }
}

/// Writes `text` at column `x` of `area` (relative to its left edge) on screen
/// row `y`, clipped to `area`. Styles without a background keep the cell's, so a
/// glow row stays lit under the text.
fn put(out: &mut Buffer, area: Rect, x: u16, y: u16, text: &str, style: Style) {
    if x >= area.width || y >= area.bottom() {
        return;
    }
    out.set_stringn(area.x + x, y, text, usize::from(area.width - x), style);
}

/// `text` in at most `room` columns, its end replaced by `…` when it doesn't fit.
pub(crate) fn cut(text: &str, room: usize) -> String {
    if text.width() <= room {
        return text.to_string();
    }
    let mut kept = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > room {
            break;
        }
        kept.push(c);
        used += w;
    }
    if room > 0 {
        kept.push('…');
    }
    kept
}

/// `text` in at most `room` columns, its start replaced by `…` when it doesn't
/// fit: the end of a path says most about it.
pub(crate) fn cut_front(text: &str, room: usize) -> String {
    if text.width() <= room {
        return text.to_string();
    }
    if room == 0 {
        return String::new();
    }
    let mut kept = Vec::new();
    let mut used = 0;
    for c in text.chars().rev() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > room {
            break;
        }
        kept.push(c);
        used += w;
    }
    std::iter::once('…').chain(kept.into_iter().rev()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::style::Color;
    use std::fs;

    fn draw(
        theme: &Theme,
        tree: &Tree,
        marks: &TreeMarks,
        focused: bool,
    ) -> anyhow::Result<(Buffer, Option<(u16, u16)>)> {
        let mut terminal = Terminal::new(TestBackend::new(30, 8))?;
        let mut at = None;
        terminal.draw(|frame| {
            at = render_tree(theme, tree, marks, focused, Rect::new(0, 0, 28, 8), frame)
        })?;
        Ok((terminal.backend().buffer().clone(), at))
    }

    fn row(buffer: &Buffer, y: u16) -> String {
        (0..28).map(|x| buffer[(x, y)].symbol()).collect()
    }

    #[test]
    fn rows_are_indented_with_guides_and_the_active_file_glows() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::create_dir(dir.path().join("src"))?;
        let long = dir
            .path()
            .join("src")
            .join("a_very_long_file_name_indeed.rs");
        fs::write(&long, "")?;
        fs::write(dir.path().join("notes.txt"), "")?;
        let mut tree = Tree::new(dir.path());
        tree.expand();

        let theme = Theme::named("aurora").expect("aurora exists");
        let long = std::path::absolute(&long)?;
        let marks = TreeMarks {
            project: "~/src",
            active: Some(&long),
            dirty: std::slice::from_ref(&long),
        };
        let (buffer, at) = draw(&theme, &tree, &marks, false)?;
        assert_eq!(row(&buffer, 1).trim_end(), "  ✦ tome    ~/src");
        assert_eq!(row(&buffer, 3).trim_end(), "  ▾ src");
        assert_eq!(row(&buffer, 4).trim_end(), "   │  a_very_long_file_n…•");
        assert_eq!(row(&buffer, 5).trim_end(), "    notes.txt");
        assert_eq!(at, Some((2, 3)));

        assert_eq!(buffer[(2, 1)].fg, theme.accent2);
        assert_eq!(buffer[(4, 1)].fg, theme.accent);
        assert_eq!(buffer[(7, 1)].fg, theme.accent2);
        assert!(buffer[(6, 1)].modifier.contains(Modifier::BOLD));
        assert_eq!(buffer[(12, 1)].fg, theme.muted);
        assert_eq!(buffer[(3, 4)].fg, theme.guide);
        assert_eq!(buffer[(25, 4)].fg, theme.warn);

        // The active row glows from `accent` into `surface`; unfocused, the
        // selection on row 3 isn't drawn at all.
        assert_eq!(buffer[(0, 4)].bg, mix(theme.surface, theme.accent, 0.26));
        assert_eq!(buffer[(0, 3)].bg, theme.surface);
        assert_eq!(buffer[(6, 4)].fg, theme.strong);
        assert!(buffer[(6, 4)].modifier.contains(Modifier::BOLD));
        assert_eq!(buffer[(4, 5)].fg, theme.text);
        assert_eq!(buffer[(4, 3)].fg, theme.text);
        Ok(())
    }

    #[test]
    fn the_focused_selection_glows_and_mono_reverses_it() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        fs::write(dir.path().join("a.txt"), "")?;
        fs::write(dir.path().join("b.txt"), "")?;
        let mut tree = Tree::new(dir.path());
        tree.move_by(1);
        let marks = TreeMarks {
            project: "",
            active: None,
            dirty: &[],
        };

        let aurora = Theme::named("aurora").expect("aurora exists");
        let (buffer, at) = draw(&aurora, &tree, &marks, true)?;
        assert_eq!(at, Some((2, 4)));
        assert_eq!(buffer[(0, 4)].bg, mix(aurora.surface, aurora.accent, 0.26));
        assert_eq!(buffer[(0, 3)].bg, aurora.surface);
        assert!(!buffer[(0, 4)].modifier.contains(Modifier::REVERSED));
        assert!(buffer[(4, 4)].modifier.contains(Modifier::BOLD));

        let mono = Theme::named("mono").expect("mono exists");
        let (buffer, _) = draw(&mono, &tree, &marks, true)?;
        for x in 0..28 {
            assert!(buffer[(x, 4)].modifier.contains(Modifier::REVERSED));
            assert!(!buffer[(x, 3)].modifier.contains(Modifier::REVERSED));
        }
        assert!(buffer[(4, 4)].modifier.contains(Modifier::BOLD));
        assert_eq!(buffer[(0, 4)].bg, Color::Reset);
        Ok(())
    }

    #[test]
    fn names_and_paths_are_cut_with_an_ellipsis() {
        assert_eq!(cut("main.rs", 7), "main.rs");
        assert_eq!(cut("main.rs", 5), "main…");
        assert_eq!(cut("main.rs", 0), "");
        assert_eq!(cut_front("~/a/project", 8), "…project");
    }

    #[test]
    fn the_project_reads_from_home() -> anyhow::Result<()> {
        let home = std::path::absolute("home")?;
        let root = home.join("src").join("tome");
        assert_eq!(project_label(&root, Some(&home)), "~/src/tome");
        let elsewhere = std::path::absolute("elsewhere")?;
        assert_eq!(
            project_label(&elsewhere, Some(&home)),
            elsewhere.display().to_string()
        );
        Ok(())
    }
}
