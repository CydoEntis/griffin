//! The file tree as a flat list of visible rows: expanding a folder reads it from
//! disk and inserts its entries below it, collapsing removes them again.

use std::path::Path;

use super::walk::{Entry, list_dir};

/// One visible line of the tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub entry: Entry,
    /// 0 for the root's own entries.
    pub depth: usize,
    /// Only ever true for folders.
    pub expanded: bool,
}

#[derive(Debug, Default, Clone)]
pub struct Tree {
    rows: Vec<Row>,
    selected: usize,
    /// First row shown.
    pub scroll: usize,
}

impl Tree {
    /// The root's entries, collapsed, with the first one selected.
    pub fn new(root: &Path) -> Self {
        let rows = list_dir(root)
            .into_iter()
            .map(|entry| Row {
                entry,
                depth: 0,
                expanded: false,
            })
            .collect();
        Tree {
            rows,
            selected: 0,
            scroll: 0,
        }
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn selected_row(&self) -> Option<&Row> {
        self.rows.get(self.selected)
    }

    /// Moves the selection by `delta` rows, stopping at either end.
    pub fn move_by(&mut self, delta: isize) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    pub fn select(&mut self, index: usize) {
        if index < self.rows.len() {
            self.selected = index;
        }
    }

    /// Expands the selected folder, reading it from disk. Does nothing on a file
    /// or an already expanded folder.
    pub fn expand(&mut self) {
        let index = self.selected;
        let Some(row) = self.rows.get_mut(index) else {
            return;
        };
        if !row.entry.is_dir || row.expanded {
            return;
        }
        row.expanded = true;
        let depth = row.depth + 1;
        let children = list_dir(&row.entry.path).into_iter().map(|entry| Row {
            entry,
            depth,
            expanded: false,
        });
        self.rows.splice(index + 1..index + 1, children);
    }

    /// Collapses the selected folder; on a file or a collapsed folder, selects its
    /// parent folder instead, so ← keeps walking up.
    pub fn collapse(&mut self) {
        let index = self.selected;
        let Some(row) = self.rows.get(index) else {
            return;
        };
        if row.expanded {
            let depth = row.depth;
            let end = self.rows[index + 1..]
                .iter()
                .position(|r| r.depth <= depth)
                .map_or(self.rows.len(), |offset| index + 1 + offset);
            self.rows.drain(index + 1..end);
            self.rows[index].expanded = false;
        } else if row.depth > 0
            && let Some(parent) = self.rows[..index].iter().rposition(|r| r.depth < row.depth)
        {
            self.selected = parent;
        }
    }

    /// Expands a collapsed folder or collapses an expanded one.
    pub fn toggle(&mut self) {
        match self.selected_row() {
            Some(row) if row.expanded => self.collapse(),
            Some(_) => self.expand(),
            None => {}
        }
    }

    /// The row drawn on line `line` of a pane, if any.
    pub fn row_at(&self, line: usize) -> Option<usize> {
        let index = self.scroll + line;
        (index < self.rows.len()).then_some(index)
    }

    /// Scrolls just enough that the selection is inside a pane `height` rows tall.
    pub fn follow(&mut self, height: usize) {
        let height = height.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + height {
            self.scroll = self.selected + 1 - height;
        }
    }

    /// Scrolls by `lines` (negative is up) without moving the selection, stopping
    /// when the last row reaches the bottom of a pane `height` rows tall.
    pub fn scroll_by(&mut self, lines: isize, height: usize) {
        let max = self.rows.len().saturating_sub(height.max(1));
        self.scroll = self.scroll.saturating_add_signed(lines).min(max);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// `a/` holding `a/inner/deep.txt` and `a/x.txt`, `b/`, and `z.txt`.
    fn project() -> anyhow::Result<tempfile::TempDir> {
        let dir = tempfile::tempdir()?;
        fs::create_dir_all(dir.path().join("a").join("inner"))?;
        fs::write(dir.path().join("a").join("inner").join("deep.txt"), "")?;
        fs::write(dir.path().join("a").join("x.txt"), "")?;
        fs::create_dir(dir.path().join("b"))?;
        fs::write(dir.path().join("z.txt"), "")?;
        Ok(dir)
    }

    fn lines(tree: &Tree) -> Vec<String> {
        tree.rows()
            .iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth), r.entry.name))
            .collect()
    }

    #[test]
    fn expand_inserts_children_and_collapse_removes_them_all() -> anyhow::Result<()> {
        let dir = project()?;
        let mut tree = Tree::new(dir.path());
        assert_eq!(lines(&tree), ["a", "b", "z.txt"]);

        tree.expand();
        assert_eq!(lines(&tree), ["a", "  inner", "  x.txt", "b", "z.txt"]);
        tree.move_by(1);
        tree.expand();
        assert_eq!(
            lines(&tree),
            ["a", "  inner", "    deep.txt", "  x.txt", "b", "z.txt"]
        );

        // Collapsing the top folder drops the grandchildren too.
        tree.select(0);
        tree.collapse();
        assert_eq!(lines(&tree), ["a", "b", "z.txt"]);
        // Expanding again reads the disk afresh: nothing stays expanded.
        tree.toggle();
        assert_eq!(lines(&tree), ["a", "  inner", "  x.txt", "b", "z.txt"]);
        Ok(())
    }

    #[test]
    fn collapse_on_a_child_selects_its_parent() -> anyhow::Result<()> {
        let dir = project()?;
        let mut tree = Tree::new(dir.path());
        tree.expand();
        tree.select(2);
        assert_eq!(
            tree.selected_row().map(|r| r.entry.name.as_str()),
            Some("x.txt")
        );
        tree.collapse();
        assert_eq!(tree.selected(), 0);
        // A top-level row has no parent to go to.
        tree.select(4);
        tree.collapse();
        assert_eq!(tree.selected(), 4);
        Ok(())
    }

    #[test]
    fn expanding_a_file_does_nothing_and_moves_stop_at_the_ends() -> anyhow::Result<()> {
        let dir = project()?;
        let mut tree = Tree::new(dir.path());
        tree.move_by(-1);
        assert_eq!(tree.selected(), 0);
        tree.move_by(10);
        assert_eq!(tree.selected(), 2);
        tree.expand();
        assert_eq!(tree.rows().len(), 3);
        Ok(())
    }

    #[test]
    fn follow_and_scroll_keep_rows_in_the_pane() -> anyhow::Result<()> {
        let dir = project()?;
        let mut tree = Tree::new(dir.path());
        tree.expand();
        tree.move_by(4);
        tree.follow(2);
        assert_eq!(tree.scroll, 3);
        assert_eq!(tree.row_at(0), Some(3));
        assert_eq!(tree.row_at(2), None);
        tree.scroll_by(-10, 2);
        assert_eq!(tree.scroll, 0);
        tree.scroll_by(10, 2);
        assert_eq!(tree.scroll, 3);
        Ok(())
    }
}
